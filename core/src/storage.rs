use crate::error::{AppError, AppResult};
use crate::model::{Collection, Meeting};
use rusqlite::{params, Connection};

/// Хранилище встреч поверх SQLite.
pub struct Repo {
    conn: Connection,
}

impl Repo {
    /// Открывает БД по пути к файлу и создаёт схему при необходимости.
    pub fn open(db_path: &std::path::Path) -> AppResult<Self> {
        let conn = Connection::open(db_path)?;
        Self::init(conn)
    }

    /// БД в памяти — для тестов.
    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                title TEXT NOT NULL,
                participants TEXT NOT NULL,
                topic TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                folder TEXT NOT NULL,
                status TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'recorded',
                notes TEXT NOT NULL DEFAULT '',
                collection TEXT NOT NULL DEFAULT ''
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS collections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Идемпотентные миграции для БД, созданных прежними версиями.
    fn migrate(conn: &Connection) -> AppResult<()> {
        if !Self::column_exists(conn, "source")? {
            conn.execute(
                "ALTER TABLE meetings ADD COLUMN source TEXT NOT NULL DEFAULT 'recorded'",
                [],
            )?;
        }
        if !Self::column_exists(conn, "notes")? {
            conn.execute(
                "ALTER TABLE meetings ADD COLUMN notes TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !Self::column_exists(conn, "collection")? {
            conn.execute(
                "ALTER TABLE meetings ADD COLUMN collection TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        Ok(())
    }

    /// Есть ли столбец `column` в таблице meetings (через PRAGMA table_info).
    fn column_exists(conn: &Connection, column: &str) -> AppResult<bool> {
        let mut stmt = conn.prepare("PRAGMA table_info(meetings)")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn insert(&self, m: &Meeting) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO meetings
                (id, created_at, title, participants, topic, duration_secs, folder, status, source, notes, collection)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                m.id,
                m.created_at,
                m.title,
                m.participants,
                m.topic,
                m.duration_secs,
                m.folder,
                m.status,
                m.source,
                m.notes,
                m.collection
            ],
        )?;
        Ok(())
    }

    /// Все встречи, новейшие сверху.
    pub fn list(&self) -> AppResult<Vec<Meeting>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, title, participants, topic, duration_secs, folder, status, source, notes, collection
             FROM meetings ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_meeting)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> AppResult<Meeting> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, title, participants, topic, duration_secs, folder, status, source, notes, collection
             FROM meetings WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::row_to_meeting)?;
        match rows.next() {
            Some(r) => Ok(r?),
            None => Err(AppError::NotFound(id.to_string())),
        }
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        let n = self
            .conn
            .execute("DELETE FROM meetings WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Меняет статус встречи (например, на "transcribed").
    pub fn update_status(&self, id: &str, status: &str) -> AppResult<()> {
        let n = self.conn.execute(
            "UPDATE meetings SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Обновляет длительность встречи (после правки аудио — вырезания
    /// фрагментов или возврата к оригиналу).
    pub fn update_duration(&self, id: &str, duration_secs: u64) -> AppResult<()> {
        let n = self.conn.execute(
            "UPDATE meetings SET duration_secs = ?1 WHERE id = ?2",
            params![duration_secs, id],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Обновляет заголовок/участников/тему встречи.
    pub fn update_meta(
        &self,
        id: &str,
        title: &str,
        participants: &str,
        topic: &str,
    ) -> AppResult<()> {
        let n = self.conn.execute(
            "UPDATE meetings SET title = ?1, participants = ?2, topic = ?3 WHERE id = ?4",
            params![title, participants, topic, id],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Сохраняет заметки пользователя к встрече.
    pub fn update_notes(&self, id: &str, notes: &str) -> AppResult<()> {
        let n = self.conn.execute(
            "UPDATE meetings SET notes = ?1 WHERE id = ?2",
            params![notes, id],
        )?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn row_to_meeting(row: &rusqlite::Row) -> rusqlite::Result<Meeting> {
        Ok(Meeting {
            id: row.get(0)?,
            created_at: row.get(1)?,
            title: row.get(2)?,
            participants: row.get(3)?,
            topic: row.get(4)?,
            duration_secs: u64::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
            folder: row.get(6)?,
            status: row.get(7)?,
            source: row.get(8)?,
            notes: row.get(9)?,
            collection: row.get(10)?,
        })
    }

    // ── Папки списка встреч ────────────────────────────────────────────────

    /// Все папки, по имени.
    pub fn list_collections(&self) -> AppResult<Vec<Collection>> {
        let mut stmt = self.conn.prepare("SELECT id, name, created_at FROM collections ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |r| Ok(Collection { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)? }))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn insert_collection(&self, c: &Collection) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO collections (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![c.id, c.name, c.created_at],
        )?;
        Ok(())
    }

    pub fn rename_collection(&self, id: &str, name: &str) -> AppResult<()> {
        let n = self.conn.execute("UPDATE collections SET name = ?1 WHERE id = ?2", params![name, id])?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Удаляет папку; встречи из неё остаются (переходят «вне папок»).
    pub fn delete_collection(&self, id: &str) -> AppResult<()> {
        self.conn.execute("UPDATE meetings SET collection = '' WHERE collection = ?1", params![id])?;
        let n = self.conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Кладёт встречу в папку (`""` — вне папок).
    pub fn set_meeting_collection(&self, id: &str, collection: &str) -> AppResult<()> {
        let n = self.conn.execute("UPDATE meetings SET collection = ?1 WHERE id = ?2", params![collection, id])?;
        if n == 0 {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, created_at: &str) -> Meeting {
        Meeting {
            id: id.into(),
            created_at: created_at.into(),
            title: "t".into(),
            participants: "p".into(),
            topic: "x".into(),
            duration_secs: 10,
            folder: id.into(),
            status: "recorded".into(),
            source: "recorded".into(),
            notes: String::new(),
            collection: String::new(),
        }
    }

    #[test]
    fn insert_then_get_returns_same() {
        let repo = Repo::open_in_memory().unwrap();
        let m = sample("a", "2026-06-04T10:00:00Z");
        repo.insert(&m).unwrap();
        assert_eq!(repo.get("a").unwrap(), m);
    }

    #[test]
    fn list_orders_newest_first() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("old", "2026-06-01T10:00:00Z")).unwrap();
        repo.insert(&sample("new", "2026-06-04T10:00:00Z")).unwrap();
        let ids: Vec<String> = repo.list().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["new", "old"]);
    }

    #[test]
    fn get_missing_is_not_found() {
        let repo = Repo::open_in_memory().unwrap();
        assert!(matches!(repo.get("nope"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn delete_removes_row() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("a", "2026-06-04T10:00:00Z")).unwrap();
        repo.delete("a").unwrap();
        assert!(repo.list().unwrap().is_empty());
    }

    #[test]
    fn update_status_changes_status() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("a", "2026-06-04T10:00:00Z")).unwrap();
        repo.update_status("a", "transcribed").unwrap();
        assert_eq!(repo.get("a").unwrap().status, "transcribed");
    }

    #[test]
    fn update_status_missing_is_not_found() {
        let repo = Repo::open_in_memory().unwrap();
        assert!(matches!(repo.update_status("x", "t"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn update_duration_changes_duration() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("a", "2026-06-04T10:00:00Z")).unwrap();
        repo.update_duration("a", 7).unwrap();
        assert_eq!(repo.get("a").unwrap().duration_secs, 7);
        assert!(matches!(
            repo.update_duration("nope", 1),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn update_meta_changes_fields() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("a", "2026-06-04T10:00:00Z")).unwrap();
        repo.update_meta("a", "Заголовок", "Иван", "Тема").unwrap();
        let m = repo.get("a").unwrap();
        assert_eq!((m.title, m.participants, m.topic), ("Заголовок".into(), "Иван".into(), "Тема".into()));
    }

    #[test]
    fn notes_roundtrip_and_update() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("a", "2026-06-04T10:00:00Z")).unwrap();
        assert_eq!(repo.get("a").unwrap().notes, "");
        repo.update_notes("a", "перезвонить Олегу\nбюджет Q3").unwrap();
        assert_eq!(repo.get("a").unwrap().notes, "перезвонить Олегу\nбюджет Q3");
        assert!(repo.update_notes("missing", "x").is_err());
    }

    #[test]
    fn migrates_old_schema_without_source_column() {
        // Эмулируем БД прежней версии: таблица без столбца `source`.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE meetings (
                id TEXT PRIMARY KEY, created_at TEXT NOT NULL, title TEXT NOT NULL,
                participants TEXT NOT NULL, topic TEXT NOT NULL, duration_secs INTEGER NOT NULL,
                folder TEXT NOT NULL, status TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meetings VALUES ('a','2026-06-04T10:00:00Z','t','p','x',10,'a','recorded')",
            [],
        )
        .unwrap();

        // init() должен добавить столбец и не потерять данные.
        let repo = Repo::init(conn).unwrap();
        let m = repo.get("a").unwrap();
        assert_eq!(m.source, "recorded");
        assert_eq!(m.notes, "");
        assert_eq!(m.title, "t");
    }

    #[test]
    fn collections_hold_meetings_and_release_them_on_delete() {
        let repo = Repo::open_in_memory().unwrap();
        repo.insert(&sample("m1", "2026-10-01T10:00:00Z")).unwrap();
        repo.insert(&sample("m2", "2026-10-02T10:00:00Z")).unwrap();
        let c = Collection { id: "c1".into(), name: "Проект".into(), created_at: "2026-10-03T00:00:00Z".into() };
        repo.insert_collection(&c).unwrap();
        repo.set_meeting_collection("m1", "c1").unwrap();
        assert_eq!(repo.get("m1").unwrap().collection, "c1");
        repo.rename_collection("c1", "Клиент").unwrap();
        assert_eq!(repo.list_collections().unwrap()[0].name, "Клиент");
        // Удаление папки встречи не удаляет — они выходят из неё.
        repo.delete_collection("c1").unwrap();
        assert!(repo.list_collections().unwrap().is_empty());
        assert_eq!(repo.get("m1").unwrap().collection, "");
        assert_eq!(repo.list().unwrap().len(), 2);
    }
}
