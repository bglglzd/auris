//! Подготовка трудных участков к повторному распознаванию: оценка шума
//! (отношение сигнал/шум) и шумоподавление спектральным гейтом — чистый Rust,
//! без моделей и сети.
//!
//! Шумоподавление предназначено ТОЛЬКО для второго прохода распознавания по
//! шумным участкам: на чистой речи оно не нужно (и не применяется).

const SR: usize = 16_000;
const N_FFT: usize = 512;
const HOP: usize = 128;

/// Оценка отношения сигнал/шум (дБ) по кадрам 32 мс: уровень речи — 90-й
/// перцентиль RMS, шум — 15-й. `None` — нет речи (тишина/почти тишина).
pub fn snr_db(samples: &[f32]) -> Option<f32> {
    let frame = SR * 32 / 1000;
    let mut rms: Vec<f32> = samples
        .chunks(frame)
        .filter(|c| c.len() == frame)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    if rms.len() < 10 {
        return None;
    }
    rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f32| rms[((rms.len() - 1) as f32 * q) as usize];
    let speech = pick(0.90);
    if speech < 0.01 {
        return None;
    }
    let noise = pick(0.15).max(1e-5);
    Some(20.0 * (speech / noise).log10())
}

/// Читает WAV (16 кГц моно i16) в f32 [-1, 1].
pub fn read_wav_f32(path: &std::path::Path) -> crate::error::AppResult<Vec<f32>> {
    let r = hound::WavReader::open(path).map_err(|e| crate::error::AppError::Audio(e.to_string()))?;
    r.into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| crate::error::AppError::Audio(e.to_string()))
}

/// Окно Ханна длины `n`.
fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect()
}

/// Шумоподавление: профиль шума по самым тихим кадрам (нижние 20 % по
/// энергии), по каждой частоте — мягкий гейт `1 − α·шум/сигнал` с полом
/// 0.12 и сглаживанием во времени; срез ниже 70 Гц (гул, ветер, транспорт).
/// Громкость результата выравнивается (пик 0.9).
pub fn denoise(samples: &[f32]) -> Vec<f32> {
    if samples.len() < N_FFT * 4 {
        return samples.to_vec();
    }
    let win = hann(N_FFT);
    let bins = N_FFT / 2 + 1;
    let frames = (samples.len() - N_FFT) / HOP + 1;
    // STFT.
    let mut spec: Vec<(Vec<f64>, Vec<f64>)> = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut re: Vec<f64> = (0..N_FFT).map(|i| samples[f * HOP + i] as f64 * win[i]).collect();
        let mut im = vec![0.0; N_FFT];
        crate::nemo_mel::fft(&mut re, &mut im);
        spec.push((re, im));
    }
    let mag = |re: &[f64], im: &[f64], k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
    // Профиль шума: средняя амплитуда по тихим кадрам.
    let mut energy: Vec<(usize, f64)> = spec
        .iter()
        .enumerate()
        .map(|(i, (re, im))| (i, (0..bins).map(|k| mag(re, im, k)).sum()))
        .collect();
    energy.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let quiet = &energy[..(frames / 5).max(1)];
    let mut noise = vec![0.0f64; bins];
    for &(i, _) in quiet {
        for (k, n) in noise.iter_mut().enumerate() {
            *n += mag(&spec[i].0, &spec[i].1, k);
        }
    }
    for n in &mut noise {
        *n /= quiet.len() as f64;
    }
    let low_cut = (70.0 * N_FFT as f64 / SR as f64).ceil() as usize;
    const ALPHA: f64 = 1.6;
    const FLOOR: f64 = 0.12;
    let mut prev_gain = vec![1.0f64; bins];
    // Обратное STFT с перекрытием (окно Ханна на анализе и синтезе).
    let mut out = vec![0.0f64; samples.len()];
    let mut norm = vec![0.0f64; samples.len()];
    for (f, (re, im)) in spec.iter_mut().enumerate() {
        for k in 0..bins {
            let m = mag(re, im, k);
            let mut g = if k < low_cut { 0.0 } else { (1.0 - ALPHA * noise[k] / m.max(1e-12)).max(FLOOR) };
            // Сглаживание: быстрое открытие, плавное закрытие — без «бульканья».
            g = if g > prev_gain[k] { g } else { 0.6 * prev_gain[k] + 0.4 * g };
            prev_gain[k] = g;
            re[k] *= g;
            im[k] *= g;
            if k > 0 && k < N_FFT / 2 {
                re[N_FFT - k] = re[k];
                im[N_FFT - k] = -im[k];
            }
        }
        // Обратное БПФ через сопряжение.
        for v in im.iter_mut() {
            *v = -*v;
        }
        crate::nemo_mel::fft(re, im);
        for i in 0..N_FFT {
            let y = re[i] / N_FFT as f64;
            out[f * HOP + i] += y * win[i];
            norm[f * HOP + i] += win[i] * win[i];
        }
    }
    let mut y: Vec<f32> = out
        .iter()
        .zip(&norm)
        .map(|(o, n)| if *n > 1e-6 { (o / n) as f32 } else { 0.0 })
        .collect();
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 1e-4 {
        let k = 0.9 / peak;
        for v in &mut y {
            *v *= k;
        }
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Детерминированный «шум» (xorshift).
    fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    /// «Речь»: тон 300 Гц + 1200 Гц, включается и выключается слогами.
    fn speech(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let on = ((t * 3.0) as usize % 2 == 0) as u8 as f32;
                on * 0.3 * ((2.0 * std::f32::consts::PI * 300.0 * t).sin() + 0.5 * (2.0 * std::f32::consts::PI * 1200.0 * t).sin())
            })
            .collect()
    }

    fn snr_vs(clean: &[f32], test: &[f32]) -> f32 {
        // Масштаб подбираем МНК, чтобы нормализация громкости не мешала.
        let k = clean.iter().zip(test).map(|(c, t)| c * t).sum::<f32>() / test.iter().map(|t| t * t).sum::<f32>();
        let err: f32 = clean.iter().zip(test).map(|(c, t)| (c - k * t).powi(2)).sum();
        let sig: f32 = clean.iter().map(|c| c * c).sum();
        10.0 * (sig / err.max(1e-12)).log10()
    }

    #[test]
    fn snr_estimate_separates_clean_and_noisy() {
        let s = speech(SR * 6);
        let clean: Vec<f32> = s.iter().zip(noise(s.len(), 0.002, 7)).map(|(a, b)| a + b).collect();
        let noisy: Vec<f32> = s.iter().zip(noise(s.len(), 0.12, 7)).map(|(a, b)| a + b).collect();
        let (c, n) = (snr_db(&clean).unwrap(), snr_db(&noisy).unwrap());
        assert!(c > 30.0, "clean {c}");
        assert!(n < 15.0, "noisy {n}");
        assert!(snr_db(&vec![0.0; SR * 3]).is_none());
    }

    #[test]
    fn denoise_improves_snr_and_keeps_length() {
        let s = speech(SR * 6);
        let noisy: Vec<f32> = s.iter().zip(noise(s.len(), 0.1, 3)).map(|(a, b)| a + b).collect();
        let d = denoise(&noisy);
        assert_eq!(d.len(), noisy.len());
        let (before, after) = (snr_vs(&s, &noisy), snr_vs(&s, &d));
        assert!(after > before + 3.0, "before {before:.1} dB, after {after:.1} dB");
        assert!(d.iter().all(|v| v.is_finite() && v.abs() <= 0.91));
    }
}
