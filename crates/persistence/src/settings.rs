use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::Serialize;
use velox_types::AppSettings;

use crate::{Database, DbResult};

const APP_SETTINGS_KEY: &str = "app";

impl Database {
    /// Store an arbitrary JSON value under `key`.
    pub fn put_setting<T: Serialize>(&self, key: &str, value: &T) -> DbResult<()> {
        let json = serde_json::to_string(value)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, &json],
            )?;
            Ok(())
        })
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> DbResult<Option<T>> {
        let raw: Option<String> = self.with(|c| {
            Ok(
                c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })?;
        match raw {
            None => Ok(None),
            Some(s) => match serde_json::from_str(&s) {
                Ok(v) => Ok(Some(v)),
                Err(e) => {
                    tracing::warn!(key, error = %e, "ignoring unreadable setting");
                    Ok(None)
                }
            },
        }
    }

    /// Load application settings (defaults when missing), normalized.
    pub fn load_settings(&self) -> DbResult<AppSettings> {
        Ok(self
            .get_setting::<AppSettings>(APP_SETTINGS_KEY)?
            .unwrap_or_default()
            .normalized())
    }

    pub fn save_settings(&self, s: &AppSettings) -> DbResult<()> {
        self.put_setting(APP_SETTINGS_KEY, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(
            db.load_settings().unwrap(),
            AppSettings::default().normalized()
        );
        let mut s = AppSettings::default();
        s.downloads.max_concurrent = 7;
        s.appearance.language = "ar".into();
        db.save_settings(&s).unwrap();
        let loaded = db.load_settings().unwrap();
        assert_eq!(loaded.downloads.max_concurrent, 7);
        assert_eq!(loaded.appearance.language, "ar");
        db.put_setting("x", &vec![1, 2, 3]).unwrap();
        assert_eq!(
            db.get_setting::<Vec<i32>>("x").unwrap(),
            Some(vec![1, 2, 3])
        );
    }
}
