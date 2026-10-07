//! Распознавание речи на сервере ИИ пользователя (по желанию, через его ключ):
//! OpenAI-совместимый `POST {base}/audio/transcriptions` (OpenAI Whisper /
//! gpt-4o-transcribe, Groq, локальные серверы с тем же API).
//!
//! Звук уходит на сервер — поэтому это отдельный явный выбор в настройках;
//! по умолчанию распознавание локальное. Запись отправляется кусками до
//! 10 минут (лимит размера файла у серверов — 25 МБ), разрезанными в паузах;
//! время реплик пересчитывается на всю запись.

use std::io::Cursor;
use std::path::Path;

use crate::ai::AiConfig;
use crate::error::{AppError, AppResult};
use crate::transcript::Segment;

const SR: usize = 16_000;
/// Кусок для отправки: ≤ 10 минут (16 кГц × 16 бит ≈ 19 МБ).
const CHUNK_SECS: usize = 600;

/// Модель распознавания по умолчанию (есть у OpenAI и большинства совместимых).
pub const DEFAULT_MODEL: &str = "whisper-1";

/// WAV (16 кГц моно i16) в памяти.
fn wav_bytes(samples: &[f32]) -> AppResult<Vec<u8>> {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).map_err(|e| AppError::Audio(e.to_string()))?;
        for v in samples {
            w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(|e| AppError::Audio(e.to_string()))?;
        }
        w.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
    }
    Ok(cur.into_inner())
}

/// Тело multipart/form-data: текстовые поля и файл `file`.
pub fn multipart(boundary: &str, fields: &[(&str, &str)], file: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(file.len() + 1024);
    for (k, v) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Реплики из ответа сервера: `verbose_json` с `segments` или просто `text`
/// (тогда одна реплика на весь кусок). Время сдвигается на `offset`.
pub fn parse_response(v: &serde_json::Value, offset: f64, chunk_secs: f64) -> Vec<Segment> {
    if let Some(segs) = v.get("segments").and_then(|s| s.as_array()) {
        return segs
            .iter()
            .filter_map(|s| {
                let text = s.get("text")?.as_str()?.trim().to_string();
                let a = s.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let b = s.get("end").and_then(|x| x.as_f64()).unwrap_or(a);
                (!text.is_empty()).then(|| Segment { start_secs: offset + a, end_secs: offset + b.max(a), text })
            })
            .collect();
    }
    match v.get("text").and_then(|t| t.as_str()).map(str::trim) {
        Some(t) if !t.is_empty() => vec![Segment { start_secs: offset, end_secs: offset + chunk_secs, text: t.to_string() }],
        _ => Vec::new(),
    }
}

fn http_err(e: ureq::Error) -> AppError {
    match e {
        ureq::Error::Status(404, _) | ureq::Error::Status(405, _) => AppError::Http(
            "сервер ИИ не умеет распознавать речь (нет /audio/transcriptions) — выберите распознавание на этом компьютере".into(),
        ),
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            AppError::Http(format!("HTTP {code}: {}", body.chars().take(400).collect::<String>()))
        }
        other => AppError::Http(other.to_string()),
    }
}

/// Отправляет кусок звука и возвращает реплики со временем записи.
fn send_chunk(
    config: &AiConfig,
    model: &str,
    language: Option<&str>,
    prompt: Option<&str>,
    samples: &[f32],
    offset: f64,
) -> AppResult<Vec<Segment>> {
    let boundary = format!("memiro-{}", uuid::Uuid::new_v4().simple());
    let mut fields: Vec<(&str, &str)> = vec![("model", model), ("response_format", "verbose_json")];
    if let Some(l) = language.map(str::trim).filter(|l| !l.is_empty() && *l != "auto") {
        fields.push(("language", l));
    }
    if let Some(p) = prompt.map(str::trim).filter(|p| !p.is_empty()) {
        fields.push(("prompt", p));
    }
    let body = multipart(&boundary, &fields, &wav_bytes(samples)?);
    let url = format!("{}/audio/transcriptions", config.base_url.trim_end_matches('/'));
    let send = |fields_body: &[u8]| {
        ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", config.api_key))
            .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .timeout(std::time::Duration::from_secs(600))
            .send_bytes(fields_body)
    };
    let resp = match send(&body) {
        // Модель без verbose_json (gpt-4o-transcribe) — просим обычный json.
        Err(ureq::Error::Status(400, r)) => {
            let msg = r.into_string().unwrap_or_default();
            if !msg.contains("response_format") {
                return Err(AppError::Http(format!("HTTP 400: {}", msg.chars().take(400).collect::<String>())));
            }
            let fields: Vec<(&str, &str)> =
                fields.iter().map(|&(k, v)| if k == "response_format" { (k, "json") } else { (k, v) }).collect();
            send(&multipart(&boundary, &fields, &wav_bytes(samples)?)).map_err(http_err)?
        }
        other => other.map_err(http_err)?,
    };
    let v: serde_json::Value = resp.into_json().map_err(|e| AppError::Http(e.to_string()))?;
    Ok(parse_response(&v, offset, samples.len() as f64 / SR as f64))
}

/// Распознаёт WAV-файл (16 кГц моно) на сервере. `on_progress(done, total)`.
pub fn transcribe_wav(
    config: &AiConfig,
    model: &str,
    language: Option<&str>,
    prompt: Option<&str>,
    wav: &Path,
    on_progress: &dyn Fn(usize, usize),
) -> AppResult<Vec<Segment>> {
    let audio = crate::enhance::read_wav_f32(wav)?;
    if audio.is_empty() {
        return Ok(Vec::new());
    }
    let chunks = crate::audio::quiet_chunks(&audio, CHUNK_SECS * SR, 20 * SR, SR / 5);
    let mut out = Vec::new();
    for (k, &(a, b)) in chunks.iter().enumerate() {
        let part = &audio[a..b];
        // Тишину не отправляем.
        if part.iter().fold(0f32, |m, v| m.max(v.abs())) > 0.003 {
            out.extend(send_chunk(config, model, language, prompt, part, a as f64 / SR as f64)?);
        }
        on_progress(k + 1, chunks.len());
    }
    Ok(out)
}

/// Проверка: умеет ли сервер распознавать речь (секунда тишины).
pub fn check(config: &AiConfig, model: &str) -> AppResult<()> {
    send_chunk(config, model, None, None, &vec![0.0; SR], 0.0).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_multipart_with_fields_and_file() {
        let body = multipart("b", &[("model", "whisper-1"), ("language", "ru")], b"RIFF");
        let s = String::from_utf8_lossy(&body);
        assert!(s.starts_with("--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(s.contains("name=\"language\"\r\n\r\nru\r\n"));
        assert!(s.contains("filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF\r\n--b--\r\n"));
    }

    #[test]
    fn parses_segments_or_plain_text_with_offset() {
        let v = serde_json::json!({"text": "все", "segments": [
            {"start": 0.0, "end": 2.5, "text": " Привет "},
            {"start": 2.5, "end": 4.0, "text": "  "}
        ]});
        let s = parse_response(&v, 600.0, 10.0);
        assert_eq!(s, vec![Segment { start_secs: 600.0, end_secs: 602.5, text: "Привет".into() }]);
        let plain = parse_response(&serde_json::json!({"text": "Только текст"}), 10.0, 5.0);
        assert_eq!(plain, vec![Segment { start_secs: 10.0, end_secs: 15.0, text: "Только текст".into() }]);
        assert!(parse_response(&serde_json::json!({}), 0.0, 1.0).is_empty());
    }

    #[test]
    fn wav_in_memory_is_valid() {
        let bytes = wav_bytes(&[0.0, 0.5, -0.5]).unwrap();
        let r = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.len(), 3);
    }

    /// Настоящий HTTP-запрос к локальному «серверу»: заголовки, поля, файл.
    #[test]
    fn sends_real_request_to_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 65536];
            // Читаем до конца тела (по Content-Length).
            loop {
                let n = sock.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(h) = text.find("\r\n\r\n") {
                    let len: usize = text[..h]
                        .lines()
                        .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if buf.len() >= h + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let body = r#"{"segments":[{"start":0.1,"end":0.9,"text":" Проверка "}]}"#;
            write!(sock, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });
        let cfg = AiConfig { base_url: format!("http://127.0.0.1:{port}/v1/"), api_key: "k".into(), model: String::new() };
        let segs = send_chunk(&cfg, "whisper-1", Some("ru"), Some("Jira"), &vec![0.1; 1600], 5.0).unwrap();
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /v1/audio/transcriptions"), "{}", &req[..60]);
        assert!(req.contains("Authorization: Bearer k") || req.contains("authorization: Bearer k"));
        assert!(req.contains("name=\"language\"\r\n\r\nru"));
        assert!(req.contains("name=\"prompt\"\r\n\r\nJira"));
        assert_eq!(segs, vec![Segment { start_secs: 5.1, end_secs: 5.9, text: "Проверка".into() }]);
    }
}
