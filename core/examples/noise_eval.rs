//! Стенд второго прохода: как Parakeet распознаёт речь в шуме (уличный шум +
//! чужие голоса) — как есть и после шумоподавления; уверенность модели и
//! расхождение со «чистым» текстом (WER).
//! `cargo run --release -p uxo-core --features parakeet --example noise_eval -- <data_dir> <speech.wav> <babble.wav>...`

use std::path::Path;

use uxo_core::enhance::{denoise, snr_db};
use uxo_core::parakeet::ParakeetTranscriber;

fn load(path: &str) -> Vec<f32> {
    let tmp = std::env::temp_dir().join(format!("noise_eval_{}.wav", std::process::id()));
    uxo_core::decode::decode_to_wav_16k_mono(Path::new(path), &tmp).expect("decode");
    hound::WavReader::open(&tmp).unwrap().samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// «Улица»: коричневый шум + гудки + чужая речь.
fn street(n: usize, babble: &[Vec<f32>], seed: u64) -> Vec<f32> {
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
    };
    let mut brown = 0.0f32;
    let mut out = vec![0.0f32; n];
    for (i, o) in out.iter_mut().enumerate() {
        brown = (brown + 0.02 * rnd()).clamp(-1.0, 1.0) * 0.98;
        let t = i as f32 / 16000.0;
        let honk = if (t % 3.7) < 0.4 { 0.3 * (2.0 * std::f32::consts::PI * 420.0 * t).sin() } else { 0.0 };
        *o = brown + 0.15 * rnd() + honk;
    }
    for (k, b) in babble.iter().enumerate() {
        let shift = (k * 7919) % b.len().max(1);
        for (i, o) in out.iter_mut().enumerate() {
            *o += 0.8 * b[(i + shift) % b.len()];
        }
    }
    out
}

fn mix(speech: &[f32], noise: &[f32], snr: f32) -> Vec<f32> {
    let k = rms(speech) / (rms(noise) * 10f32.powf(snr / 20.0));
    let y: Vec<f32> = speech.iter().zip(noise).map(|(s, n)| s + k * n).collect();
    let peak = y.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    y.iter().map(|v| v / peak * 0.9).collect()
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

fn wer(reference: &str, hyp: &str) -> f32 {
    let (r, h) = (words(reference), words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(r[i - 1] != h[j - 1]));
        }
        prev = cur;
    }
    prev[h.len()] as f32 / r.len().max(1) as f32
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let asr = ParakeetTranscriber::managed(Path::new(&args[1]), &|_| {}).expect("model");
    let speech = load(&args[2]);
    let babble: Vec<Vec<f32>> = args[3..].iter().map(|p| load(p)).collect();
    let text = |x: &[f32]| {
        let (segs, conf) = asr.transcribe_samples(x).expect("asr");
        (segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" "), conf.unwrap_or(0.0))
    };
    // Второй проход: короткие окна (5 с с перекрытием 1 с) — модель меньше
    // «сдаётся» на всё окно, когда мешают чужие голоса.
    let short = |x: &[f32]| {
        let mut out = Vec::new();
        let (win, hop) = (5 * 16000, 4 * 16000);
        let mut a = 0;
        while a < x.len() {
            let b = (a + win).min(x.len());
            let toks = asr.transcribe_chunk(&x[a..b]).expect("asr");
            for t in toks {
                let at = a as f64 / 16000.0 + t.secs;
                // Перекрытие: берём токен только из «своей» части окна.
                if a == 0 || t.secs >= 0.5 {
                    out.push((at, t.piece));
                }
            }
            if b == x.len() {
                break;
            }
            a += hop;
        }
        out.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
        out.into_iter().map(|(_, p)| p).collect::<String>().replace('▁', " ").trim().to_string()
    };
    let (reference, c0) = text(&speech);
    println!("чисто   snr {:5.1} conf {c0:.3}: {reference}", snr_db(&speech).unwrap_or(0.0));
    let mut noise = street(speech.len(), &babble, 42);
    // Без постоянной составляющей (иначе «шум» — сдвиг уровня, а не звук).
    let mean = noise.iter().sum::<f32>() / noise.len().max(1) as f32;
    for v in &mut noise {
        *v -= mean;
    }
    for snr in [10.0, 5.0, 0.0, -3.0] {
        let noisy = mix(&speech, &noise, snr);
        let t = std::time::Instant::now();
        let (raw, c1) = text(&noisy);
        let d = denoise(&noisy);
        let dn_ms = t.elapsed().as_millis();
        let (den, c2) = text(&d);
        let sh = short(&noisy);
        let shd = short(&d);
        println!("    короткие окна WER {:.2}: {sh}\n    короткие+шумодав WER {:.2}: {shd}", wer(&reference, &sh), wer(&reference, &shd));
        println!(
            "SNR {snr:>4} (оценка {:5.1}) | как есть WER {:.2} conf {c1:.3} | шумодав WER {:.2} conf {c2:.3} ({dn_ms} мс)\n    как есть: {raw}\n    шумодав:  {den}",
            snr_db(&noisy).unwrap_or(0.0),
            wer(&reference, &raw),
            wer(&reference, &den),
        );
    }
}
