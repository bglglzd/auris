//! Признаки для моделей NVIDIA NeMo (Parakeet): log-mel спектрограмма ровно
//! как `FilterbankFeatures` в NeMo — преэмфазис 0.97, STFT (n_fft 512, окно
//! Ханна 400 симметричное, шаг 160, center + нули), мощность, 128 мел-полос
//! (slaney), `ln(x + 2^-24)` и нормировка по каждой полосе (mean/std).
//! Чистый Rust без зависимостей — собирается и тестируется на любой ОС.

const N_FFT: usize = 512;
const WIN: usize = 400;
const HOP: usize = 160;
const SR: f64 = 16_000.0;
const PREEMPH: f32 = 0.97;
const LOG_GUARD: f32 = 5.960_464_5e-8; // 2^-24
const STD_EPS: f32 = 1e-5;

/// Число кадров признаков для `n` сэмплов (как `get_seq_len` в NeMo).
pub fn num_frames(n: usize) -> usize {
    n / HOP + 1
}

// ---------- FFT (радикс-2, на месте) ----------

pub(crate) fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nc = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nc;
            }
        }
        len <<= 1;
    }
}

// ---------- Мел-фильтры (librosa, slaney) ----------

fn hz_to_mel(f: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

/// Мел-фильтры `n_mels × (N_FFT/2+1)` как `librosa.filters.mel(norm="slaney")`.
pub fn mel_filters(n_mels: usize) -> Vec<Vec<f32>> {
    let bins = N_FFT / 2 + 1;
    let fft_freqs: Vec<f64> = (0..bins).map(|i| i as f64 * SR / N_FFT as f64).collect();
    let (mmin, mmax) = (hz_to_mel(0.0), hz_to_mel(SR / 2.0));
    let pts: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f64 / (n_mels + 1) as f64))
        .collect();
    (0..n_mels)
        .map(|m| {
            let (lo, c, hi) = (pts[m], pts[m + 1], pts[m + 2]);
            let enorm = 2.0 / (hi - lo);
            fft_freqs
                .iter()
                .map(|&f| {
                    let lower = (f - lo) / (c - lo);
                    let upper = (hi - f) / (hi - c);
                    (lower.min(upper).max(0.0) * enorm) as f32
                })
                .collect()
        })
        .collect()
}

/// Log-mel признаки NeMo: `n_mels × frames` (по строкам: полоса, затем кадры),
/// нормированные по каждой полосе. Вход — f32 [-1, 1], 16 кГц.
pub fn features(samples: &[f32], n_mels: usize) -> (Vec<f32>, usize) {
    let frames = num_frames(samples.len());
    if samples.is_empty() {
        return (vec![0.0; n_mels * frames], frames);
    }
    // Преэмфазис.
    let mut x = Vec::with_capacity(samples.len());
    x.push(samples[0]);
    for i in 1..samples.len() {
        x.push(samples[i] - PREEMPH * samples[i - 1]);
    }
    // Окно Ханна (симметричное) в центре 512-точечного кадра.
    let off = (N_FFT - WIN) / 2;
    let window: Vec<f64> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (WIN - 1) as f64).cos())
        .collect();
    let fb = mel_filters(n_mels);
    let bins = N_FFT / 2 + 1;
    let pad = N_FFT / 2;
    let n = x.len() as isize;

    let mut out = vec![0f32; n_mels * frames];
    let mut re = vec![0f64; N_FFT];
    let mut im = vec![0f64; N_FFT];
    let mut power = vec![0f32; bins];
    for t in 0..frames {
        let start = (t * HOP) as isize - pad as isize;
        for k in 0..N_FFT {
            let w = if k >= off && k < off + WIN { window[k - off] } else { 0.0 };
            let idx = start + k as isize;
            let v = if idx >= 0 && idx < n { x[idx as usize] as f64 } else { 0.0 };
            re[k] = v * w;
            im[k] = 0.0;
        }
        fft(&mut re, &mut im);
        for b in 0..bins {
            power[b] = (re[b] * re[b] + im[b] * im[b]) as f32;
        }
        for (m, f) in fb.iter().enumerate() {
            let e: f32 = f.iter().zip(&power).map(|(a, p)| a * p).sum();
            out[m * frames + t] = (e + LOG_GUARD).ln();
        }
    }
    // Нормировка по полосе: (x - mean) / (std + eps), std несмещённое.
    for m in 0..n_mels {
        let row = &mut out[m * frames..(m + 1) * frames];
        let mean = row.iter().map(|&v| v as f64).sum::<f64>() / frames as f64;
        let var = if frames > 1 {
            row.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / (frames - 1) as f64
        } else {
            0.0
        };
        let std = var.sqrt() as f32 + STD_EPS;
        for v in row.iter_mut() {
            *v = (*v - mean as f32) / std;
        }
    }
    (out, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_dft_on_small_signal() {
        let sig: Vec<f64> = (0..8).map(|i| (i as f64 * 0.7).sin() + 0.3).collect();
        let (mut re, mut im) = (sig.clone(), vec![0.0; 8]);
        fft(&mut re, &mut im);
        for k in 0..8 {
            let (mut r, mut i) = (0.0, 0.0);
            for (n, &x) in sig.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k * n) as f64 / 8.0;
                r += x * a.cos();
                i += x * a.sin();
            }
            assert!((re[k] - r).abs() < 1e-9 && (im[k] - i).abs() < 1e-9);
        }
    }

    #[test]
    fn mel_filters_match_librosa_reference() {
        // librosa.filters.mel(sr=16000, n_fft=512, n_mels=128, norm="slaney")
        let fb = mel_filters(128);
        assert_eq!(fb.len(), 128);
        assert_eq!(fb[0].len(), 257);
        assert!((fb[0][1] - 0.028_377_542).abs() < 1e-6, "{}", fb[0][1]);
        assert!((fb[127][255] - 0.000_870_531_4).abs() < 1e-7, "{}", fb[127][255]);
    }

    #[test]
    fn features_shape_and_normalization() {
        let sig: Vec<f32> = (0..16_000).map(|i| ((i as f32) * 0.05).sin() * 0.3).collect();
        let (f, frames) = features(&sig, 128);
        assert_eq!(frames, 101);
        assert_eq!(f.len(), 128 * 101);
        let row = &f[10 * frames..11 * frames];
        let mean: f32 = row.iter().sum::<f32>() / frames as f32;
        assert!(mean.abs() < 1e-3);
    }
}
