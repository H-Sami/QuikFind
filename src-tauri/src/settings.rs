use crate::error::{QuikFindError, Result};
use crate::models::{AppSettings, HistoryItem};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;
use tracing::info;

pub struct SettingsDatabase {
    conn: Mutex<Connection>,
}

impl SettingsDatabase {
    /// Opens or creates the settings database at the given config directory.
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened or tables cannot be initialized.
    pub fn open(config_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir).map_err(QuikFindError::Io)?;
        let db_path = config_dir.join("quikfind.db");
        info!("Opening settings database at {:?}", db_path);

        let conn = Connection::open(&db_path).map_err(QuikFindError::Database)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.initialize_tables()?;
        Ok(db)
    }

    fn initialize_tables(&self) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS history (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'File',
                opened_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_history_opened_at
                ON history(opened_at DESC);

            CREATE TABLE IF NOT EXISTS indexed_files (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL DEFAULT 0,
                modified INTEGER NOT NULL DEFAULT 0,
                doc_id TEXT NOT NULL,
                last_indexed INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_indexed_files_doc_id
                ON indexed_files(doc_id);

            CREATE TABLE IF NOT EXISTS cached_apps (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                icon BLOB,
                last_scanned INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_cached_apps_name
                ON cached_apps(name);
            ",
        )
        .map_err(QuikFindError::Database)?;

        info!("Database tables initialized");
        Ok(())
    }

    /// Loads settings from the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn load_settings(&self) -> Result<AppSettings> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT key, value FROM settings")
            .map_err(QuikFindError::Database)?;

        let rows = stmt
            .query_map([], |row| {
                let key: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok((key, value))
            })
            .map_err(QuikFindError::Database)?;

        let mut settings = AppSettings::default();

        for row in rows {
            let (key, value) = row.map_err(QuikFindError::Database)?;
            match key.as_str() {
                "indexed_paths" => {
                    settings.indexed_paths =
                        serde_json::from_str(&value).unwrap_or_default();
                }
                "excluded_patterns" => {
                    settings.excluded_patterns =
                        serde_json::from_str(&value).unwrap_or_default();
                }
                "max_results" => {
                    settings.max_results = value.parse().unwrap_or(25);
                }
                "hotkey" => {
                    settings.hotkey = value;
                }
                "theme" => {
                    settings.theme = value;
                }
                "enable_content_search" => {
                    settings.enable_content_search = value == "true";
                }
                "indexing_interval_minutes" => {
                    settings.indexing_interval_minutes = value.parse().unwrap_or(30);
                }
                "fuzzy_threshold" => {
                    settings.fuzzy_threshold = value.parse().unwrap_or(0.6);
                }
                "has_completed_onboarding" => {
                    settings.has_completed_onboarding = value == "true";
                }
                "launch_on_startup" => {
                    settings.launch_on_startup = value == "true";
                }
                _ => {}
            }
        }

        Ok(settings)
    }

    /// Saves settings to the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let conn = self.conn.lock();

        let pairs: Vec<(&str, String)> = vec![
            (
                "indexed_paths",
                serde_json::to_string(&settings.indexed_paths)
                    .map_err(QuikFindError::Serialization)?,
            ),
            (
                "excluded_patterns",
                serde_json::to_string(&settings.excluded_patterns)
                    .map_err(QuikFindError::Serialization)?,
            ),
            ("max_results", settings.max_results.to_string()),
            ("hotkey", settings.hotkey.clone()),
            ("theme", settings.theme.clone()),
            (
                "enable_content_search",
                settings.enable_content_search.to_string(),
            ),
            (
                "indexing_interval_minutes",
                settings.indexing_interval_minutes.to_string(),
            ),
            ("fuzzy_threshold", settings.fuzzy_threshold.to_string()),
            (
                "has_completed_onboarding",
                settings.has_completed_onboarding.to_string(),
            ),
            (
                "launch_on_startup",
                settings.launch_on_startup.to_string(),
            ),
        ];

        for (key, value) in &pairs {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(QuikFindError::Database)?;
        }

        info!("Settings saved to database");
        Ok(())
    }

    /// Adds an item to the history.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn add_history(&self, item: &HistoryItem) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "INSERT INTO history (id, path, name, kind, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET opened_at = excluded.opened_at",
            params![item.id, item.path, item.name, item.kind, item.opened_at],
        )
        .map_err(QuikFindError::Database)?;

        Ok(())
    }

    /// Gets history items from the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn get_history(&self, limit: u32) -> Result<Vec<HistoryItem>> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, kind, opened_at
                 FROM history ORDER BY opened_at DESC LIMIT ?1",
            )
            .map_err(QuikFindError::Database)?;

        let items = stmt
            .query_map(params![limit], |row| {
                Ok(HistoryItem {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    opened_at: row.get(4)?,
                })
            })
            .map_err(QuikFindError::Database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(QuikFindError::Database)?;

        Ok(items)
    }

    /// Records an indexed file in the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn record_indexed_file(
        &self,
        path: &str,
        size: u64,
        modified: i64,
        doc_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock();

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO indexed_files (path, size, modified, doc_id, last_indexed)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size,
                modified = excluded.modified,
                doc_id = excluded.doc_id,
                last_indexed = excluded.last_indexed",
            params![path, size, modified, doc_id, now],
        )
        .map_err(QuikFindError::Database)?;

        Ok(())
    }

    /// Removes an indexed file from the database and returns its doc ID.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn remove_indexed_file(&self, path: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();

        let doc_id: Option<String> = conn
            .query_row(
                "SELECT doc_id FROM indexed_files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok();

        conn.execute(
            "DELETE FROM indexed_files WHERE path = ?1",
            params![path],
        )
        .map_err(QuikFindError::Database)?;

        Ok(doc_id)
    }

    /// Returns the number of indexed files.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn get_indexed_file_count(&self) -> Result<u64> {
        let conn = self.conn.lock();

        let count: u64 = conn
            .query_row("SELECT COUNT(*) FROM indexed_files", [], |row| row.get(0))
            .map_err(QuikFindError::Database)?;

        Ok(count)
    }

    /// Gets the saved window state.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn get_window_state(&self) -> Result<Option<String>> {
        let conn = self.conn.lock();

        let result: std::result::Result<String, _> = conn.query_row(
            "SELECT value FROM settings WHERE key = 'window_state'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(QuikFindError::Database(e)),
        }
    }

    /// Saves the window state.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn save_window_state(&self, json: &str) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('window_state', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![json],
        )
        .map_err(QuikFindError::Database)?;

        Ok(())
    }

    /// Clears all indexed file records from the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn clear_indexed_files(&self) -> Result<()> {
        let conn = self.conn.lock();

        conn.execute("DELETE FROM indexed_files", [])
            .map_err(QuikFindError::Database)?;

        Ok(())
    }

    /// Caches an app entry in the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be written.
    pub fn cache_app(&self, id: &str, name: &str, path: &str, _icon: Option<&[u8]>) -> Result<()> {
        let conn = self.conn.lock();

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO cached_apps (id, name, path, last_scanned)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                path = excluded.path,
                last_scanned = excluded.last_scanned",
            params![id, name, path, now],
        )
        .map_err(QuikFindError::Database)?;

        Ok(())
    }

    /// Returns all cached apps from the database.
    ///
    /// # Errors
    /// Returns an error if the database cannot be read.
    pub fn get_cached_apps(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT id, name, path FROM cached_apps ORDER BY name")
            .map_err(QuikFindError::Database)?;

        let apps = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(QuikFindError::Database)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(QuikFindError::Database)?;

        Ok(apps)
    }
}
