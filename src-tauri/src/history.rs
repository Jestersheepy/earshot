//! Every dictation (raw and final text, target app, timing) in a small SQLite file, for paste-last and
//! the History view. %APPDATA%\Earshot\history.db

use std::path::PathBuf;

use anyhow::Result;
use chrono::Local;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

pub struct Entry<'a> {
    pub mode: &'a str,
    pub app: Option<&'a str>,
    pub raw: &'a str,
    pub text: &'a str,
    pub llm: bool,
    pub ms: u64,
}

pub struct History {
    conn: Mutex<Connection>,
}

pub fn path() -> PathBuf {
    crate::config::path().with_file_name("history.db")
}

impl History {
    pub fn open() -> Result<Self> {
        Self::open_at(&path())
    }

    pub fn open_at(p: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(p.parent().unwrap())?;
        let conn = Connection::open(p)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS dictations (
                 id INTEGER PRIMARY KEY,
                 ts TEXT NOT NULL,
                 mode TEXT NOT NULL,
                 app TEXT,
                 raw TEXT NOT NULL,
                 text TEXT NOT NULL,
                 llm INTEGER NOT NULL,
                 ms INTEGER NOT NULL
             );",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn add(&self, e: &Entry) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO dictations (ts, mode, app, raw, text, llm, ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(), e.mode, e.app, e.raw, e.text, e.llm, e.ms as i64],
        )?;
        Ok(())
    }

    pub fn last_text(&self) -> Option<String> {
        self.conn
            .lock()
            .query_row("SELECT text FROM dictations WHERE text != '' ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_last() {
        let p = std::env::temp_dir().join(format!("earshot-history-{}.db", std::process::id()));
        let h = History::open_at(&p).unwrap();
        assert_eq!(h.last_text(), None);
        h.add(&Entry { mode: "dictate", app: Some("notepad.exe"), raw: "um hi", text: "Hi", llm: false, ms: 120 }).unwrap();
        h.add(&Entry { mode: "dictate", app: None, raw: "", text: "", llm: false, ms: 5 }).unwrap();
        assert_eq!(h.last_text().as_deref(), Some("Hi"));
        drop(h);
        let _ = std::fs::remove_file(&p);
    }
}
