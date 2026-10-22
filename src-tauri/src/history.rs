//! Every dictation (raw and final text, target app, timing) in a small SQLite file, for paste-last and
//! the History view, plus the voices of people you've named in meetings. %APPDATA%\Earshot\history.db

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

#[derive(serde::Serialize)]
pub struct Row {
    pub id: i64,
    pub ts: String,
    pub mode: String,
    pub app: Option<String>,
    pub raw: String,
    pub text: String,
    pub llm: bool,
    pub ms: i64,
}

pub struct History {
    conn: Mutex<Connection>,
}

#[derive(serde::Serialize, Default)]
pub struct Stats {
    pub today_words: i64,
    pub week_words: i64,
    pub total_words: i64,
    pub today_count: i64,
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
             );
             CREATE TABLE IF NOT EXISTS voices (
                 name TEXT PRIMARY KEY,
                 centroid BLOB NOT NULL,
                 n INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS note_speakers (
                 note TEXT NOT NULL,
                 label TEXT NOT NULL,
                 centroid BLOB NOT NULL,
                 n INTEGER NOT NULL,
                 PRIMARY KEY (note, label)
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

    /// Newest first; `query` matches raw or final text, or the app.
    pub fn list(&self, query: &str, limit: u32) -> Result<Vec<Row>> {
        let like = format!("%{}%", query.trim().replace('%', "\\%").replace('_', "\\_"));
        let conn = self.conn.lock();
        let mut st = conn.prepare(
            "SELECT id, ts, mode, app, raw, text, llm, ms FROM dictations
             WHERE text != '' AND (text LIKE ?1 ESCAPE '\\' OR raw LIKE ?1 ESCAPE '\\' OR app LIKE ?1 ESCAPE '\\')
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![like, limit], |r| {
            Ok(Row {
                id: r.get(0)?,
                ts: r.get(1)?,
                mode: r.get(2)?,
                app: r.get(3)?,
                raw: r.get(4)?,
                text: r.get(5)?,
                llm: r.get(6)?,
                ms: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.lock().execute("DELETE FROM dictations WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Named voices and their mean embeddings.
    pub fn voices(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare("SELECT name, centroid FROM voices ORDER BY name")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, floats(&r.get::<_, Vec<u8>>(1)?))))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Named voices with how many utterances they were learned from.
    pub fn voice_names(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare("SELECT name, n FROM voices ORDER BY name")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn forget_voice(&self, name: &str) -> Result<()> {
        self.conn.lock().execute("DELETE FROM voices WHERE name = ?1", [name])?;
        Ok(())
    }

    /// The voices heard in a meeting note, by transcript label.
    pub fn save_note_speakers(&self, note: &str, speakers: &[(String, Vec<f32>, usize)]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM note_speakers WHERE note = ?1", [note])?;
        for (label, c, n) in speakers {
            tx.execute(
                "INSERT INTO note_speakers (note, label, centroid, n) VALUES (?1, ?2, ?3, ?4)",
                params![note, label, bytes(c), *n as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// `from` in `note` is `to`: relabel it there and teach the voice to `to` (unless `to` is you). Notes from
    /// before speaker detection have no stored voice; then only the label changes.
    pub fn name_speaker(&self, note: &str, from: &str, to: &str, learn: bool) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let heard: Option<(Vec<u8>, i64)> = tx
            .query_row("SELECT centroid, n FROM note_speakers WHERE note = ?1 AND label = ?2", [note, from], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((c, n)) = heard {
            let c = floats(&c);
            // merge into an existing row for `to` in this note, else rename
            let existing: Option<(Vec<u8>, i64)> = tx
                .query_row("SELECT centroid, n FROM note_speakers WHERE note = ?1 AND label = ?2", [note, to], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            let (merged, total) = match existing {
                Some((e, m)) => (weighted(&floats(&e), m, &c, n), m + n),
                None => (c.clone(), n),
            };
            tx.execute("DELETE FROM note_speakers WHERE note = ?1 AND label IN (?2, ?3)", [note, from, to])?;
            tx.execute(
                "INSERT INTO note_speakers (note, label, centroid, n) VALUES (?1, ?2, ?3, ?4)",
                params![note, to, bytes(&merged), total],
            )?;
            if learn {
                let known: Option<(Vec<u8>, i64)> = tx
                    .query_row("SELECT centroid, n FROM voices WHERE name = ?1", [to], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let (v, total) = match known {
                    Some((k, m)) => (weighted(&floats(&k), m, &c, n), m + n),
                    None => (c, n),
                };
                tx.execute(
                    "INSERT INTO voices (name, centroid, n) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name) DO UPDATE SET centroid = excluded.centroid, n = excluded.n",
                    params![to, bytes(&v), total],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Words dictated today, in the last 7 days and ever, and today's dictation count.
    pub fn stats(&self) -> Result<Stats> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let week = (Local::now() - chrono::Duration::days(6)).format("%Y-%m-%d").to_string();
        const WORDS: &str = "COALESCE(SUM(CASE WHEN trim(text) = '' THEN 0 ELSE length(trim(text)) - length(replace(trim(text), ' ', '')) + 1 END), 0)";
        let c = self.conn.lock();
        let words = |since: &str| -> Result<i64> {
            Ok(c.query_row(&format!("SELECT {WORDS} FROM dictations WHERE ts >= ?1"), [since], |r| r.get(0))?)
        };
        Ok(Stats {
            today_words: words(&today)?,
            week_words: words(&week)?,
            total_words: words("")?,
            today_count: c.query_row("SELECT COUNT(*) FROM dictations WHERE ts >= ?1", [&today], |r| r.get(0))?,
        })
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

fn bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn floats(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Unit-length weighted mean of two centroids.
fn weighted(a: &[f32], na: i64, b: &[f32], nb: i64) -> Vec<f32> {
    earshot_core::speakers::normalized(a.iter().zip(b).map(|(x, y)| x * na as f32 + y * nb as f32).collect())
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
        let rows = h.list("notepad", 10).unwrap();
        assert_eq!(rows.len(), 1);
        h.delete(rows[0].id).unwrap();
        assert!(h.list("", 10).unwrap().is_empty());

        h.save_note_speakers("a.md", &[("Speaker 1".into(), vec![1.0, 0.0], 4), ("Speaker 2".into(), vec![0.0, 1.0], 2)]).unwrap();
        h.name_speaker("a.md", "Speaker 2", "Sarah", true).unwrap();
        h.name_speaker("a.md", "Speaker 1", "Sarah", true).unwrap();
        let v = h.voices().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(h.voice_names().unwrap(), vec![("Sarah".to_string(), 6)]);
        assert!(v[0].1[0] > v[0].1[1], "weighted toward the voice heard more");
        h.forget_voice("Sarah").unwrap();
        assert!(h.voices().unwrap().is_empty());
        drop(h);
        let _ = std::fs::remove_file(&p);
    }
}
