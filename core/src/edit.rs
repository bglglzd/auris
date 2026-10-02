//! Правка записанного аудио: карта громкости для таймлайна и вырезание
//! фрагментов без перекодирования (PCM i16 → PCM i16).
//!
//! Формат дорожек в конвейере всегда один — WAV, моно, 16 кГц, 16 бит (его
//! пишет и рекордер, и декодер импорта), поэтому вырез — это просто пропуск
//! сэмплов: качество не теряется даже при многократной правке.
//!
//! Чистый Rust без cargo-фич → собирается и тестируется на любой ОС.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::transcript::Transcript;

/// Шкала громкости 0..1000 — та же, что у живых уровней записи
/// (`recorder::TrackLevels`), чтобы громкость во всём приложении измерялась
/// одинаково.
const LEVEL_SCALE: f64 = 1000.0;

/// Реплика короче этого остатка (сек) после выреза считается вырезанной
/// целиком и удаляется из расшифровки.
const MIN_KEPT_SEGMENT_SECS: f64 = 0.05;

/// Интервал времени дорожки в секундах, полуинтервал `[start, end)`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Range {
    pub start_secs: f64,
    pub end_secs: f64,
}

impl Range {
    pub fn new(start_secs: f64, end_secs: f64) -> Self {
        Self {
            start_secs,
            end_secs,
        }
    }

    pub fn len_secs(&self) -> f64 {
        (self.end_secs - self.start_secs).max(0.0)
    }
}

/// Карта громкости дорожки для таймлайна: по «корзине» на пиксель.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Waveform {
    /// Пик амплитуды в корзине, 0..1000 — ореол волны (видно щелчки и всплески).
    pub peaks: Vec<u16>,
    /// Средняя громкость (RMS) в корзине, 0..1000 — тело волны (видно речь).
    pub rms: Vec<u16>,
    pub duration_secs: f64,
    pub sample_rate: u32,
}

/// Строит карту громкости WAV-дорожки: `buckets` корзин от начала до конца.
///
/// Файл читается ПОТОКОВО (двухчасовая запись — это ~230 МБ, в память её
/// поднимать нельзя). Поддерживает моно и многоканальный WAV, целочисленный
/// (16/24/32 бита) и float — на всякий случай, хотя конвейер всегда даёт
/// 16 кГц моно i16.
pub fn waveform(path: &Path, buckets: usize) -> AppResult<Waveform> {
    let reader = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1);
    let frames = reader.len() as usize / channels;
    let duration_secs = frames as f64 / sample_rate as f64;

    if frames == 0 || buckets == 0 {
        return Ok(Waveform {
            peaks: Vec::new(),
            rms: Vec::new(),
            duration_secs,
            sample_rate,
        });
    }

    // Кадров на корзину: не меньше одного, поэтому корзин может выйти меньше
    // запрошенного (короткая дорожка) — фронтенд растягивает по длительности.
    let per_bucket = frames.div_ceil(buckets.min(frames));
    let count = frames.div_ceil(per_bucket);
    let mut peak_acc = vec![0.0f64; count];
    let mut sumsq_acc = vec![0.0f64; count];
    let mut n_acc = vec![0u32; count];

    let mut push = |index: usize, value: f64| {
        let frame = index / channels;
        let bucket = (frame / per_bucket).min(count - 1);
        let v = value.abs().min(1.0);
        if v > peak_acc[bucket] {
            peak_acc[bucket] = v;
        }
        sumsq_acc[bucket] += v * v;
        n_acc[bucket] += 1;
    };

    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => {
            for (i, s) in reader.into_samples::<f32>().enumerate() {
                match s {
                    Ok(v) => push(i, v as f64),
                    Err(_) => break, // обрыв файла — рисуем, что успели прочесть
                }
            }
        }
        (hound::SampleFormat::Int, bits) => {
            let full = (1i64 << (bits.max(1) as i64 - 1)) as f64;
            for (i, s) in reader.into_samples::<i32>().enumerate() {
                match s {
                    Ok(v) => push(i, v as f64 / full),
                    Err(_) => break,
                }
            }
        }
    }

    let peaks = peak_acc
        .iter()
        .map(|&v| (v * LEVEL_SCALE).round() as u16)
        .collect();
    let rms = sumsq_acc
        .iter()
        .zip(n_acc.iter())
        .map(|(&sum, &n)| {
            if n == 0 {
                0
            } else {
                ((sum / n as f64).sqrt() * LEVEL_SCALE).round() as u16
            }
        })
        .collect();

    Ok(Waveform {
        peaks,
        rms,
        duration_secs,
        sample_rate,
    })
}

/// Приводит набор интервалов к каноническому виду: отбрасывает пустые и
/// «перевёрнутые», сортирует по началу, склеивает пересекающиеся и смежные.
pub fn merge_ranges(ranges: &[Range]) -> Vec<Range> {
    let mut sorted: Vec<Range> = ranges
        .iter()
        .map(|r| Range::new(r.start_secs.max(0.0), r.end_secs.max(0.0)))
        .filter(|r| r.len_secs() > 0.0)
        .collect();
    sorted.sort_by(|a, b| {
        a.start_secs
            .partial_cmp(&b.start_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out: Vec<Range> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match out.last_mut() {
            Some(last) if r.start_secs <= last.end_secs => {
                if r.end_secs > last.end_secs {
                    last.end_secs = r.end_secs;
                }
            }
            _ => out.push(r),
        }
    }
    out
}

/// Сколько секунд из интервала `seg` попадает в вырезы `merged`
/// (`merged` — результат [`merge_ranges`]).
pub fn overlap(seg: &Range, merged: &[Range]) -> f64 {
    let mut total = 0.0;
    for cut in merged {
        let lo = seg.start_secs.max(cut.start_secs);
        let hi = seg.end_secs.min(cut.end_secs);
        if hi > lo {
            total += hi - lo;
        }
    }
    total
}

/// Время `t` исходной дорожки → время на дорожке после вырезов `merged`.
/// Функция монотонна; точка внутри выреза «садится» на его начало.
pub fn map_time(t: f64, merged: &[Range]) -> f64 {
    let mut removed = 0.0;
    for cut in merged {
        if t <= cut.start_secs {
            break;
        }
        removed += if t >= cut.end_secs {
            cut.len_secs()
        } else {
            t - cut.start_secs
        };
    }
    (t - removed).max(0.0)
}

/// Пишет в `dst` дорожку `src` без интервалов `cuts`. Сэмплы копируются
/// как есть (PCM i16 → PCM i16), поэтому правка не теряет качество.
/// Возвращает длительность результата в секундах.
///
/// `dst` должен отличаться от `src` — вызывающий пишет во временный файл и
/// затем подменяет им дорожку.
pub fn apply_cuts(src: &Path, dst: &Path, cuts: &[Range]) -> AppResult<f64> {
    let reader = hound::WavReader::open(src).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(AppError::Audio(format!(
            "правка поддерживает только 16-битный PCM WAV (у файла {} бит)",
            spec.bits_per_sample
        )));
    }
    let channels = spec.channels.max(1) as usize;
    let rate = spec.sample_rate.max(1) as f64;

    // Границы вырезов в кадрах: сравнение по индексу кадра дешевле, чем по
    // времени, а сами вырезы отсортированы — идём по ним курсором `ci`.
    let cut_frames: Vec<(usize, usize)> = merge_ranges(cuts)
        .iter()
        .map(|r| {
            (
                (r.start_secs * rate).round().max(0.0) as usize,
                (r.end_secs * rate).round().max(0.0) as usize,
            )
        })
        .filter(|(a, b)| b > a)
        .collect();

    let mut writer =
        hound::WavWriter::create(dst, spec).map_err(|e| AppError::Audio(e.to_string()))?;
    let mut ci = 0usize;
    let mut kept_frames = 0usize;
    for (i, sample) in reader.into_samples::<i16>().enumerate() {
        let s = match sample {
            Ok(s) => s,
            Err(_) => break, // обрыв внутри файла — сохраняем, что успели
        };
        let frame = i / channels;
        while ci < cut_frames.len() && frame >= cut_frames[ci].1 {
            ci += 1;
        }
        if ci < cut_frames.len() && frame >= cut_frames[ci].0 {
            continue;
        }
        writer
            .write_sample(s)
            .map_err(|e| AppError::Audio(e.to_string()))?;
        if i % channels == 0 {
            kept_frames += 1;
        }
    }
    writer
        .finalize()
        .map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(kept_frames as f64 / rate)
}

/// Пересчитывает времена расшифровки под вырезы: реплика целиком внутри выреза
/// удаляется, задетая частично — подрезается по времени (текст сохраняется),
/// последующие сдвигаются влево. Порядок реплик сохраняется.
pub fn remap_transcript(transcript: &Transcript, cuts: &[Range]) -> Transcript {
    let merged = merge_ranges(cuts);
    if merged.is_empty() {
        return transcript.clone();
    }
    let segments = transcript
        .segments
        .iter()
        .filter_map(|s| {
            let span = Range::new(s.start_secs, s.end_secs);
            let kept = span.len_secs() - overlap(&span, &merged);
            if kept < MIN_KEPT_SEGMENT_SECS {
                return None;
            }
            let mut out = s.clone();
            out.start_secs = map_time(s.start_secs, &merged);
            out.end_secs = map_time(s.end_secs, &merged);
            Some(out)
        })
        .collect();
    Transcript { segments }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TranscriptSegment;

    const RATE: u32 = 16_000;

    /// Пишет WAV (16 кГц моно i16), где каждая секунда заполнена своим
    /// значением — так по результату видно, какие секунды остались.
    fn write_marked_wav(path: &Path, per_second: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &v in per_second {
            for _ in 0..RATE {
                w.write_sample(v).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    fn read_samples(path: &Path) -> Vec<i16> {
        hound::WavReader::open(path)
            .unwrap()
            .into_samples::<i16>()
            .map(|s| s.unwrap())
            .collect()
    }

    fn seg(speaker: &str, start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment { origin: None,
            speaker: speaker.into(),
            start_secs: start,
            end_secs: end,
            text: text.into(),
        }
    }

    #[test]
    fn merge_sorts_joins_and_drops_empty() {
        let merged = merge_ranges(&[
            Range::new(5.0, 6.0),
            Range::new(1.0, 2.0),
            Range::new(1.5, 3.0),
            Range::new(3.0, 4.0), // смежный с предыдущим — склеивается
            Range::new(9.0, 9.0), // пустой — отбрасывается
            Range::new(8.0, 7.0), // перевёрнутый — отбрасывается
        ]);
        assert_eq!(
            merged,
            vec![Range::new(1.0, 4.0), Range::new(5.0, 6.0)]
        );
    }

    #[test]
    fn map_time_shifts_after_cuts_and_collapses_inside() {
        let merged = merge_ranges(&[Range::new(2.0, 4.0)]);
        assert_eq!(map_time(1.0, &merged), 1.0); // до выреза — без изменений
        assert_eq!(map_time(3.0, &merged), 2.0); // внутри — на начало выреза
        assert_eq!(map_time(6.0, &merged), 4.0); // после — сдвиг на длину
        assert_eq!(map_time(0.0, &[]), 0.0);
    }

    #[test]
    fn overlap_counts_only_intersections() {
        let merged = merge_ranges(&[Range::new(1.0, 2.0), Range::new(4.0, 6.0)]);
        assert_eq!(overlap(&Range::new(0.0, 10.0), &merged), 3.0);
        assert_eq!(overlap(&Range::new(2.0, 4.0), &merged), 0.0);
        assert_eq!(overlap(&Range::new(1.5, 5.0), &merged), 1.5);
    }

    #[test]
    fn waveform_shows_loud_and_quiet_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("marked.wav");
        // 3 секунды: тишина, громко, тихо.
        write_marked_wav(&p, &[0, 32_000, 3_200]);

        let wf = waveform(&p, 3).unwrap();

        assert_eq!(wf.sample_rate, RATE);
        assert!((wf.duration_secs - 3.0).abs() < 1e-9);
        assert_eq!(wf.peaks.len(), 3);
        assert_eq!(wf.rms.len(), 3);
        assert_eq!(wf.peaks[0], 0);
        assert!(wf.peaks[1] > 950, "громкая секунда: {}", wf.peaks[1]);
        assert!(
            wf.peaks[2] > 80 && wf.peaks[2] < 150,
            "тихая секунда: {}",
            wf.peaks[2]
        );
        // Постоянный сигнал ⇒ RMS == пик.
        assert_eq!(wf.rms[1], wf.peaks[1]);
    }

    #[test]
    fn waveform_of_short_track_returns_fewer_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("short.wav");
        crate::audio::write_silence_wav(&p, 1).unwrap();
        // Просим больше корзин, чем кадров — получаем по корзине на кадр.
        let wf = waveform(&p, 100_000).unwrap();
        assert_eq!(wf.peaks.len(), RATE as usize);
    }

    #[test]
    fn waveform_of_empty_track_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.wav");
        crate::audio::write_silence_wav(&p, 0).unwrap();
        let wf = waveform(&p, 50).unwrap();
        assert!(wf.peaks.is_empty());
        assert_eq!(wf.duration_secs, 0.0);
    }

    #[test]
    fn apply_cuts_removes_middle_second() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let dst = dir.path().join("dst.wav");
        write_marked_wav(&src, &[11, 22, 33]);

        let kept = apply_cuts(&src, &dst, &[Range::new(1.0, 2.0)]).unwrap();

        assert!((kept - 2.0).abs() < 1e-9);
        let samples = read_samples(&dst);
        assert_eq!(samples.len(), 2 * RATE as usize);
        assert_eq!(samples[0], 11);
        assert_eq!(samples[RATE as usize], 33); // вторая секунда результата = 33
        assert!(!samples.contains(&22));
    }

    #[test]
    fn apply_cuts_trims_head_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let dst = dir.path().join("dst.wav");
        write_marked_wav(&src, &[11, 22, 33, 44]);

        // «Оставить только это» = вырезать всё вне [1, 3).
        let kept = apply_cuts(&src, &dst, &[Range::new(0.0, 1.0), Range::new(3.0, 4.0)]).unwrap();

        assert!((kept - 2.0).abs() < 1e-9);
        let samples = read_samples(&dst);
        assert_eq!(samples[0], 22);
        assert_eq!(samples[samples.len() - 1], 33);
    }

    #[test]
    fn apply_cuts_without_cuts_copies_track() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let dst = dir.path().join("dst.wav");
        write_marked_wav(&src, &[7, 8]);
        let kept = apply_cuts(&src, &dst, &[]).unwrap();
        assert!((kept - 2.0).abs() < 1e-9);
        assert_eq!(read_samples(&src), read_samples(&dst));
    }

    #[test]
    fn apply_cuts_overlapping_ranges_removed_once() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let dst = dir.path().join("dst.wav");
        write_marked_wav(&src, &[1, 2, 3, 4]);
        // Пересекающиеся вырезы [1,3) и [2,3) — итог тот же, что от [1,3).
        let kept = apply_cuts(&src, &dst, &[Range::new(1.0, 3.0), Range::new(2.0, 3.0)]).unwrap();
        assert!((kept - 2.0).abs() < 1e-9);
        let samples = read_samples(&dst);
        assert_eq!(samples[0], 1);
        assert_eq!(samples[RATE as usize], 4);
    }

    #[test]
    fn apply_cuts_whole_track_leaves_valid_empty_wav() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.wav");
        let dst = dir.path().join("dst.wav");
        write_marked_wav(&src, &[5, 5]);
        let kept = apply_cuts(&src, &dst, &[Range::new(0.0, 99.0)]).unwrap();
        assert_eq!(kept, 0.0);
        assert!(read_samples(&dst).is_empty());
    }

    #[test]
    fn remap_transcript_drops_shifts_and_clips() {
        let transcript = Transcript {
            segments: vec![
                seg("me", 0.0, 1.0, "до выреза"),
                seg("them", 2.2, 3.5, "внутри выреза"),
                seg("me", 3.5, 5.0, "задета частично"),
                seg("them", 6.0, 7.0, "после выреза"),
            ],
        };
        // Вырезаем [2, 4).
        let out = remap_transcript(&transcript, &[Range::new(2.0, 4.0)]);

        let texts: Vec<&str> = out.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["до выреза", "задета частично", "после выреза"]);
        // Не задетая реплика — на месте.
        assert_eq!(out.segments[0].start_secs, 0.0);
        // Частично задетая: начало село на границу выреза, конец сдвинулся.
        assert_eq!(out.segments[1].start_secs, 2.0);
        assert_eq!(out.segments[1].end_secs, 3.0);
        // Последующая сдвинулась на 2 с.
        assert_eq!(out.segments[2].start_secs, 4.0);
        assert_eq!(out.segments[2].end_secs, 5.0);
    }

    #[test]
    fn remap_transcript_without_cuts_is_identity() {
        let transcript = Transcript {
            segments: vec![seg("me", 1.0, 2.0, "текст")],
        };
        assert_eq!(remap_transcript(&transcript, &[]), transcript);
    }
}
