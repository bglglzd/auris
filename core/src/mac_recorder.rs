//! Захват звука на macOS: две дорожки, как на Windows.
//!
//! - `mic.wav` — микрофон через CoreAudio (`cpal`), любая частота → 16 кГц моно.
//! - `system.wav` — системный звук (собеседники): на macOS 14.2+ — Core Audio
//!   tap (`mac_audiotap`, разрешение «Только запись системного звука»), на
//!   13–14.1 — ScreenCaptureKit («Запись экрана и системного звука»). Звук
//!   самого Memiro исключён.
//!
//! Разрешения: микрофон — `NSMicrophoneUsageDescription` в Info.plist (macOS
//! спросит сам); системный звук — см. выше. Без него запись всё равно идёт
//! (микрофон), а дорожка собеседника остаётся тихой — об этом сообщает
//! `Recorder::warning`.
//!
//! Собирается только на macOS; компиляцию проверяет CI.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use screencapturekit::prelude::*;

use crate::audio::TrackSink;
use crate::error::{AppError, AppResult};
use crate::recorder::{Recorder, RecordingResult, TrackLevels};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Есть ли разрешение «Запись экрана и системного звука». `request = true` —
/// показать системный запрос (один раз; дальше — только в Настройках).
pub fn screen_capture_access(request: bool) -> bool {
    unsafe {
        if CGPreflightScreenCaptureAccess() {
            return true;
        }
        if request {
            return CGRequestScreenCaptureAccess();
        }
        false
    }
}

type Sink = Arc<Mutex<TrackSink>>;

/// Каким способом пишется звук собеседников.
pub fn system_audio_mode() -> &'static str {
    if crate::mac_audiotap::supported() {
        "audio"
    } else {
        "screen"
    }
}

/// Статус разрешения на звук собеседников: `granted` / `denied` /
/// `undetermined` / `unknown`.
pub fn system_audio_status() -> &'static str {
    if crate::mac_audiotap::supported() {
        crate::mac_audiotap::system_audio_status()
    } else if screen_capture_access(false) {
        "granted"
    } else {
        "undetermined"
    }
}

/// Показать системный запрос на звук собеседников.
pub fn request_system_audio() {
    if crate::mac_audiotap::supported() {
        crate::mac_audiotap::request_system_audio();
    } else {
        screen_capture_access(true);
    }
}

/// Захват звука собеседников.
enum SystemCapture {
    Tap(crate::mac_audiotap::SystemTap),
    Screen(SCStream),
}

struct Running {
    mic: Sink,
    system: Sink,
    mic_stop: mpsc::Sender<()>,
    mic_thread: Option<JoinHandle<()>>,
    stream: Option<SystemCapture>,
}

pub struct MacRecorder {
    running: Mutex<Option<Running>>,
    warning: Mutex<Option<String>>,
}

impl MacRecorder {
    pub fn new() -> Self {
        Self { running: Mutex::new(None), warning: Mutex::new(None) }
    }

}

impl Default for MacRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Поток микрофона: cpal-стрим живёт в своём потоке (на macOS `Stream` не
/// `Send`) до сигнала остановки. Результат старта — через `ready`.
fn spawn_mic(sink: Sink) -> AppResult<(mpsc::Sender<()>, JoinHandle<()>)> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let handle = std::thread::Builder::new()
        .name("memiro-mic".into())
        .spawn(move || {
            let build = || -> Result<cpal::Stream, String> {
                let host = cpal::default_host();
                let device = host
                    .default_input_device()
                    .ok_or("не найден микрофон (устройство ввода)")?;
                let supported = device.default_input_config().map_err(|e| e.to_string())?;
                let fmt = supported.sample_format();
                let config: cpal::StreamConfig = supported.into();
                let channels = config.channels as usize;
                sink.lock().unwrap().set_input_rate(config.sample_rate.0);
                let err_fn = |e| eprintln!("mic stream error: {e}");
                let stream = match fmt {
                    cpal::SampleFormat::F32 => {
                        let s = sink.clone();
                        device.build_input_stream(
                            &config,
                            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                s.lock().unwrap().push(data, channels);
                            },
                            err_fn,
                            None,
                        )
                    }
                    cpal::SampleFormat::I16 => {
                        let s = sink.clone();
                        device.build_input_stream(
                            &config,
                            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                                let f: Vec<f32> = data.iter().map(|&v| v as f32 / 32768.0).collect();
                                s.lock().unwrap().push(&f, channels);
                            },
                            err_fn,
                            None,
                        )
                    }
                    other => return Err(format!("формат микрофона не поддержан: {other:?}")),
                }
                .map_err(|e| e.to_string())?;
                stream.play().map_err(|e| e.to_string())?;
                Ok(stream)
            };
            match build() {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })
        .map_err(|e| AppError::Audio(format!("mic thread: {e}")))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok((stop_tx, handle)),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(AppError::Audio(format!("микрофон: {e}")))
        }
        Err(_) => Err(AppError::Audio("микрофон: поток не запустился".into())),
    }
}

/// Обработчик системного звука ScreenCaptureKit.
struct SystemAudio {
    sink: Sink,
}

impl SCStreamOutputTrait for SystemAudio {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, of_type: SCStreamOutputType) {
        if !matches!(of_type, SCStreamOutputType::Audio) {
            return;
        }
        let Ok(list) = sample.audio_buffer_list() else { return };
        let mut sink = self.sink.lock().unwrap();
        for buffer in &list {
            let data = buffer.data();
            if data.is_empty() {
                continue;
            }
            let samples: Vec<f32> = data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            sink.push(&samples, 1);
        }
    }
}

/// Запускает захват системного звука (16 кГц моно, без звука самого Memiro).
fn start_system(sink: Sink) -> Result<SCStream, String> {
    if !screen_capture_access(true) {
        return Err(
            "нет доступа к системному звуку: разрешите Memiro в «Системные настройки → \
             Конфиденциальность и безопасность → Запись экрана и системного звука» и \
             перезапустите приложение"
                .into(),
        );
    }
    let content = SCShareableContent::get().map_err(|e| format!("{e:?}"))?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or("не найден дисплей для захвата звука")?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build()
        .map_err(|e| format!("{e:?}"))?;
    // Видео не нужно — крошечный кадр раз в секунду; звук сразу в формате дорожки.
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_minimum_frame_interval(&CMTime::new(1, 1))
        .with_captures_audio(true)
        .with_excludes_current_process_audio(true)
        .with_sample_rate(16_000)
        .with_channel_count(1);
    let mut stream = SCStream::new(&filter, &config).map_err(|e| format!("{e:?}"))?;
    stream
        .add_output_handler(SystemAudio { sink }, SCStreamOutputType::Audio)
        .map_err(|e| format!("{e:?}"))?;
    stream.start_capture().map_err(|e| format!("{e:?}"))?;
    Ok(stream)
}

impl Recorder for MacRecorder {
    fn start(&self, mic_path: &Path, system_path: &Path) -> AppResult<()> {
        let mut running = self.running.lock().unwrap();
        if running.is_some() {
            return Err(AppError::InvalidState("already recording".into()));
        }
        *self.warning.lock().unwrap() = None;
        let mic: Sink = Arc::new(Mutex::new(TrackSink::create(mic_path, 48_000)?));
        let system: Sink = Arc::new(Mutex::new(TrackSink::create(system_path, 16_000)?));
        let (mic_stop, mic_thread) = spawn_mic(mic.clone())?;
        // macOS 14.2+: Core Audio tap (мягкое разрешение «только системный
        // звук»); если не вышло — прежний путь через ScreenCaptureKit.
        let mut stream = None;
        if crate::mac_audiotap::supported() {
            if crate::mac_audiotap::system_audio_status() == "denied" {
                *self.warning.lock().unwrap() = Some(
                    "нет доступа к звуку собеседников: включите Memiro AI в «Системные \
                     настройки → Конфиденциальность и безопасность → Запись экрана и \
                     системного звука → Только запись системного звука»"
                        .into(),
                );
            }
            match crate::mac_audiotap::SystemTap::start(system.clone()) {
                Ok(t) => stream = Some(SystemCapture::Tap(t)),
                Err(e) => eprintln!("system audio tap: {e}; fallback to ScreenCaptureKit"),
            }
        }
        if stream.is_none() {
            if let Ok(mut s) = system.lock() {
                s.set_input_rate(16_000);
            }
            stream = match start_system(system.clone()) {
                Ok(s) => Some(SystemCapture::Screen(s)),
                Err(e) => {
                    eprintln!("system audio: {e}");
                    *self.warning.lock().unwrap() = Some(e);
                    None
                }
            };
        }
        *running = Some(Running { mic, system, mic_stop, mic_thread: Some(mic_thread), stream });
        Ok(())
    }

    fn stop(&self) -> AppResult<RecordingResult> {
        let mut r = self
            .running
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AppError::InvalidState("not recording".into()))?;
        let _ = r.mic_stop.send(());
        if let Some(h) = r.mic_thread.take() {
            let _ = h.join();
        }
        match r.stream.take() {
            Some(SystemCapture::Screen(stream)) => {
                let _ = stream.stop_capture();
            }
            Some(SystemCapture::Tap(tap)) => drop(tap),
            None => {}
        }
        let secs = {
            let mut mic = r.mic.lock().unwrap();
            mic.finalize()?;
            mic.secs()
        };
        r.system.lock().unwrap().finalize()?;
        Ok(RecordingResult { duration_secs: secs as u64 })
    }

    fn is_recording(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    fn levels(&self) -> TrackLevels {
        let running = self.running.lock().unwrap();
        let Some(r) = running.as_ref() else { return TrackLevels::default() };
        let mic = r.mic.lock().unwrap().peak.swap(0, Ordering::Relaxed);
        let sys = r.system.lock().unwrap().peak.swap(0, Ordering::Relaxed);
        TrackLevels { mic: mic * 1000 / 32767, system: sys * 1000 / 32767 }
    }

    fn warning(&self) -> Option<String> {
        self.warning.lock().unwrap().clone()
    }
}
