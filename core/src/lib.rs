//! Memiro core domain logic: data model, storage, audio helpers, recorder
//! abstraction and service layer. Deliberately free of any GUI/Tauri
//! dependency so it builds and tests on any platform.

pub mod ai;
pub mod audio;
pub mod call_detector;
pub mod cluster;
pub mod cli_transcriber;
pub mod decode;
pub mod diarize;
pub mod edit;
pub mod enhance;
pub mod error;
pub mod langguard;
pub mod model;
pub mod models;
pub mod nemo_mel;
#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod profanity;
pub mod recorder;
pub mod rescue;
pub mod service;
pub mod storage;
pub mod transcript;
pub mod vocab;
pub mod transcriber;

#[cfg(feature = "whisper")]
pub mod whisper;

/// Реальный захват звука на Windows (WASAPI). На других ОС используется
/// `recorder::MockRecorder`.
#[cfg(target_os = "windows")]
pub mod wasapi_recorder;

/// Захват звука на macOS (CoreAudio + ScreenCaptureKit).
#[cfg(target_os = "macos")]
pub mod mac_recorder;
#[cfg(target_os = "macos")]
pub mod mac_audiotap;
