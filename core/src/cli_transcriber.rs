use crate::error::{AppError, AppResult};
use crate::transcript::Segment;
use crate::transcriber::Transcriber;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// User-supplied overrides for the whisper CLI invocation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscribeOptions {
    /// Absolute path to the whisper binary. Auto-discovered when absent.
    pub whisper_path: Option<String>,
    /// Model path (whisper.cpp) or model name / alias (openai-whisper).
    pub model: Option<String>,
    /// Language code, e.g. "ru", "en". Uses whisper auto-detect when absent.
    pub language: Option<String>,
    /// Словарь терминов (по строке или через запятую) — см. [`crate::vocab`].
    #[serde(default)]
    pub vocabulary: Option<String>,
    /// Языки разговора («ru», «en»): распознавание не выходит за них.
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    /// Распознавание на сервере ИИ пользователя (по желанию; звук уходит на
    /// сервер). `None` — на этом компьютере.
    #[serde(default)]
    pub cloud: Option<CloudAsr>,
}

/// Сервер распознавания речи (OpenAI-совместимый `/audio/transcriptions`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CloudAsr {
    pub base_url: String,
    pub api_key: String,
    /// Модель распознавания (например, `whisper-1`); пусто — по умолчанию.
    #[serde(default)]
    pub model: String,
}

/// Which whisper CLI family is being used; drives argument construction.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Flavor {
    WhisperCpp,
    OpenAiWhisper,
    FasterWhisper,
}

/// Infer flavor from a binary file-stem (lower-case comparison).
fn flavor_from_stem(stem: &str) -> Flavor {
    match stem {
        "whisper-cli" | "main" => Flavor::WhisperCpp,
        "whisper-ctranslate2" => Flavor::FasterWhisper,
        _ => Flavor::OpenAiWhisper,
    }
}

/// Search PATH for the first available whisper binary, returning its path and
/// the inferred CLI flavor.
fn discover_binary() -> Option<(PathBuf, Flavor)> {
    // Candidate names in priority order.
    let candidates: &[&str] = &["whisper-cli", "whisper-ctranslate2", "whisper", "main"];

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();

    for &name in candidates {
        let flavor = flavor_from_stem(name);
        for dir in &dirs {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some((candidate, flavor));
            }
            // On Windows also try the .exe variant.
            #[cfg(target_os = "windows")]
            {
                let exe = dir.join(format!("{}.exe", name));
                if exe.is_file() {
                    return Some((exe, flavor));
                }
            }
        }
    }
    None
}

/// Transcriber that shells out to a locally installed whisper CLI.
pub struct CliTranscriber {
    opts: TranscribeOptions,
}

impl CliTranscriber {
    pub fn new(opts: TranscribeOptions) -> Self {
        Self { opts }
    }
}

impl Transcriber for CliTranscriber {
    fn transcribe(&self, wav_path: &Path) -> AppResult<Vec<Segment>> {
        // ── Resolve binary + flavor ──────────────────────────────────────────
        let (binary, flavor) = if let Some(ref p) = self.opts.whisper_path {
            let pb = PathBuf::from(p);
            let stem = pb
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            (pb, flavor_from_stem(&stem))
        } else {
            discover_binary().ok_or_else(|| {
                AppError::Audio(
                    "no whisper CLI found; set the whisper path in Settings".into(),
                )
            })?
        };

        // ── Temp dir for JSON output ─────────────────────────────────────────
        let tmp = tempfile::tempdir()?;

        // ── Build and run command ─────────────────────────────────────────────
        let output = match flavor {
            Flavor::WhisperCpp => {
                let model = self.opts.model.as_deref().ok_or_else(|| {
                    AppError::Audio(
                        "whisper.cpp needs a model path — set it in Settings".into(),
                    )
                })?;
                let out_prefix = tmp.path().join("out");
                std::process::Command::new(&binary)
                    .args([
                        "-m",
                        model,
                        "-f",
                        &wav_path.to_string_lossy(),
                        "-oj",
                        "-of",
                        &out_prefix.to_string_lossy(),
                    ])
                    .output()?
            }
            Flavor::OpenAiWhisper | Flavor::FasterWhisper => {
                let model = self.opts.model.as_deref().unwrap_or("small");
                let mut cmd = std::process::Command::new(&binary);
                cmd.arg(wav_path)
                    .args(["--model", model])
                    .args(["--output_format", "json"])
                    .args(["--output_dir", &tmp.path().to_string_lossy()])
                    .args(["--task", "transcribe"]);
                if let Some(lang) = self.opts.language.as_deref() {
                    cmd.args(["--language", lang]);
                }
                cmd.output()?
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let truncated: String = stderr.chars().take(500).collect();
            return Err(AppError::Audio(format!("whisper CLI failed: {truncated}")));
        }

        // ── Read and parse the JSON output ────────────────────────────────────
        match flavor {
            Flavor::WhisperCpp => {
                let json_path = tmp.path().join("out.json");
                let json = std::fs::read_to_string(&json_path)?;
                parse_whispercpp_json(&json)
            }
            Flavor::OpenAiWhisper | Flavor::FasterWhisper => {
                let stem = wav_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("audio");
                let json_path = tmp.path().join(format!("{stem}.json"));
                let json = std::fs::read_to_string(&json_path)?;
                parse_openai_whisper_json(&json)
            }
        }
    }
}

// ── Public parser functions ───────────────────────────────────────────────────

/// Parse whisper.cpp `-oj` JSON output.
///
/// Expected shape:
/// ```json
/// {"transcription":[{"offsets":{"from":<ms>,"to":<ms>},"text":"..."},...]}
/// ```
/// Timestamps in milliseconds → converted to seconds.  Empty-after-trim entries
/// are silently skipped.
pub fn parse_whispercpp_json(json: &str) -> AppResult<Vec<Segment>> {
    #[derive(Deserialize)]
    struct Offsets {
        from: i64,
        to: i64,
    }
    #[derive(Deserialize)]
    struct Entry {
        offsets: Offsets,
        text: String,
    }
    #[derive(Deserialize)]
    struct Root {
        transcription: Vec<Entry>,
    }

    let root: Root = serde_json::from_str(json)?;
    let mut segments = Vec::new();
    for entry in root.transcription {
        let text = entry.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start_secs: entry.offsets.from as f64 / 1000.0,
            end_secs: entry.offsets.to as f64 / 1000.0,
            text,
        });
    }
    Ok(segments)
}

/// Parse openai-whisper / faster-whisper JSON output.
///
/// Expected shape:
/// ```json
/// {"segments":[{"start":<f64>,"end":<f64>,"text":"..."},...]}
/// ```
/// Timestamps already in seconds.  Empty-after-trim entries are skipped.
pub fn parse_openai_whisper_json(json: &str) -> AppResult<Vec<Segment>> {
    #[derive(Deserialize)]
    struct Entry {
        start: f64,
        end: f64,
        text: String,
    }
    #[derive(Deserialize)]
    struct Root {
        segments: Vec<Entry>,
    }

    let root: Root = serde_json::from_str(json).map_err(|e| {
        AppError::Audio(format!("could not parse whisper JSON output: {e}"))
    })?;
    let mut segments = Vec::new();
    for entry in root.segments {
        let text = entry.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start_secs: entry.start,
            end_secs: entry.end,
            text,
        });
    }
    Ok(segments)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_whispercpp_json_two_segments() {
        let json = r#"{
            "transcription": [
                {"offsets": {"from": 0, "to": 2500}, "text": "  Hello world  "},
                {"offsets": {"from": 3000, "to": 5000}, "text": "  Goodbye  "}
            ]
        }"#;
        let segs = parse_whispercpp_json(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start_secs, 0.0);
        assert_eq!(segs[0].end_secs, 2.5);
        assert_eq!(segs[0].text, "Hello world");
        assert_eq!(segs[1].start_secs, 3.0);
        assert_eq!(segs[1].end_secs, 5.0);
        assert_eq!(segs[1].text, "Goodbye");
    }

    #[test]
    fn parse_whispercpp_json_skips_empty_text() {
        let json = r#"{
            "transcription": [
                {"offsets": {"from": 0, "to": 1000}, "text": "  "},
                {"offsets": {"from": 1000, "to": 2000}, "text": "keep me"}
            ]
        }"#;
        let segs = parse_whispercpp_json(json).unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "keep me");
    }

    #[test]
    fn parse_openai_whisper_json_two_segments() {
        let json = r#"{
            "segments": [
                {"start": 0.0, "end": 3.5, "text": "  First segment  "},
                {"start": 4.0, "end": 7.0, "text": " Second segment "}
            ]
        }"#;
        let segs = parse_openai_whisper_json(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start_secs, 0.0);
        assert_eq!(segs[0].end_secs, 3.5);
        assert_eq!(segs[0].text, "First segment");
        assert_eq!(segs[1].start_secs, 4.0);
        assert_eq!(segs[1].end_secs, 7.0);
        assert_eq!(segs[1].text, "Second segment");
    }

    #[test]
    fn parse_openai_whisper_json_skips_empty_text() {
        let json = r#"{
            "segments": [
                {"start": 0.0, "end": 1.0, "text": "   "},
                {"start": 1.0, "end": 2.0, "text": "non-empty"}
            ]
        }"#;
        let segs = parse_openai_whisper_json(json).unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "non-empty");
    }

    #[test]
    fn parse_whispercpp_json_malformed_returns_err() {
        let result = parse_whispercpp_json("this is not json {{{{");
        assert!(result.is_err());
    }

    #[test]
    fn parse_openai_whisper_json_malformed_returns_err() {
        let result = parse_openai_whisper_json("not json at all >>>>");
        assert!(result.is_err());
    }
}
