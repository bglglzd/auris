use serde::{Deserialize, Serialize};

/// Дорожка записи: микрофон (это «Я») или системный звук (это «Собеседник»).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingTrack {
    Mic,
    System,
}

/// Одна записанная встреча.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meeting {
    pub id: String,
    /// ISO-8601, UTC.
    pub created_at: String,
    pub title: String,
    pub participants: String,
    pub topic: String,
    pub duration_secs: u64,
    /// Имя папки встречи внутри каталога данных.
    pub folder: String,
    /// recorded | transcribed | summarized (в Плане 1 всегда "recorded").
    pub status: String,
    /// Источник встречи: "recorded" (записана приложением, 2 дорожки) или
    /// "imported" (импортированный файл, одна дорожка `audio.wav`).
    #[serde(default = "default_source")]
    pub source: String,
    /// Заметки пользователя к встрече (свободный текст). Пусто — нет заметок.
    #[serde(default)]
    pub notes: String,
    /// Папка в списке встреч (id из [`Collection`]); пусто — вне папок.
    #[serde(default)]
    pub collection: String,
}

/// Папка списка встреч: пользователь собирает в неё встречи по теме.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    /// ISO-8601, UTC.
    pub created_at: String,
}

/// Дефолт для `source` — на случай старых записей без этого поля.
fn default_source() -> String {
    "recorded".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_round_trips_through_json() {
        let m = Meeting {
            id: "abc".into(),
            created_at: "2026-06-04T10:00:00Z".into(),
            title: "Звонок".into(),
            participants: "Иван".into(),
            topic: "Планы".into(),
            duration_secs: 42,
            folder: "abc".into(),
            status: "recorded".into(),
            source: "recorded".into(),
            notes: "".into(),
            collection: String::new(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: Meeting = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn track_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&RecordingTrack::Mic).unwrap(), "\"mic\"");
        assert_eq!(
            serde_json::to_string(&RecordingTrack::System).unwrap(),
            "\"system\""
        );
    }
}
