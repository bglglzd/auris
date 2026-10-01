use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 16_000;

/// Записывает `secs` секунд тишины в WAV-файл (моно, 16 кГц, 16 бит).
pub fn write_silence_wav(path: &Path, secs: u64) -> AppResult<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| AppError::Audio(e.to_string()))?;
    let total = SAMPLE_RATE as u64 * secs;
    for _ in 0..total {
        writer
            .write_sample(0i16)
            .map_err(|e| AppError::Audio(e.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}

/// Потоковый ресемплер в 16 кГц моно для захвата (микрофон и системный звук
/// на macOS приходят на частоте устройства, обычно 48 кГц).
///
/// Оконный sinc (окно Блэкмана) с полосой 0.92 от Найквиста выхода: частоты
/// выше 8 кГц не «заворачиваются» в речевую полосу (у простой линейной
/// интерполяции они превращались в шум и мешали распознаванию). ~100 умножений
/// на выходной сэмпл — для записи в реальном времени это ничто.
pub struct StreamResampler {
    /// Шаг по входу на один выходной сэмпл: in_rate / 16000.
    step: f64,
    /// Частота среза относительно частоты входа (≤ 0.5·0.92·out/in).
    fc: f64,
    /// Полуширина ядра во входных сэмплах.
    half: i64,
    /// История входа (моно); `hist[0]` — абсолютный индекс `base`.
    hist: Vec<f32>,
    base: i64,
    /// Абсолютная позиция (во входных сэмплах) следующего выходного сэмпла.
    next: f64,
    /// Сколько входных сэмплов принято всего.
    seen: i64,
    passthrough: bool,
}

/// Нулевых переходов sinc на каждой стороне ядра.
const SINC_ZC: f64 = 16.0;

impl StreamResampler {
    pub fn new(in_rate: u32) -> Self {
        let in_rate = in_rate.max(1) as f64;
        let ratio = SAMPLE_RATE as f64 / in_rate; // out/in
        let fc = 0.5 * ratio.min(1.0) * 0.92;
        let half = (SINC_ZC / (2.0 * fc)).ceil() as i64;
        Self {
            step: in_rate / SAMPLE_RATE as f64,
            fc,
            half,
            // До начала записи — тишина: первые выходы не ждут истории.
            hist: vec![0.0; half as usize],
            base: -half,
            next: 0.0,
            seen: 0,
            passthrough: (in_rate - SAMPLE_RATE as f64).abs() < 0.5,
        }
    }

    fn kernel(&self, d: f64) -> f64 {
        let span = self.half as f64;
        if d.abs() >= span {
            return 0.0;
        }
        let x = 2.0 * self.fc * d;
        let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
        // Окно Блэкмана на [-span, span].
        let w = 0.42 + 0.5 * (std::f64::consts::PI * d / span).cos() + 0.08 * (2.0 * std::f64::consts::PI * d / span).cos();
        2.0 * self.fc * sinc * w
    }

    /// Один выходной сэмпл в позиции `next` (вход вокруг неё уже в истории).
    fn emit_one(&mut self) -> f32 {
        let t = self.next;
        let c = t.floor() as i64;
        let mut acc = 0.0f64;
        for n in (c - self.half + 1)..=(c + self.half) {
            let i = n - self.base;
            if i >= 0 && (i as usize) < self.hist.len() {
                acc += self.hist[i as usize] as f64 * self.kernel(t - n as f64);
            }
        }
        self.next += self.step;
        acc as f32
    }

    /// Выдаёт выходные сэмплы, для которых уже есть вход до `limit` (абс. индекс).
    fn drain(&mut self, limit: i64, out: &mut Vec<f32>) {
        while (self.next.floor() as i64) + self.half <= limit {
            let y = self.emit_one();
            out.push(y);
        }
        // История старше ядра следующего выхода больше не нужна.
        let keep_from = self.next.floor() as i64 - self.half;
        let drop = (keep_from - self.base).clamp(0, self.hist.len() as i64) as usize;
        if drop > 4096 {
            self.hist.drain(..drop);
            self.base += drop as i64;
        }
    }

    /// Принимает кадры с `channels` каналами (interleaved), сводит в моно и
    /// дописывает выход (16 кГц) в `out`.
    pub fn push(&mut self, interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
        let ch = channels.max(1);
        if self.passthrough {
            out.extend(interleaved.chunks(ch).map(|f| f.iter().sum::<f32>() / f.len() as f32));
            return;
        }
        for frame in interleaved.chunks(ch) {
            self.hist.push(frame.iter().sum::<f32>() / frame.len() as f32);
            self.seen += 1;
        }
        self.drain(self.seen - 1, out);
    }

    /// Конец записи: досчитывает хвост (вход за концом — тишина), ровно до
    /// конца записанного звука.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            return;
        }
        let end = self.seen;
        self.hist.extend(std::iter::repeat(0.0).take(self.half as usize + 1));
        while self.next < end as f64 {
            let y = self.emit_one();
            out.push(y);
        }
    }
}

/// Приёмник одной дорожки живой записи: принимает куски звука любой частоты
/// и каналов, пишет WAV 16 кГц моно i16 и копит пиковый уровень для индикатора.
pub struct TrackSink {
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    resampler: StreamResampler,
    buf: Vec<f32>,
    written: u64,
    /// Пик |x| (0..32767) с прошлого опроса уровня.
    pub peak: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl TrackSink {
    pub fn create(path: &Path, in_rate: u32) -> AppResult<Self> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer =
            hound::WavWriter::create(path, spec).map_err(|e| AppError::Audio(e.to_string()))?;
        Ok(Self {
            writer: Some(writer),
            resampler: StreamResampler::new(in_rate),
            buf: Vec::new(),
            written: 0,
            peak: Default::default(),
        })
    }

    /// Частота входа изменилась (напр. устройство переподключили).
    pub fn set_input_rate(&mut self, in_rate: u32) {
        self.resampler = StreamResampler::new(in_rate);
    }

    /// Дописывает кусок звука (interleaved f32).
    pub fn push(&mut self, interleaved: &[f32], channels: usize) {
        if self.writer.is_none() {
            return;
        }
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        self.resampler.push(interleaved, channels, &mut buf);
        self.write_samples(&buf);
        self.buf = buf;
    }

    fn write_samples(&mut self, samples: &[f32]) {
        let Some(w) = self.writer.as_mut() else { return };
        let mut peak = 0u32;
        for &x in samples {
            let v = f32_to_i16(x);
            peak = peak.max(v.unsigned_abs() as u32);
            if w.write_sample(v).is_ok() {
                self.written += 1;
            }
        }
        self.peak.fetch_max(peak, std::sync::atomic::Ordering::Relaxed);
    }

    /// Сколько секунд записано.
    pub fn secs(&self) -> f64 {
        self.written as f64 / SAMPLE_RATE as f64
    }

    /// Закрывает WAV (заголовок с длиной). Повторный вызов — без эффекта.
    pub fn finalize(&mut self) -> AppResult<()> {
        if self.writer.is_some() {
            // Хвост ресемплера — последние ~мс звука.
            let mut tail = Vec::new();
            self.resampler.flush(&mut tail);
            self.write_samples(&tail);
        }
        if let Some(w) = self.writer.take() {
            w.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
        }
        Ok(())
    }
}

/// f32 [-1, 1] → i16 с насыщением.
pub fn f32_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Границы окон для пооконной расшифровки: окна около `target` сэмплов, но
/// разрез ставится в самое тихое место (по RMS кадров длиной `frame`) внутри
/// последних `search` сэмплов окна — чтобы не резать слово пополам.
pub fn quiet_chunks(samples: &[f32], target: usize, search: usize, frame: usize) -> Vec<(usize, usize)> {
    let len = samples.len();
    let frame = frame.max(1);
    let target = target.max(frame * 2);
    let search = search.min(target / 2);
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < len {
        let hard_end = start + target;
        // Хвост короче полуокна не отделяем — он уйдёт в текущее окно.
        if hard_end + target / 2 >= len {
            out.push((start, len));
            break;
        }
        let mut best = (hard_end, f32::INFINITY);
        let mut pos = hard_end - search;
        while pos + frame <= hard_end {
            let e: f32 = samples[pos..pos + frame].iter().map(|x| x * x).sum();
            if e < best.1 {
                best = (pos + frame / 2, e);
            }
            pos += frame;
        }
        out.push((start, best.0));
        start = best.0;
    }
    out
}

/// Возвращает длительность WAV-файла в секундах (округление вниз).
pub fn wav_duration_secs(path: &Path) -> AppResult<u64> {
    let reader = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = reader.spec();
    let frames = reader.len() as u64 / spec.channels as u64;
    Ok(frames / spec.sample_rate as u64)
}

/// Склеивает несколько WAV-частей (моно, 16 кГц, 16 бит i16) в один файл `dst`.
/// Используется для сборки сегментов записи (пауза/возобновление + восстановление
/// после сбоя) в единый `mic.wav`/`system.wav`. Части читаются по порядку
/// переданного списка. Битые/нечитаемые части ПРОПУСКАЮТСЯ (устойчивость
/// восстановления важнее строгости). Пустой список → валидный WAV нулевой длины.
pub fn concat_wavs(parts: &[PathBuf], dst: &Path) -> AppResult<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(dst, spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for part in parts {
        // Не валим всю склейку из-за одной битой части — пропускаем её.
        let reader = match hound::WavReader::open(part) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for sample in reader.into_samples::<i16>() {
            match sample {
                Ok(s) => writer
                    .write_sample(s)
                    .map_err(|e| AppError::Audio(e.to_string()))?,
                Err(_) => break, // обрыв внутри части — берём, что успели
            }
        }
    }
    writer
        .finalize()
        .map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_resampler_rates_and_mono_mix() {
        // 48 кГц стерео, 1 секунда кусками → 16000 моно-сэмплов.
        let mut r = StreamResampler::new(48_000);
        let mut out = Vec::new();
        let chunk: Vec<f32> = (0..960).flat_map(|_| [0.5f32, -0.5]).collect();
        for _ in 0..50 {
            r.push(&chunk, 2, &mut out);
        }
        r.flush(&mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 2, "len {}", out.len());
        assert!(out.iter().all(|x| x.abs() < 1e-6), "стерео в противофазе → 0");

        // 16 кГц моно — без изменений.
        let mut r = StreamResampler::new(16_000);
        let mut out = Vec::new();
        let sig: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.01).sin()).collect();
        r.push(&sig, 1, &mut out);
        r.flush(&mut out);
        assert_eq!(out.len(), 1600);
        assert!((out[800] - sig[800]).abs() < 1e-6);

        // 44.1 кГц моно, куски разной длины: длина и уровень сохраняются.
        let mut r = StreamResampler::new(44_100);
        let mut out = Vec::new();
        let sig = vec![0.25f32; 44_100];
        for part in sig.chunks(333) {
            r.push(part, 1, &mut out);
        }
        r.flush(&mut out);
        assert!((out.len() as i64 - 16_000).abs() <= 2, "len {}", out.len());
        // Края — переход из тишины; в середине уровень точный.
        assert!(out[200..15_800].iter().all(|x| (x - 0.25).abs() < 2e-3));
        assert_eq!(f32_to_i16(2.0), 32767);
        assert_eq!(f32_to_i16(-1.0), -32767);
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn tone(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn resampler_keeps_speech_band_and_blocks_aliasing() {
        // Речь (1 кГц и 3,4 кГц) проходит без потерь уровня.
        for hz in [1_000.0, 3_400.0] {
            let mut r = StreamResampler::new(48_000);
            let mut out = Vec::new();
            for part in tone(48_000, hz, 1.0).chunks(480) {
                r.push(part, 1, &mut out);
            }
            r.flush(&mut out);
            let level = rms(&out[1_000..15_000]);
            assert!((level - 0.3536).abs() < 0.01, "{hz} Гц: rms {level}");
        }
        // 12 кГц (выше 8 кГц Найквиста выхода) не превращается в шум 4 кГц.
        let mut r = StreamResampler::new(48_000);
        let mut out = Vec::new();
        for part in tone(48_000, 12_000.0, 1.0).chunks(480) {
            r.push(part, 1, &mut out);
        }
        r.flush(&mut out);
        let leak = rms(&out[1_000..15_000]);
        assert!(leak < 0.002, "алиасинг: rms {leak}");
    }

    #[test]
    fn track_sink_writes_16k_mono_and_tracks_peak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.wav");
        let mut sink = TrackSink::create(&path, 48_000).unwrap();
        // 1 кГц, 0.5 — речевая полоса проходит, пик ≈ 16383.
        let sig = tone(48_000, 1_000.0, 1.0);
        for chunk in sig.chunks(4800) {
            sink.push(chunk, 1); // 10 × 0.1 с
        }
        assert!((sink.secs() - 1.0).abs() < 0.01);
        assert!(sink.peak.load(std::sync::atomic::Ordering::Relaxed) > 10_000);
        sink.finalize().unwrap();
        sink.finalize().unwrap();
        let r = hound::WavReader::open(&path).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().channels, 1);
        assert!((r.len() as i64 - 16_000).abs() <= 2);
    }

    #[test]
    fn quiet_chunks_cut_in_silence_and_cover_everything() {
        // 25 «секунд» по 100 сэмплов: громко, но тишина на 8.5 и 17.2 с.
        let sr = 100;
        let mut v = vec![0.5f32; 25 * sr];
        for x in &mut v[850..860] {
            *x = 0.0;
        }
        for x in &mut v[1720..1730] {
            *x = 0.0;
        }
        let c = quiet_chunks(&v, 10 * sr, 3 * sr, 10);
        assert_eq!(c.first().unwrap().0, 0);
        assert_eq!(c.last().unwrap().1, v.len());
        for w in c.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        assert_eq!(c[0].1, 855);
        assert!((1715..=1730).contains(&c[1].1), "cut {}", c[1].1);
        assert_eq!(c.len(), 3);
        // Короткий файл — одно окно.
        assert_eq!(quiet_chunks(&v[..300], 10 * sr, 3 * sr, 10), vec![(0, 300)]);
        assert!(quiet_chunks(&[], 1000, 300, 10).is_empty());
    }

    #[test]
    fn writes_and_measures_three_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.wav");
        write_silence_wav(&path, 3).unwrap();
        assert!(path.exists());
        assert_eq!(wav_duration_secs(&path).unwrap(), 3);
    }

    #[test]
    fn concat_sums_durations() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        let b = dir.path().join("b.wav");
        write_silence_wav(&a, 1).unwrap();
        write_silence_wav(&b, 2).unwrap();
        let dst = dir.path().join("all.wav");
        concat_wavs(&[a, b], &dst).unwrap();
        assert_eq!(wav_duration_secs(&dst).unwrap(), 3);
    }

    #[test]
    fn concat_empty_writes_zero_length() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("empty.wav");
        concat_wavs(&[], &dst).unwrap();
        assert!(dst.exists());
        assert_eq!(wav_duration_secs(&dst).unwrap(), 0);
    }

    #[test]
    fn concat_skips_missing_part() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        write_silence_wav(&a, 2).unwrap();
        let missing = dir.path().join("nope.wav");
        let dst = dir.path().join("all.wav");
        concat_wavs(&[a, missing], &dst).unwrap();
        assert_eq!(wav_duration_secs(&dst).unwrap(), 2);
    }
}
