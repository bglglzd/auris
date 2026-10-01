//! Реальная транскрибация на Whisper (whisper.cpp через `whisper-rs`).
//!
//! Включается только с cargo-фичей `whisper` (тяжёлая C++-сборка + файл
//! модели). На Linux-сервере разработки без GTK это не собирается и не
//! проверяется — код предназначен для сборки на Windows:
//! `cargo build --features whisper` (нужен файл модели ggml, напр.
//! `ggml-medium.bin`, для русского лучше medium/large).
//!
//! Ожидаемый формат входа — то, что пишет рекордер: WAV PCM, моно, 16 кГц,
//! 16 бит (см. `crate::audio`). Whisper как раз хочет 16 кГц f32 моно.

use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::error::{AppError, AppResult};
use crate::transcriber::Transcriber;
use crate::transcript::Segment;

/// Транскрайбер на локальной модели Whisper (модель грузится в память один раз).
pub struct WhisperTranscriber {
    ctx: WhisperContext,
    /// Код языка (напр. "ru"); `None` — автоопределение.
    language: Option<String>,
    /// При автоопределении — приоритетный язык: берётся, если модель не
    /// уверена в другом (см. [`pick_language`]).
    prefer: Option<String>,
    /// Подсказка модели: термины из словаря (`initial_prompt`).
    prompt: Option<String>,
}

/// Выбор языка окна по вероятностям Whisper: приоритетный — если он не
/// исключён явно; другой — только когда модель в нём уверена (≥ 0.7), а у
/// приоритетного шансов почти нет (< 0.1).
pub fn pick_language(prefer: &str, detected: &str, p_detected: f32, p_prefer: f32) -> String {
    if detected != prefer && p_detected >= 0.7 && p_prefer < 0.1 {
        detected.to_string()
    } else {
        prefer.to_string()
    }
}

/// Модель по умолчанию — см. [`crate::models::DEFAULT_WHISPER`].
pub const DEFAULT_MODEL_SIZE: &str = crate::models::DEFAULT_WHISPER;

/// Длина окна (сек) при пооконной расшифровке — для прогресса и памяти.
pub const DEFAULT_WINDOW_SECS: usize = 60;
const SAMPLE_RATE: usize = 16_000;

/// Порог энергии (RMS) участка: ниже — считаем тишиной и отбрасываем реплику
/// как галлюцинацию whisper (типа «Спасибо за внимание» на молчании).
const SILENCE_RMS: f32 = 0.01;

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| s as f64 * s as f64).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Гарантирует наличие ggml-модели в `<data_dir>/models` (скачивает один
/// раз; прогресс зовётся только при реальной загрузке).
pub fn ensure_model(
    data_dir: &Path,
    size: &str,
    on_progress: &dyn Fn(f32),
) -> AppResult<PathBuf> {
    crate::models::ensure_whisper(data_dir, size, on_progress)
}

impl WhisperTranscriber {
    /// Встроенный движок: гарантирует модель (скачивает с прогрессом при
    /// необходимости), грузит её в память один раз и возвращает транскрайбер.
    pub fn managed(
        data_dir: &Path,
        size: Option<&str>,
        language: Option<String>,
        on_download: &dyn Fn(f32),
    ) -> AppResult<Self> {
        let size = crate::models::whisper_id(size);
        let model_path = ensure_model(data_dir, size, on_download)?;
        // По умолчанию — русский. Пусто/не задано → "ru"; "auto" → автоопределение.
        let language = match language {
            Some(l) if !l.is_empty() => Some(l),
            _ => Some("ru".to_string()),
        };
        let ctx = WhisperContext::new_with_params(
            &model_path,
            WhisperContextParameters::default(),
        )
        .map_err(|e| AppError::Audio(format!("whisper: cannot load model: {e}")))?;
        Ok(Self { ctx, language, prefer: None, prompt: None })
    }

    /// Автоопределение языка с приоритетным `prefer` (для участков, где
    /// основной движок мог ошибиться языком).
    pub fn with_preferred_language(mut self, prefer: Option<String>) -> Self {
        let prefer = prefer.map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty() && p != "auto");
        if prefer.is_some() {
            self.language = Some("auto".into());
        }
        self.prefer = prefer;
        self
    }

    /// Подсказка модели (термины словаря).
    pub fn with_prompt(mut self, prompt: Option<String>) -> Self {
        self.prompt = prompt.filter(|p| !p.trim().is_empty() && !p.contains('\0'));
        self
    }

    /// Язык окна: автоопределение Whisper с приоритетом `prefer`. Ошибка
    /// определения → приоритетный язык.
    fn detect_preferring(&self, state: &mut whisper_rs::WhisperState, chunk: &[f32], prefer: &str, threads: usize) -> String {
        let detected = state
            .pcm_to_mel(chunk, threads.max(1))
            .ok()
            .and_then(|_| state.lang_detect(0, threads.max(1)).ok());
        let Some((id, probs)) = detected else { return prefer.to_string() };
        let name = whisper_rs::get_lang_str(id).unwrap_or(prefer);
        let p_det = probs.get(id.max(0) as usize).copied().unwrap_or(0.0);
        let p_pref = whisper_rs::get_lang_id(prefer)
            .and_then(|i| probs.get(i.max(0) as usize).copied())
            .unwrap_or(0.0);
        pick_language(prefer, name, p_det, p_pref)
    }

    /// Читает WAV (i16 моно) и нормализует в f32 [-1.0, 1.0].
    fn read_wav_as_f32(path: &Path) -> AppResult<Vec<f32>> {
        let reader = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
        let samples: Vec<f32> = reader
            .into_samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Audio(e.to_string()))?;
        Ok(samples)
    }

    /// Расшифровка пооконно: модель грузится один раз, аудио идёт окнами по
    /// `window_secs` секунд, после каждого окна зовётся `on_progress(0.0..=1.0)`.
    /// Прогресс идёт из нашего цикла (без FFI-колбэка whisper, который ронял
    /// приложение). Подходит для очень длинных записей (память ограничена окном).
    pub fn transcribe_windowed(
        &self,
        wav_path: &Path,
        window_secs: usize,
        on_progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<Segment>> {
        let audio = Self::read_wav_as_f32(wav_path)?;
        if audio.is_empty() {
            return Ok(Vec::new());
        }

        let ctx = &self.ctx;
        let win = window_secs.max(1) * SAMPLE_RATE;
        // Окна режем в паузах (не посреди слова): ищем тишину в последних 10 с.
        let chunks = crate::audio::quiet_chunks(&audio, win, 10 * SAMPLE_RATE, SAMPLE_RATE / 5);
        let total = chunks.len().max(1);
        let mut segments = Vec::new();

        // Число потоков по ядрам CPU (whisper по умолчанию берёт мало → медленно).
        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 8) as std::os::raw::c_int;

        for (i, &(from, to)) in chunks.iter().enumerate() {
            let chunk = &audio[from..to];
            let mut state = ctx
                .create_state()
                .map_err(|e| AppError::Audio(format!("whisper: create_state: {e}")))?;

            // Beam search заметно точнее жадного декодирования (меньше пропусков
            // и искажённых слов); у turbo-моделей декодер лёгкий, цена небольшая.
            let mut params = FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            });
            params.set_n_threads(n_threads);
            let mut window_lang: Option<String> = None;
            match self.language.as_deref() {
                Some("auto") | None => {
                    if let Some(prefer) = self.prefer.as_deref() {
                        window_lang = Some(self.detect_preferring(&mut state, chunk, prefer, n_threads as usize));
                    }
                }
                Some(lang) => params.set_language(Some(lang)),
            }
            if let Some(l) = window_lang.as_deref() {
                params.set_language(Some(l));
            }
            if let Some(p) = self.prompt.as_deref() {
                params.set_initial_prompt(p);
            }
            params.set_translate(false);
            // Не опираться на предыдущий текст — меньше зацикленных галлюцинаций.
            params.set_no_context(true);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // Не выдумывать «[музыка]», «(смеётся)» и пустые токены.
            params.set_suppress_blank(true);
            params.set_suppress_nst(true);

            state
                .full(params, chunk)
                .map_err(|e| AppError::Audio(format!("whisper: full: {e}")))?;

            let offset = from as f64 / SAMPLE_RATE as f64;
            let n = state.full_n_segments();
            for s in 0..n {
                let Some(seg) = state.get_segment(s) else {
                    continue;
                };
                let text = seg
                    .to_str_lossy()
                    .map_err(|e| AppError::Audio(e.to_string()))?
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                // Таймкоды whisper — в сотых долях секунды (внутри окна).
                let t0c = seg.start_timestamp().max(0) as usize;
                let t1c = seg.end_timestamp().max(0) as usize;
                // Отбрасываем галлюцинации на тишине: если участок реплики
                // в исходном аудио почти беззвучный — это выдумка whisper.
                let a = (t0c * SAMPLE_RATE / 100).min(chunk.len());
                let b = (t1c * SAMPLE_RATE / 100).min(chunk.len());
                if b > a && rms(&chunk[a..b]) < SILENCE_RMS {
                    continue;
                }
                segments.push(Segment {
                    start_secs: offset + t0c as f64 / 100.0,
                    end_secs: offset + t1c as f64 / 100.0,
                    text,
                });
            }
            on_progress(i + 1, total);
        }
        Ok(segments)
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, wav_path: &Path) -> AppResult<Vec<Segment>> {
        self.transcribe_windowed(wav_path, DEFAULT_WINDOW_SECS, &|_, _| {})
    }
}

#[cfg(test)]
mod lang_tests {
    use super::pick_language;

    #[test]
    fn preferred_language_wins_unless_clearly_other() {
        assert_eq!(pick_language("ru", "en", 0.55, 0.3), "ru");
        assert_eq!(pick_language("ru", "en", 0.92, 0.03), "en");
        assert_eq!(pick_language("ru", "ru", 0.9, 0.9), "ru");
        assert_eq!(pick_language("ru", "uk", 0.75, 0.2), "ru");
    }
}
