//! Стенд эха в записи звонка: эталон pyannote sample.wav (голоса A и B по
//! RTTM) разведён на «меня» (A) и «собеседника» (B), эхо добавлено так, как
//! оно возникает в звонке. Сколько голосов находит разделение и что пишет
//! распознавание — как есть и после подавления эха (`echo.rs`).
//! `cargo run --release -p uxo-core --features diarize,parakeet --example echo_eval -- <data_dir> <sample.wav>`

use std::path::Path;

use uxo_core::diarize::OnnxDiarizer;
use uxo_core::parakeet::ParakeetTranscriber;

const SR: f32 = 16_000.0;
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

/// Только голос `who` (без перекрытий с другим).
fn only(x: &[f32], who: &str) -> Vec<f32> {
    (0..x.len())
        .map(|i| {
            let t = i as f64 / SR as f64;
            let act: Vec<&str> = RTTM.iter().filter(|r| r.0 <= t && t < r.0 + r.1).map(|r| r.2).collect();
            if act == [who] {
                x[i]
            } else {
                0.0
            }
        })
        .collect()
}

/// Биквад (RBJ): высокие/низкие частоты.
fn biquad(x: &[f32], f0: f32, high: bool) -> Vec<f32> {
    let w = 2.0 * std::f32::consts::PI * f0 / SR;
    let (c, s) = (w.cos(), w.sin());
    let alpha = s / (2.0 * 0.707);
    let (b0, b1, b2) = if high { ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0) } else { ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0) };
    let (a0, a1, a2) = (1.0 + alpha, -2.0 * c, 1.0 - alpha);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    x.iter()
        .map(|&v| {
            let y = (b0 * v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
            x2 = x1;
            x1 = v;
            y2 = y1;
            y1 = y;
            y
        })
        .collect()
}

/// «Телефон»: полоса 300–3400 Гц и лёгкая перегрузка динамика.
fn phone(x: &[f32]) -> Vec<f32> {
    let y = biquad(&biquad(x, 300.0, true), 3400.0, false);
    y.iter().map(|v| (2.0 * v).tanh() / 2.0).collect()
}

fn delay(x: &[f32], secs: f32, gain: f32) -> Vec<f32> {
    let d = (secs * SR) as usize;
    (0..x.len()).map(|i| if i >= d { gain * x[i - d] } else { 0.0 }).collect()
}

fn mix(parts: &[&[f32]]) -> Vec<f32> {
    let n = parts.iter().map(|p| p.len()).max().unwrap_or(0);
    (0..n).map(|i| parts.iter().map(|p| p.get(i).copied().unwrap_or(0.0)).sum()).collect()
}

fn text(asr: &ParakeetTranscriber, x: &[f32]) -> String {
    let (segs, _) = asr.transcribe_samples(x).expect("asr");
    segs.iter().map(|s| s.text.trim()).collect::<Vec<_>>().join(" ")
}



/// Реплики дорожки с голосами (как в приложении: разделение → назначение).
fn voiced(d: &OnnxDiarizer, x: &[f32], segs: Vec<uxo_core::transcript::Segment>) -> (usize, String) {
    let tmp = std::env::temp_dir().join(format!("echo_eval_v_{}.wav", std::process::id()));
    write(&tmp, x);
    let w = d.embed(&tmp, &|_, _| {}).expect("embed");
    let t = uxo_core::transcript::assign_speakers(segs, uxo_core::cluster::diarize_windows(&w, None));
    let n = t.segments.iter().map(|s| s.speaker.as_str()).collect::<std::collections::BTreeSet<_>>().len();
    (n, t.segments.iter().map(|s| format!("[{} {:.1}] {}", s.speaker, s.start_secs, s.text.trim())).collect::<Vec<_>>().join(" | "))
}

fn segs(asr: &ParakeetTranscriber, x: &[f32]) -> Vec<uxo_core::transcript::Segment> {
    asr.transcribe_samples(x).expect("asr").0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = Path::new(&args[1]);
    let d = OnnxDiarizer::managed(data, None, &|_| {}).expect("diar models");
    let asr = ParakeetTranscriber::managed(data, &|_| {}).expect("asr model");
    let x = read(Path::new(&args[2]));
    let (a, b) = (only(&x, "A"), only(&x, "B"));
    println!("Эталон A: {}", text(&asr, &a));
    println!("Эталон B: {}\n", text(&asr, &b));

    // 1) Звонок на компьютере: системный звук = собеседник + моё эхо (у него
    //    не работает эхоподавление, 0.45 с); микрофон = я + собеседник из колонок.
    let system = mix(&[&phone(&b), &phone(&delay(&a, 0.45, 0.5))]);
    let mic = mix(&[&a, &delay(&biquad(&system, 4000.0, false), 0.03, 0.35)]);
    println!("1) Звонок на компьютере (колонки), две дорожки.");
    let (n, t) = voiced(&d, &system, segs(&asr, &system));
    println!("   БЫЛО  звонок: {n} гол. | {t}");
    println!("   БЫЛО  «Я»: {}", text(&asr, &mic));
    let mic_clean = match uxo_core::echo::detect(&mic, &system, 0.0, 0.3) {
        Some(p) => {
            println!("   собеседник в микрофоне: {:.2} с (корр. {:.2}) → подавление", p.lag_secs, p.corr);
            uxo_core::echo::suppress(&mic, &system, &p)
        }
        None => mic.clone(),
    };
    let (m, s, dm, ds) = uxo_core::transcript::drop_cross_echo(segs(&asr, &mic_clean), segs(&asr, &system));
    let (n, t) = voiced(&d, &system, s);
    println!("   СТАЛО звонок: {n} гол. | {t}   (убрано эхо: из «Я» {dm}, из звонка {ds})");
    println!("   СТАЛО «Я»: {}", m.iter().map(|s| s.text.trim()).collect::<Vec<_>>().join(" "));

    // 2) Телефон на громкой связи, пишет микрофон: я напрямую, собеседник и
    //    моё эхо — из динамика телефона (0.35 с).
    let room = mix(&[&a, &phone(&b), &phone(&delay(&a, 0.35, 0.6))]);
    println!("\n2) Телефон на громкой связи, одна дорожка.");
    let raw = segs(&asr, &room);
    let (n, t) = voiced(&d, &room, raw.clone());
    println!("   БЫЛО  {n} гол. | {t}");
    let (clean, k) = uxo_core::transcript::drop_self_echo(raw);
    let (n, t) = voiced(&d, &room, clean);
    println!("   СТАЛО {n} гол. | {t}   (убрано повторов: {k})");

    // 3) Без эха: ничего не находим и ничего не портим.
    println!("\n3) Без эха (наушники): {:?}", uxo_core::echo::detect(&a, &b, 0.0, 0.3));
}
