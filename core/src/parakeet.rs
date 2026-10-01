//! Распознавание NVIDIA Parakeet TDT 0.6B v3 (ONNX Runtime, int8).
//!
//! 25 европейских языков (русский в том числе) с пунктуацией и регистром;
//! на обычном CPU в разы быстрее Whisper и не выдумывает текст на тишине.
//! Модель — экспорт sherpa-onnx (encoder/decoder/joiner + tokens) с GitHub,
//! качается один раз архивом `.tar.bz2` (~490 МБ).
//!
//! Конвейер: log-mel признаки NeMo ([`crate::nemo_mel`]) → encoder (шаг 80 мс)
//! → жадное TDT-декодирование (joiner даёт токен и «сколько кадров
//! пропустить») → слова с таймкодами → реплики.

use std::io::Read;
use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;

use crate::error::{AppError, AppResult};
use crate::models::{parakeet_dir, parakeet_present, PARAKEET_FILES};
use crate::transcriber::Transcriber;
use crate::transcript::Segment;

const URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2";

const SR: usize = 16_000;
const N_MELS: usize = 128;
/// Длительность кадра энкодера: 10 мс × субдискретизация 8.
const FRAME_SECS: f64 = 0.08;
/// Окно распознавания: ~15 с, разрез в самой тихой точке последних 5 с.
/// На окнах 30 с жадное TDT-декодирование теряло хвосты фраз при наложении
/// голосов (проверено на pyannote sample); 15 с — близко к длине обучающих
/// фрагментов, текст полный.
const CHUNK_SECS: usize = 15;
const CHUNK_SEARCH_SECS: usize = 5;
const PRED_HIDDEN: usize = 640;
/// Длительности TDT: сколько кадров пропустить после токена.
const DURATIONS: [usize; 5] = [0, 1, 2, 3, 4];
/// Страховка от зацикливания: не больше символов на один кадр.
const MAX_SYMBOLS_PER_FRAME: usize = 10;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Audio(format!("parakeet: {e}"))
}

/// Счётчик прочитанных байт (для прогресса загрузки архива).
struct Counting<R> {
    inner: R,
    got: u64,
    total: u64,
    report: Box<dyn Fn(f32)>,
    last: f32,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.got += n as u64;
        if self.total > 0 {
            let f = self.got as f32 / self.total as f32;
            if f - self.last >= 0.002 {
                self.last = f;
                (self.report)(f.min(1.0));
            }
        }
        Ok(n)
    }
}

/// Гарантирует модель на диске: качает архив один раз и распаковывает только
/// нужные файлы (атомарно: во временный каталог, затем переименование).
pub fn ensure_model(data_dir: &Path, on_progress: &dyn Fn(f32)) -> AppResult<()> {
    if parakeet_present(data_dir) {
        return Ok(());
    }
    let dir = parakeet_dir(data_dir);
    let tmp = dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;

    let resp = ureq::get(URL)
        .call()
        .map_err(|e| AppError::Http(format!("download parakeet: {e}")))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // Прогресс шлём через канал-замыкание без заимствований (нужен 'static).
    let (tx, rx) = std::sync::mpsc::channel::<f32>();
    let reader = Counting {
        inner: resp.into_reader(),
        got: 0,
        total,
        report: Box::new(move |f| {
            let _ = tx.send(f);
        }),
        last: -1.0,
    };
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(reader));
    let mut found = 0usize;
    for entry in archive.entries().map_err(err)? {
        while let Ok(f) = rx.try_recv() {
            on_progress(f);
        }
        let mut entry = entry.map_err(err)?;
        let path = entry.path().map_err(err)?.into_owned();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        if PARAKEET_FILES.contains(&name.as_str()) {
            let mut out = std::fs::File::create(tmp.join(&name))?;
            std::io::copy(&mut entry, &mut out)?;
            found += 1;
        }
    }
    while let Ok(f) = rx.try_recv() {
        on_progress(f);
    }
    if found < PARAKEET_FILES.len() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(AppError::Http("download parakeet: архив неполный".into()));
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir)?;
    on_progress(1.0);
    Ok(())
}

fn load(path: &Path) -> AppResult<Session> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    Session::builder()
        .map_err(err)?
        .with_intra_threads(threads)
        .map_err(err)?
        .commit_from_file(path)
        .map_err(err)
}

/// Распознанный токен: номер кадра (от начала окна) и текст.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub secs: f64,
    pub piece: String,
    /// Уверенность модели в токене (вероятность после softmax), 0..1.
    pub prob: f32,
}

/// Окно расшифровки с оценкой: где модель «слышала» плохо.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowScore {
    pub start_secs: f64,
    pub end_secs: f64,
    /// Средняя уверенность по токенам окна; `None` — токенов нет.
    pub confidence: Option<f32>,
    pub tokens: usize,
}

/// Средняя уверенность по токенам.
pub fn mean_confidence(tokens: &[Token]) -> Option<f32> {
    (!tokens.is_empty()).then(|| tokens.iter().map(|t| t.prob).sum::<f32>() / tokens.len() as f32)
}

pub struct ParakeetTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    joiner: Mutex<Session>,
    tokens: Vec<String>,
    blank: usize,
}

type DecOut = (Vec<f32>, Vec<f32>, Vec<f32>);

impl ParakeetTranscriber {
    /// Гарантирует модель (скачивание — только в первый раз) и грузит её.
    pub fn managed(data_dir: &Path, on_download: &dyn Fn(f32)) -> AppResult<Self> {
        ensure_model(data_dir, on_download)?;
        let dir = parakeet_dir(data_dir);
        let tokens: Vec<String> = std::fs::read_to_string(dir.join("tokens.txt"))?
            .lines()
            .map(|l| l.rsplit_once(' ').map(|(t, _)| t).unwrap_or(l).to_string())
            .collect();
        let blank = tokens
            .iter()
            .position(|t| t == "<blk>")
            .unwrap_or(tokens.len().saturating_sub(1));
        Ok(Self {
            encoder: Mutex::new(load(&dir.join("encoder.int8.onnx"))?),
            decoder: Mutex::new(load(&dir.join("decoder.int8.onnx"))?),
            joiner: Mutex::new(load(&dir.join("joiner.int8.onnx"))?),
            tokens,
            blank,
        })
    }

    /// Прогон предсказателя: токен → (выход 640, состояния h, c).
    fn decode_step(&self, token: usize, h: &[f32], c: &[f32]) -> AppResult<DecOut> {
        let mut dec = self.decoder.lock().unwrap();
        let out = dec
            .run(ort::inputs![
                Tensor::from_array(([1usize, 1], vec![token as i32])).map_err(err)?,
                Tensor::from_array(([1usize], vec![1i32])).map_err(err)?,
                Tensor::from_array(([2usize, 1, PRED_HIDDEN], h.to_vec())).map_err(err)?,
                Tensor::from_array(([2usize, 1, PRED_HIDDEN], c.to_vec())).map_err(err)?
            ])
            .map_err(err)?;
        let g = out[0].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
        let nh = out[2].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
        let nc = out[3].try_extract_tensor::<f32>().map_err(err)?.1.to_vec();
        Ok((g, nh, nc))
    }

    /// Распознаёт один кусок звука (≤ ~30 с) → токены с временем от начала куска.
    pub fn transcribe_chunk(&self, samples: &[f32]) -> AppResult<Vec<Token>> {
        if samples.len() < SR / 10 {
            return Ok(Vec::new());
        }
        let (feats, frames) = crate::nemo_mel::features(samples, N_MELS);
        let (enc, t_len, dim) = {
            let mut e = self.encoder.lock().unwrap();
            let out = e
                .run(ort::inputs![
                    Tensor::from_array(([1usize, N_MELS, frames], feats)).map_err(err)?,
                    Tensor::from_array(([1usize], vec![frames as i64])).map_err(err)?
                ])
                .map_err(err)?;
            let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(err)?;
            let dim = shape[1] as usize;
            let t_all = shape[2] as usize;
            let lens = out[1].try_extract_tensor::<i64>().map_err(err)?.1.to_vec();
            let t_len = (lens.first().copied().unwrap_or(t_all as i64).max(0) as usize).min(t_all);
            // [1, dim, T] → по кадрам: кадр t = столбец.
            let mut by_frame = vec![0f32; t_all * dim];
            for d in 0..dim {
                for t in 0..t_all {
                    by_frame[t * dim + d] = data[d * t_all + t];
                }
            }
            (by_frame, t_len, dim)
        };

        let zeros = vec![0f32; 2 * PRED_HIDDEN];
        let (mut g, mut h, mut c) = self.decode_step(self.blank, &zeros, &zeros)?;
        let vocab = self.tokens.len();
        let mut out = Vec::new();
        let mut t = 0usize;
        let mut emitted_here = 0usize;
        let mut joiner = self.joiner.lock().unwrap();
        while t < t_len {
            let frame = enc[t * dim..(t + 1) * dim].to_vec();
            let logits = {
                let res = joiner
                    .run(ort::inputs![
                        Tensor::from_array(([1usize, dim, 1], frame)).map_err(err)?,
                        Tensor::from_array(([1usize, PRED_HIDDEN, 1], g.clone())).map_err(err)?
                    ])
                    .map_err(err)?;
                res[0].try_extract_tensor::<f32>().map_err(err)?.1.to_vec()
            };
            let argmax = |s: &[f32]| {
                s.iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            };
            let y = argmax(&logits[..vocab.min(logits.len())]);
            let mut skip = if logits.len() > vocab {
                DURATIONS[argmax(&logits[vocab..]).min(DURATIONS.len() - 1)]
            } else {
                1
            };
            if y != self.blank {
                let v = &logits[..vocab.min(logits.len())];
                let max = v[y];
                let denom: f32 = v.iter().map(|l| (l - max).exp()).sum();
                let prob = if denom > 0.0 { 1.0 / denom } else { 0.0 };
                out.push(Token { secs: t as f64 * FRAME_SECS, piece: self.tokens[y].clone(), prob });
                let next = self.decode_step(y, &h, &c)?;
                g = next.0;
                h = next.1;
                c = next.2;
                emitted_here += 1;
            }
            if skip == 0 && (y == self.blank || emitted_here >= MAX_SYMBOLS_PER_FRAME) {
                skip = 1;
            }
            if skip > 0 {
                emitted_here = 0;
            }
            t += skip;
        }
        Ok(out)
    }

    /// Расшифровка файла по окнам (разрез в паузах) с прогрессом (готово, всего).
    pub fn transcribe_windowed(
        &self,
        wav_path: &Path,
        _window_secs: usize,
        on_progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<Segment>> {
        Ok(self.transcribe_scored(wav_path, on_progress)?.0)
    }

    /// Расшифровка отрывка (16 кГц моно f32) в реплики со временем от его начала.
    pub fn transcribe_samples(&self, samples: &[f32]) -> AppResult<(Vec<Segment>, Option<f32>)> {
        let mut tokens = Vec::new();
        let chunks = crate::audio::quiet_chunks(samples, CHUNK_SECS * SR, CHUNK_SEARCH_SECS * SR, SR / 5);
        for &(a, b) in &chunks {
            let offset = a as f64 / SR as f64;
            for tk in self.transcribe_chunk(&samples[a..b])? {
                tokens.push(Token { secs: tk.secs + offset, ..tk });
            }
        }
        let conf = mean_confidence(&tokens);
        Ok((tokens_to_segments(&tokens, samples.len() as f64 / SR as f64), conf))
    }

    /// Как [`Self::transcribe_windowed`], плюс оценка каждого окна.
    pub fn transcribe_scored(
        &self,
        wav_path: &Path,
        on_progress: &dyn Fn(usize, usize),
    ) -> AppResult<(Vec<Segment>, Vec<WindowScore>)> {
        let reader = hound::WavReader::open(wav_path).map_err(|e| AppError::Audio(e.to_string()))?;
        let audio: Vec<f32> = reader
            .into_samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Audio(e.to_string()))?;
        if audio.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let chunks =
            crate::audio::quiet_chunks(&audio, CHUNK_SECS * SR, CHUNK_SEARCH_SECS * SR, SR / 5);
        let total = chunks.len();
        let mut tokens = Vec::new();
        let mut scores = Vec::with_capacity(total);
        for (i, &(a, b)) in chunks.iter().enumerate() {
            let offset = a as f64 / SR as f64;
            let window = self.transcribe_chunk(&audio[a..b])?;
            scores.push(WindowScore {
                start_secs: offset,
                end_secs: b as f64 / SR as f64,
                confidence: mean_confidence(&window),
                tokens: window.len(),
            });
            for tk in window {
                tokens.push(Token { secs: tk.secs + offset, ..tk });
            }
            on_progress(i + 1, total);
        }
        Ok((tokens_to_segments(&tokens, audio.len() as f64 / SR as f64), scores))
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(&self, wav_path: &Path) -> AppResult<Vec<Segment>> {
        self.transcribe_windowed(wav_path, CHUNK_SECS, &|_, _| {})
    }
}

/// Токены (SentencePiece, «▁» — начало слова) → реплики: разрыв на конце
/// предложения, на паузе > 1 с или если реплика длиннее 20 с.
pub fn tokens_to_segments(tokens: &[Token], audio_secs: f64) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut text = String::new();
    let mut start = 0.0f64;
    let mut last = 0.0f64;
    let flush = |segs: &mut Vec<Segment>, text: &mut String, start: f64, end: f64| {
        let t = text.trim().to_string();
        if !t.is_empty() {
            segs.push(Segment { start_secs: start, end_secs: end.max(start + 0.2), text: t });
        }
        text.clear();
    };
    for tk in tokens {
        let piece = tk.piece.as_str();
        if piece.starts_with("<|") || piece == "<unk>" || piece == "<pad>" {
            continue;
        }
        let new_word = piece.starts_with('▁');
        if !text.is_empty() && new_word {
            let gap = tk.secs - last;
            let sentence_end = text.trim_end().ends_with(['.', '?', '!', '…']);
            if gap > 1.0 || (sentence_end && tk.secs - start >= 1.5) || tk.secs - start > 20.0 {
                flush(&mut segs, &mut text, start, last + FRAME_SECS * 3.0);
            }
        }
        if text.is_empty() {
            start = tk.secs;
        }
        text.push_str(&piece.replace('▁', " "));
        last = tk.secs;
    }
    flush(&mut segs, &mut text, start, (last + FRAME_SECS * 3.0).min(audio_secs.max(last)));
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tk(secs: f64, p: &str) -> Token {
        Token { secs, piece: p.into(), prob: 1.0 }
    }

    #[test]
    fn tokens_join_into_sentences_and_split_on_pause() {
        let t = vec![
            tk(0.0, "▁При"),
            tk(0.2, "вет"),
            tk(0.4, "."),
            tk(1.9, "▁Как"),
            tk(2.1, "▁дела"),
            tk(2.3, "?"),
            tk(5.0, "▁Нор"),
            tk(5.2, "м"),
        ];
        let s = tokens_to_segments(&t, 6.0);
        let texts: Vec<&str> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, vec!["Привет.", "Как дела?", "Норм"]);
        assert_eq!(s[1].start_secs, 1.9);
        assert!(s[2].end_secs <= 6.0);
    }
}
