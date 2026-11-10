//! Реальный захват звука на Windows через WASAPI (крейт `wasapi`).
//!
//! Реализует [`crate::recorder::Recorder`], поэтому подменяет `MockRecorder`
//! без изменений в остальном коде. Пишет ДВЕ дорожки в формате конвейера
//! (WAV, моно, 16 кГц, 16 бит): микрофон («Я») и системный звук через
//! loopback («Собеседник»).
//!
//! Собирается только на Windows (крейт `wasapi`); компиляцию проверяет CI.
//! Основа — пример `wasapi/examples/loopback.rs`.
//!
//! Ключевой приём: shared-режим с `autoconvert: true` просит WASAPI самому
//! сконвертировать поток устройства в запрошенные 16 кГц/моно/16 бит.

#![cfg(target_os = "windows")]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasapi::{AudioCaptureClient, AudioClient, Direction, Handle, SampleType, ShareMode, WaveFormat};

use crate::audio::wav_duration_secs;
use crate::error::{AppError, AppResult};
use crate::recorder::{Recorder, RecordingResult, TrackLevels};

const SAMPLE_RATE: u32 = 16_000;
/// Порог заглохания loopback: нет данных дольше — переоткрываем поток.
const STALL_SECS: f64 = 2.0;
/// Сколько подряд неудачных переоткрытий терпим, прежде чем сдаться.
const MAX_REOPEN_FAILS: u32 = 5;

/// Что захватываем: микрофон или системный звук (loopback).
#[derive(Clone, Copy)]
enum Source {
    Mic,
    Loopback,
}

/// Захватчик двух дорожек на WASAPI.
pub struct WasapiRecorder {
    running: Mutex<Option<Running>>,
    mic_level: Arc<AtomicU32>,
    system_level: Arc<AtomicU32>,
}

struct Running {
    stop: Arc<AtomicBool>,
    mic: JoinHandle<AppResult<()>>,
    system: JoinHandle<AppResult<()>>,
    mic_path: PathBuf,
}

impl WasapiRecorder {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(None),
            mic_level: Arc::new(AtomicU32::new(0)),
            system_level: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Default for WasapiRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder for WasapiRecorder {
    fn start(&self, mic_path: &Path, system_path: &Path) -> AppResult<()> {
        let mut running = self.running.lock().unwrap();
        if running.is_some() {
            return Err(AppError::InvalidState("already recording".into()));
        }
        let stop = Arc::new(AtomicBool::new(false));

        let mic_pb = mic_path.to_path_buf();
        let sys_pb = system_path.to_path_buf();
        let stop_mic = stop.clone();
        let stop_sys = stop.clone();

        let mic_level = self.mic_level.clone();
        let sys_level = self.system_level.clone();

        let mic = std::thread::spawn(move || {
            capture_loop(mic_pb, Source::Mic, stop_mic, mic_level)
        });
        let system = std::thread::spawn(move || {
            capture_loop(sys_pb, Source::Loopback, stop_sys, sys_level)
        });

        *running = Some(Running {
            stop,
            mic,
            system,
            mic_path: mic_path.to_path_buf(),
        });
        Ok(())
    }

    fn stop(&self) -> AppResult<RecordingResult> {
        let running = {
            let mut guard = self.running.lock().unwrap();
            guard
                .take()
                .ok_or_else(|| AppError::InvalidState("not recording".into()))?
        };
        running.stop.store(true, Ordering::Relaxed);
        // Оба файла должны закрыться до склейки, даже если первый поток упал.
        let joined = join_capture_threads(running.mic, running.system);
        if let Err(ref error) = joined {
            dlog(&running.mic_path, &format!("stop: {error}"));
        }
        joined?;

        let duration_secs = wav_duration_secs(&running.mic_path).unwrap_or(0);
        Ok(RecordingResult { duration_secs })
    }

    fn is_recording(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    fn levels(&self) -> TrackLevels {
        if self.running.lock().unwrap().is_none() {
            return TrackLevels::default();
        }
        // Читаем-и-сбрасываем пик с прошлого опроса; нормализуем 0..32767 → 0..1000.
        let mic_raw = self.mic_level.swap(0, Ordering::Relaxed);
        let sys_raw = self.system_level.swap(0, Ordering::Relaxed);
        TrackLevels {
            mic: mic_raw * 1000 / 32767,
            system: sys_raw * 1000 / 32767,
        }
    }
}

fn join_capture_threads(
    mic: std::thread::JoinHandle<AppResult<()>>,
    system: std::thread::JoinHandle<AppResult<()>>,
) -> AppResult<()> {
    let mic_result = mic.join();
    let system_result = system.join();
    mic_result.map_err(|_| AppError::Audio("mic capture thread panicked".into()))??;
    system_result.map_err(|_| AppError::Audio("system capture thread panicked".into()))??;
    Ok(())
}

#[cfg(test)]
mod join_tests {
    use super::*;

    #[test]
    fn mic_panic_or_error_still_joins_the_system_worker() {
        for panic in [false, true] {
            let finished = Arc::new(AtomicBool::new(false));
            let other = finished.clone();
            let mic = std::thread::spawn(move || {
                if panic { panic!("synthetic mic panic"); }
                Err(AppError::Audio("synthetic mic error".into()))
            });
            let system = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                other.store(true, Ordering::SeqCst);
                Ok(())
            });
            assert!(join_capture_threads(mic, system).is_err());
            assert!(finished.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn system_panic_is_reported_after_both_workers_exit() {
        let mic = std::thread::spawn(|| Ok(()));
        let system = std::thread::spawn(|| panic!("synthetic system panic"));
        assert!(join_capture_threads(mic, system).is_err());
    }
}

/// Дописывает диагностику захвата в backend-лог. Путь выводим из пути дорожки:
/// `<data_root>/meetings/<id>/mic.wav` → `<data_root>/3uxo.log` (без проброса
/// data_root через сигнатуры).
fn dlog(track_path: &Path, msg: &str) {
    if let Some(root) = track_path.ancestors().nth(3) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("3uxo.log"))
        {
            let _ = writeln!(f, "[wasapi] {msg}");
        }
    }
}

/// Открывает (или переоткрывает) поток захвата на ТЕКУЩЕМ дефолтном устройстве.
/// Для loopback берёт Render-устройство, но клиент инициализирует как Capture.
/// Возврат: (клиент, capture-клиент, event-handle) с уже запущенным стримом.
fn open_stream(
    source: Source,
    format: &WaveFormat,
) -> AppResult<(AudioClient, AudioCaptureClient, Handle)> {
    let device_direction = match source {
        Source::Mic => Direction::Capture,
        Source::Loopback => Direction::Render,
    };
    let device = wasapi::get_default_device(&device_direction)
        .map_err(|e| AppError::Audio(format!("wasapi: default device: {e}")))?;
    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|e| AppError::Audio(format!("wasapi: get_iaudioclient: {e}")))?;
    let (_def_time, min_time) = audio_client
        .get_periods()
        .map_err(|e| AppError::Audio(format!("wasapi: get_periods: {e}")))?;
    audio_client
        .initialize_client(format, min_time, &Direction::Capture, &ShareMode::Shared, true)
        .map_err(|e| AppError::Audio(format!("wasapi: initialize_client: {e}")))?;
    let h_event = audio_client
        .set_get_eventhandle()
        .map_err(|e| AppError::Audio(format!("wasapi: eventhandle: {e}")))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| AppError::Audio(format!("wasapi: capture_client: {e}")))?;
    audio_client
        .start_stream()
        .map_err(|e| AppError::Audio(format!("wasapi: start_stream: {e}")))?;
    Ok((audio_client, capture_client, h_event))
}

/// Поток захвата одной дорожки: пишет WAV до сигнала stop.
/// Заполняет тишиной по стенным часам (дорожки равной длины) и — для loopback —
/// переоткрывает поток при заглохании/ошибке (смена устройства, инвалидация).
fn capture_loop(
    path: PathBuf,
    source: Source,
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
) -> AppResult<()> {
    let label = match source {
        Source::Mic => "mic",
        Source::Loopback => "system",
    };
    let _ = wasapi::initialize_mta();

    let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE as usize, 1, None);
    let blockalign = format.get_blockalign() as usize;

    let (mut client, mut capture, mut h_event) = open_stream(source, &format)?;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(&path, spec).map_err(|e| AppError::Audio(e.to_string()))?;

    dlog(&path, &format!("{label}: stream started (blockalign={blockalign})"));

    // Диагностика.
    let mut reads: u64 = 0;
    let mut events_ok: u64 = 0;
    let mut samples: u64 = 0;
    let mut peak: i32 = 0;

    // Синхронизация по стенным часам и watchdog.
    let started = Instant::now();
    let mut samples_written: u64 = 0; // включая тишину-заполнение
    let mut silence_filled: u64 = 0;
    let mut last_data = Instant::now();
    let mut reopen_fails: u32 = 0;
    let mut gave_up = false;

    let mut queue: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        let mut need_reopen = false;

        // --- Чтение (для loopback ошибка не фатальна) ---
        // `read_from_device_to_deque` возвращает BufferFlags (wasapi 0.15) —
        // флаги буфера нам не нужны, важен лишь факт успешного чтения.
        match capture.read_from_device_to_deque(&mut queue) {
            Ok(_) => {}
            Err(e) => match source {
                Source::Loopback => {
                    if !gave_up {
                        dlog(&path, &format!("{label}: read error: {e} — will reopen"));
                        need_reopen = true;
                    }
                }
                Source::Mic => return Err(AppError::Audio(format!("wasapi: read: {e}"))),
            },
        }
        reads += 1;

        // --- Слив очереди в сэмплы ---
        let mut produced: u64 = 0;
        while queue.len() >= blockalign {
            let lo = queue.pop_front().unwrap();
            let hi = queue.pop_front().unwrap();
            for _ in 2..blockalign {
                queue.pop_front();
            }
            let sample = i16::from_le_bytes([lo, hi]);
            let amp = (sample as i32).abs();
            if amp > peak {
                peak = amp;
            }
            level.fetch_max(amp as u32, Ordering::Relaxed);
            writer
                .write_sample(sample)
                .map_err(|e| AppError::Audio(e.to_string()))?;
            samples_written += 1;
            samples += 1;
            produced += 1;
        }
        if produced > 0 {
            last_data = Instant::now();
        }

        // --- Заполнение тишиной по стенным часам (обе дорожки) ---
        let expected = (started.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
        if expected > samples_written + (SAMPLE_RATE as u64 / 10) {
            let pad = expected - samples_written;
            for _ in 0..pad {
                writer
                    .write_sample(0i16)
                    .map_err(|e| AppError::Audio(e.to_string()))?;
            }
            samples_written += pad;
            silence_filled += pad;
        }

        // --- Watchdog заглохания (только loopback) ---
        if matches!(source, Source::Loopback)
            && !gave_up
            && !need_reopen
            && last_data.elapsed().as_secs_f64() > STALL_SECS
        {
            dlog(&path, &format!("{label}: stalled {STALL_SECS}s — will reopen"));
            need_reopen = true;
        }

        // --- Переоткрытие потока ---
        if need_reopen && !gave_up {
            match open_stream(source, &format) {
                Ok((c, cc, h)) => {
                    let _ = client.stop_stream();
                    client = c;
                    capture = cc;
                    h_event = h;
                    reopen_fails = 0;
                    dlog(&path, &format!("{label}: reopened ok"));
                }
                Err(e) => {
                    reopen_fails += 1;
                    dlog(&path, &format!("{label}: reopen failed ({reopen_fails}): {e}"));
                    if reopen_fails >= MAX_REOPEN_FAILS {
                        gave_up = true;
                        dlog(&path, &format!("{label}: giving up — silence to end"));
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            last_data = Instant::now(); // не долбить переоткрытие каждый тик
        }

        // --- Темп опроса ---
        match source {
            Source::Mic => {
                if h_event.wait_for_event(100).is_ok() {
                    events_ok += 1;
                }
            }
            Source::Loopback => {
                std::thread::sleep(Duration::from_millis(8));
            }
        }
    }

    // Финальное выравнивание до полной длительности.
    let expected = (started.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
    if expected > samples_written {
        let pad = expected - samples_written;
        for _ in 0..pad {
            let _ = writer.write_sample(0i16);
        }
        silence_filled += pad;
    }

    dlog(
        &path,
        &format!(
            "{label}: stop — reads={reads}, events_ok={events_ok}, samples={samples}, peak={peak}, silence_filled={silence_filled}, reopen_fails={reopen_fails}"
        ),
    );

    let _ = client.stop_stream();
    writer
        .finalize()
        .map_err(|e| AppError::Audio(e.to_string()))?;
    Ok(())
}
