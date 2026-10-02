//! Эхо в записи звонка: задержанная копия речи, которую распознавание
//! принимает за ещё одного собеседника (или пишет второй раз).
//!
//! Откуда берётся:
//! - звонок на динамиках компьютера — голос собеседника из колонок попадает в
//!   микрофон («Я» говорит его словами);
//! - у собеседника не работает эхоподавление — ваш голос возвращается в
//!   системный звук (и делится как «Собеседник 2»);
//! - разговор по телефону на громкой связи, записанный микрофоном, — ваш
//!   голос возвращается из динамика телефона искажённым и с задержкой.
//!
//! Здесь — звуковая часть, без моделей: (1) по огибающим громкости находим,
//! есть ли в дорожке задержанная копия другой дорожки и с какой задержкой; (2) спектрально подавляем копию: по каждой частоте
//! оцениваем, какая доля опорного сигнала просочилась, и гасим её, а кадры,
//! где копия — почти весь звук, глушим целиком (как эхоподавление в
//! мессенджерах, только после записи).
//!
//! Подавление применяется к микрофону (собеседник из колонок): замер
//! `--example echo_eval` — слова собеседника из «Я» уходят, свои остаются.
//! Ваш голос, вернувшийся в звук звонка, так не убирается — подавление
//! портило речь собеседника; его убирает сверка реплик
//! (`transcript::drop_cross_echo` / `drop_self_echo`).

const SR: f32 = 16_000.0;
/// Кадр огибающей — 20 мс.
const ENV: usize = 320;
const N_FFT: usize = 512;
const HOP: usize = 128;

/// Найденный путь эха: опора → цель с задержкой `lag_secs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoPath {
    pub lag_secs: f32,
    /// Корреляция огибающих на этой задержке (0..1).
    pub corr: f32,
    /// Насколько пик выделяется над соседними задержками.
    pub prominence: f32,
}

/// Огибающая: лог-энергия кадров 20 мс.
fn envelope(x: &[f32]) -> Vec<f32> {
    x.chunks(ENV)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32 + 1e-9).log10())
        .collect()
}

/// Быстрая часть огибающей: минус скользящее среднее за ~200 мс. Медленная
/// часть — очерёдность реплик (говорит один — молчит другой) — маскирует
/// эхо; копия же повторяет именно слоги.
fn detail(env: &[f32]) -> Vec<f32> {
    const W: usize = 5;
    let n = env.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + env[i] as f64;
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(W), (i + W + 1).min(n));
            env[i] - ((prefix[b] - prefix[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Корреляция Пирсона `a[t]` и `b[t − lag]` по кадрам, где опора `b` звучит.
fn corr_at(a: &[f32], b: &[f32], lag: usize, active: &[bool]) -> f32 {
    let n = a.len().min(b.len() + lag);
    let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut k) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for t in lag..n {
        if !active[t - lag] {
            continue;
        }
        let (x, y) = (a[t] as f64, b[t - lag] as f64);
        sa += x;
        sb += y;
        saa += x * x;
        sbb += y * y;
        sab += x * y;
        k += 1.0;
    }
    if k < 50.0 {
        return 0.0;
    }
    let cov = sab / k - (sa / k) * (sb / k);
    let va = saa / k - (sa / k).powi(2);
    let vb = sbb / k - (sb / k).powi(2);
    if va <= 1e-12 || vb <= 1e-12 {
        return 0.0;
    }
    (cov / (va * vb).sqrt()) as f32
}

/// Кадры, где сигнал заметно громче своего фона (речь опоры).
fn active_frames(env: &[f32]) -> Vec<bool> {
    let mut s = env.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = s.get(s.len() / 5).copied().unwrap_or(-9.0);
    let peak = s.get(s.len() * 95 / 100).copied().unwrap_or(-9.0);
    // Порог: на 40 % пути от фона к пиковой громкости (в лог-шкале).
    let gate = floor + 0.4 * (peak - floor);
    env.iter().map(|&e| e > gate && peak - floor > 1.0).collect()
}

/// Пороги обнаружения: корреляция огибающих и выделенность пика над
/// соседними задержками (речь сама по себе коррелирована на десятки мс —
/// эхо даёт отдельный пик).
const MIN_CORR: f32 = 0.3;
const MIN_PROMINENCE: f32 = 0.12;

fn best_lag(target: &[f32], reference: &[f32], min_lag: f32, max_lag: f32) -> Option<EchoPath> {
    let (ea, eb) = (envelope(target), envelope(reference));
    let active = active_frames(&eb);
    let (a, b) = (detail(&ea), detail(&eb));
    let fr = |s: f32| (s * SR / ENV as f32).round() as usize;
    let (lo, hi) = (fr(min_lag), fr(max_lag));
    if a.len() < hi + 100 {
        return None;
    }
    let curve: Vec<f32> = (0..=hi + 8).map(|l| corr_at(&a, &b, l, &active)).collect();
    let mut best: Option<EchoPath> = None;
    for l in lo..=hi {
        let c = curve[l];
        // Выделенность: пик против соседей на ±80…160 мс.
        let side = |d: usize| {
            let left = l.checked_sub(d).map(|i| curve[i]);
            let right = curve.get(l + d).copied();
            match (left, right) {
                (Some(x), Some(y)) => x.max(y),
                (Some(x), None) | (None, Some(x)) => x,
                _ => c,
            }
        };
        let around = (4..=8).map(side).fold(f32::MIN, f32::max);
        let p = EchoPath { lag_secs: l as f32 * ENV as f32 / SR, corr: c, prominence: c - around };
        // Эхо — отдельный пик: выбираем самый выделенный среди достаточно
        // коррелированных (у малых задержек корреляция высокая сама по себе).
        if c >= MIN_CORR && best.map(|b| p.prominence > b.prominence).unwrap_or(true) {
            best = Some(p);
        }
    }
    best.filter(|p| p.prominence >= MIN_PROMINENCE)
}

/// Есть ли в `target` копия `reference` с задержкой `min_lag..max_lag` с.
pub fn detect(target: &[f32], reference: &[f32], min_lag: f32, max_lag: f32) -> Option<EchoPath> {
    best_lag(target, reference, min_lag, max_lag)
}

fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect()
}

/// Комплексные кадры STFT.
fn stft(x: &[f32], win: &[f64]) -> Vec<(Vec<f64>, Vec<f64>)> {
    if x.len() < N_FFT {
        return Vec::new();
    }
    let frames = (x.len() - N_FFT) / HOP + 1;
    (0..frames)
        .map(|f| {
            let mut re: Vec<f64> = (0..N_FFT).map(|i| x[f * HOP + i] as f64 * win[i]).collect();
            let mut im = vec![0.0; N_FFT];
            crate::nemo_mel::fft(&mut re, &mut im);
            (re, im)
        })
        .collect()
}

fn mags(spec: &[(Vec<f64>, Vec<f64>)]) -> Vec<Vec<f32>> {
    let bins = N_FFT / 2 + 1;
    spec.iter().map(|(re, im)| (0..bins).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() as f32).collect()).collect()
}

/// Сдвиг сигнала на `lag` отсчётов вправо (копия, пришедшая позже).
fn shifted(x: &[f32], start: usize, len: usize, lag: usize) -> Vec<f32> {
    (start..start + len).map(|i| if i >= lag { x.get(i - lag).copied().unwrap_or(0.0) } else { 0.0 }).collect()
}

/// Размытие опоры по времени (±3 кадра ≈ ±24 мс): задержка в звонке «гуляет».
fn smear(m: &[Vec<f32>]) -> Vec<Vec<f32>> {
    (0..m.len())
        .map(|t| {
            let (a, b) = (t.saturating_sub(3), (t + 3).min(m.len() - 1));
            (0..m[t].len()).map(|k| (a..=b).map(|i| m[i][k]).fold(0.0, f32::max)).collect()
        })
        .collect()
}

/// Доля опоры в цели по частотам: медиана |T|/|R| по кадрам, где опора
/// звучит (в «чистом эхе» отношение ≈ доле утечки, при двойной речи —
/// больше, в паузах цели — около нуля). Подобрано на `--example echo_eval`.
fn leak_share(t: &[Vec<f32>], r: &[Vec<f32>]) -> Vec<f32> {
    let bins = N_FFT / 2 + 1;
    (0..bins)
        .map(|k| {
            let mut rk: Vec<f32> = r.iter().map(|f| f[k]).collect();
            rk.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // Опора звучит: верхние 30 % по энергии в этой полосе.
            let gate = rk.get(rk.len() * 7 / 10).copied().unwrap_or(0.0).max(1e-6);
            let mut ratios: Vec<f32> =
                t.iter().zip(r).filter(|(_, rf)| rf[k] >= gate).map(|(tf, rf)| tf[k] / rf[k]).collect();
            if ratios.len() < 20 {
                return 0.0;
            }
            ratios.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            ratios[ratios.len() / 2]
        })
        .collect()
}

/// Подавление в одном блоке: `r` — опора, уже совмещённая по задержке,
/// `r0` — опора на «чужой» задержке (оценка естественной корреляции).
fn suppress_block(target: &[f32], r: &[f32], r0: &[f32]) -> Vec<f32> {
    let win = hann(N_FFT);
    let bins = N_FFT / 2 + 1;
    let mut spec = stft(target, &win);
    if spec.is_empty() {
        return target.to_vec();
    }
    let tmag = mags(&spec);
    let rmag = smear(&mags(&stft(r, &win)));
    let r0mag = smear(&mags(&stft(r0, &win)));
    let (h1, h0) = (leak_share(&tmag, &rmag), leak_share(&tmag, &r0mag));
    let raw: Vec<f32> = h1.iter().zip(&h0).map(|(a, b)| (a - b).clamp(0.0, 2.0)).collect();
    // Сглаживание по частоте (3 полосы).
    let h: Vec<f32> = (0..bins)
        .map(|k| {
            let (a, b) = (k.saturating_sub(1), (k + 1).min(bins - 1));
            raw[a..=b].iter().sum::<f32>() / (b - a + 1) as f32
        })
        .collect();
    if h.iter().all(|&v| v < 1e-3) {
        return target.to_vec();
    }
    const ALPHA: f32 = 2.0;
    const FLOOR: f64 = 0.08;
    let mut prev = vec![1.0f64; bins];
    let mut out = vec![0.0f64; target.len()];
    let mut norm = vec![0.0f64; target.len()];
    const GATE: f32 = 0.35;
    for (f, (re, im)) in spec.iter_mut().enumerate() {
        // Кадр, где эхо — бо́льшая часть звука (собеседник молчит), гасим
        // целиком: остаток эха разделение голосов приняло бы за человека.
        let (mut pe, mut pt) = (0f32, 0f32);
        for k in 0..bins {
            let e = h[k] * rmag.get(f).map(|x| x[k]).unwrap_or(0.0);
            pe += e * e;
            pt += tmag[f][k] * tmag[f][k];
        }
        let echo_frame = pt > 0.0 && pe >= GATE * pt;
        for k in 0..bins {
            let echo = h[k] * rmag.get(f).map(|x| x[k]).unwrap_or(0.0);
            let m = tmag[f][k].max(1e-9);
            let mut g = if echo_frame { 0.02 } else { (1.0 - (ALPHA * echo / m) as f64).max(FLOOR) };
            // Быстро закрываем, плавно открываем — без «бульканья».
            g = if g < prev[k] { g } else { 0.5 * prev[k] + 0.5 * g };
            prev[k] = g;
            re[k] *= g;
            im[k] *= g;
            if k > 0 && k < N_FFT / 2 {
                re[N_FFT - k] = re[k];
                im[N_FFT - k] = -im[k];
            }
        }
        for v in im.iter_mut() {
            *v = -*v;
        }
        crate::nemo_mel::fft(re, im);
        for i in 0..N_FFT {
            out[f * HOP + i] += re[i] / N_FFT as f64 * win[i];
            norm[f * HOP + i] += win[i] * win[i];
        }
    }
    out.iter()
        .zip(&norm)
        .zip(target)
        .map(|((o, n), t)| if *n > 1e-6 { (o / n) as f32 } else { *t })
        .collect()
}

/// Подавляет в `target` копию `reference`, пришедшую с задержкой `path`.
/// Блоками по 30 с
/// (память не растёт с длиной записи, доля утечки уточняется по ходу
/// разговора); длина и уровень сигнала сохраняются.
pub fn suppress(target: &[f32], reference: &[f32], path: &EchoPath) -> Vec<f32> {
    let n = target.len();
    let lag = (path.lag_secs * SR).round() as usize;
    let lag0 = lag + (0.4 * SR) as usize;
    let block = (30.0 * SR) as usize;
    let fade = (0.5 * SR) as usize;
    let mut out = vec![0.0f32; n];
    let mut wsum = vec![0.0f32; n];
    let mut start = 0;
    while start < n {
        let end = (start + block).min(n);
        let y = suppress_block(&target[start..end], &shifted(reference, start, end - start, lag), &shifted(reference, start, end - start, lag0));
        for (i, v) in y.iter().enumerate() {
            let g = start + i;
            // Плавный стык блоков.
            let w_in = if start > 0 { ((i as f32 + 1.0) / fade as f32).min(1.0) } else { 1.0 };
            let w_out = if end < n { (((end - g) as f32) / fade as f32).min(1.0) } else { 1.0 };
            let w = w_in.min(w_out);
            out[g] += w * v;
            wsum[g] += w;
        }
        if end == n {
            break;
        }
        start = end - fade;
    }
    out.iter().zip(&wsum).zip(target).map(|((o, w), t)| if *w > 1e-6 { o / w } else { *t }).collect()
}

/// Микрофон без собеседника из колонок: если в `mic` есть копия `system`
/// (задержка до 0.3 с — звук идёт по комнате), пишет подавленный вариант в
/// `dst` и возвращает найденный путь; иначе `None` и файл не создаётся.
pub fn clean_mic_file(mic: &std::path::Path, system: &std::path::Path, dst: &std::path::Path) -> crate::error::AppResult<Option<EchoPath>> {
    let m = crate::enhance::read_wav_f32(mic)?;
    let s = crate::enhance::read_wav_f32(system)?;
    let Some(path) = detect(&m, &s, 0.0, 0.3) else {
        return Ok(None);
    };
    let clean = suppress(&m, &s, &path);
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let audio = |e: hound::Error| crate::error::AppError::Audio(e.to_string());
    let mut w = hound::WavWriter::create(dst, spec).map_err(audio)?;
    for v in clean {
        w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(audio)?;
    }
    w.finalize().map_err(audio)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Речь»: слоги разной длины и высоты (детерминированно).
    fn speech(secs: f32, seed: u32) -> Vec<f32> {
        let n = (secs * SR) as usize;
        let mut s = seed | 1;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s as f32 / u32::MAX as f32
        };
        let mut out = vec![0f32; n];
        let mut i = 0;
        while i < n {
            let len = (0.08 + 0.25 * rnd()) * SR;
            let gap = (0.03 + 0.3 * rnd()) * SR;
            let f0 = 110.0 + 160.0 * rnd();
            let amp = 0.1 + 0.25 * rnd();
            for k in 0..len as usize {
                if i + k >= n {
                    break;
                }
                let t = (i + k) as f32 / SR;
                let env = (std::f32::consts::PI * k as f32 / len).sin();
                out[i + k] = amp * env * ((2.0 * std::f32::consts::PI * f0 * t).sin() + 0.4 * (2.0 * std::f32::consts::PI * 3.0 * f0 * t).sin());
            }
            i += (len + gap) as usize;
        }
        out
    }

    fn delayed(x: &[f32], secs: f32, gain: f32) -> Vec<f32> {
        let d = (secs * SR) as usize;
        (0..x.len()).map(|i| if i >= d { gain * x[i - d] } else { 0.0 }).collect()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum::<f32>()
    }

    #[test]
    fn finds_leak_of_other_track() {
        let me = speech(30.0, 1);
        let them = speech(30.0, 2);
        // Микрофон: я + собеседник из колонок (через 40 мс, тише).
        let mic: Vec<f32> = me.iter().zip(delayed(&them, 0.04, 0.4)).map(|(a, b)| a + b).collect();
        let p = detect(&mic, &them, 0.0, 0.6).expect("leak");
        assert!((p.lag_secs - 0.04).abs() <= 0.021, "{p:?}");
        // Без утечки — ничего.
        assert!(detect(&me, &them, 0.0, 0.6).is_none());
    }

    #[test]
    fn suppresses_the_leak_and_keeps_own_voice() {
        let me = speech(30.0, 10);
        let them = speech(30.0, 20);
        let leak = delayed(&them, 0.04, 0.4);
        let mic: Vec<f32> = me.iter().zip(&leak).map(|(a, b)| a + b).collect();
        let p = detect(&mic, &them, 0.0, 0.6).unwrap();
        // Без утечки в цели — сигнал не тронут.
        let only_me = suppress(&me, &them, &p);
        assert!(only_me.iter().zip(&me).map(|(c, m)| (c - m).powi(2)).sum::<f32>() < 1e-3 * energy(&me));
        let clean = suppress(&mic, &them, &p);
        assert_eq!(clean.len(), mic.len());
        // Остаток утечки: ошибка относительно «чистого меня» заметно меньше.
        let err_before: f32 = energy(&leak);
        let err_after: f32 = clean.iter().zip(&me).map(|(c, m)| (c - m).powi(2)).sum();
        assert!(err_after < 0.5 * err_before, "before {err_before} after {err_after}");
        // Свой голос не выброшен: энергия сохранилась хотя бы наполовину.
        assert!(energy(&clean) > 0.5 * energy(&me));
    }
}
