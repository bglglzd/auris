//! Стенд диаризации в шуме: эталон pyannote sample.wav (2 голоса, RTTM) +
//! уличный шум; сколько речи нашла сегментация, сколько фрагментов получили
//! эмбеддинг и точность разметки — как есть и с подготовкой звука.
//! `cargo run --release -p uxo-core --features diarize --example diar_noise_eval -- <data_dir> <sample.wav>`

use std::path::Path;

use uxo_core::cluster::{cluster_windows, count_speakers, EmbWindow};
use uxo_core::diarize::OnnxDiarizer;

const RTTM: &[(f64, f64, &str)] = &[
    (6.690, 0.430, "A"),
    (7.550, 0.800, "B"),
    (8.320, 1.700, "A"),
    (9.920, 0.960, "B"),
    (10.570, 4.130, "A"),
    (14.490, 3.430, "B"),
    (18.050, 3.440, "A"),
    (18.150, 0.440, "B"),
    (21.780, 6.720, "B"),
    (27.850, 2.150, "A"),
];

fn read(path: &Path) -> Vec<f32> {
    hound::WavReader::open(path).unwrap().samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

fn write(path: &Path, x: &[f32]) {
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for v in x {
        w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn street(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0
    };
    let mut brown = 0.0f32;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            brown = (brown + 0.02 * rnd()).clamp(-1.0, 1.0) * 0.98;
            let t = i as f32 / 16000.0;
            let honk = if (t % 3.7) < 0.4 { 0.3 * (2.0 * std::f32::consts::PI * 420.0 * t).sin() } else { 0.0 };
            brown + 0.15 * rnd() + honk
        })
        .collect();
    let mean = out.iter().sum::<f32>() / n as f32;
    for v in &mut out {
        *v -= mean;
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Точность: доля кадров (где говорит ровно один по эталону), размеченных
/// верно при лучшем сопоставлении меток; плюс доля покрытых кадров.
fn accuracy(windows: &[EmbWindow], labels: &[u32]) -> (f64, f64) {
    let mut conf = std::collections::HashMap::<(&str, u32), usize>::new();
    let (mut frames, mut covered) = (0usize, 0usize);
    let mut t = 0.0;
    while t < 30.0 {
        let refs: Vec<&str> = RTTM.iter().filter(|r| r.0 <= t && t < r.0 + r.1).map(|r| r.2).collect();
        if refs.len() == 1 {
            frames += 1;
            if let Some(i) = windows.iter().position(|w| w.spans().iter().any(|&(a, b)| a <= t && t < b)) {
                covered += 1;
                *conf.entry((refs[0], labels[i])).or_default() += 1;
            }
        }
        t += 0.01;
    }
    let g = |a: &str, l: u32| conf.get(&(a, l)).copied().unwrap_or(0);
    let best = (g("A", 0) + g("B", 1)).max(g("A", 1) + g("B", 0)).max(g("A", 0) + g("B", 2)).max(g("A", 2) + g("B", 0)).max(g("A", 1) + g("B", 2)).max(g("A", 2) + g("B", 1));
    (best as f64 / frames.max(1) as f64, covered as f64 / frames.max(1) as f64)
}

fn report(tag: &str, windows: &[EmbWindow]) {
    let valid = windows.iter().filter(|w| !w.embedding.is_empty()).count();
    let speech: f64 = windows.iter().map(|w| w.speech_secs()).sum();
    let auto = cluster_windows(windows, None);
    let two = cluster_windows(windows, Some(2));
    let (acc, cov) = accuracy(windows, &two);
    println!(
        "  {tag:<14} фрагментов {:>3} (с эмбеддингом {valid:>3}), речь {speech:5.1} с | авто: {} гол. | k=2: точность {acc:.2}, покрытие {cov:.2}",
        windows.len(),
        count_speakers(&auto),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let d = OnnxDiarizer::managed(Path::new(&args[1]), None, &|_| {}).expect("models");
    let clean = read(Path::new(&args[2]));
    let tmp = std::env::temp_dir().join(format!("diar_noise_{}.wav", std::process::id()));
    let noise = street(clean.len(), 42);
    for snr in [99.0f32, 15.0, 10.0, 5.0, 0.0] {
        let k = if snr > 90.0 { 0.0 } else { rms(&clean) / (rms(&noise) * 10f32.powf(snr / 20.0)) };
        let mut noisy: Vec<f32> = clean.iter().zip(&noise).map(|(s, n)| s + k * n).collect();
        let peak = noisy.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        for v in &mut noisy {
            *v *= 0.9 / peak;
        }
        println!("SNR {snr} дБ:");
        write(&tmp, &noisy);
        report("как есть", &d.embed(&tmp, &|_, _| {}).unwrap());
        write(&tmp, &uxo_core::enhance::denoise(&noisy));
        report("шумодав", &d.embed(&tmp, &|_, _| {}).unwrap());
    }
}
