//! Search by meaning over the Markdown notes. Notes are cut into passages (transcript windows, summary
//! sections, paragraphs), embedded with bge-small, and kept in their own SQLite file. A query ranks passages
//! by embedding similarity and by keyword match, and merges the two rankings with reciprocal rank fusion.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use earshot_core::embed::{QUERY_PREFIX, TextEmbedder};
use parking_lot::Mutex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// Transcript lines are grouped until a window spans this many seconds.
const WINDOW_S: u32 = 40;
/// Longest passage, in words; longer text is split.
const MAX_WORDS: usize = 120;
const MAX_DEPTH: usize = 3;
const IDLE: Duration = Duration::from_secs(300);
const RRF_K: f32 = 60.0;
const PER_NOTE: usize = 2;
const POOL: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    pub t: Option<u32>,
    pub speakers: Vec<String>,
    pub text: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Meta {
    pub date: Option<NaiveDate>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    pub person: Option<String>,
    pub after: Option<NaiveDate>,
    pub before: Option<NaiveDate>,
    /// `meeting` or `voice-note`.
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub t: Option<u32>,
    pub speakers: Vec<String>,
    pub text: String,
    pub score: f32,
}

// ---- chunking ----

/// Splits a note into its front matter fields and passages.
pub fn chunk(md: &str) -> (Meta, Vec<Passage>) {
    let mut meta = Meta::default();
    let mut lines = md.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim_end() == "---" {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                match k.trim() {
                    "date" => meta.date = v.get(..10).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
                    "type" => meta.kind = Some(v.to_string()),
                    _ => {}
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut heading = String::new();
    let mut block: Vec<&str> = Vec::new();
    // The open transcript window: start, speakers, text.
    let mut win: Option<(u32, Vec<String>, String)> = None;

    fn flush_block(out: &mut Vec<Passage>, block: &mut Vec<&str>, heading: &str) {
        let body = block.drain(..).collect::<Vec<_>>().join("\n");
        let body = body.trim();
        if body.is_empty() {
            return;
        }
        let text = if heading.is_empty() { body.to_string() } else { format!("{heading}: {body}") };
        for text in split_words(&text) {
            out.push(Passage { t: None, speakers: Vec::new(), text });
        }
    }
    fn flush_win(out: &mut Vec<Passage>, win: &mut Option<(u32, Vec<String>, String)>) {
        if let Some((t, speakers, text)) = win.take() {
            for text in split_words(&text) {
                out.push(Passage { t: Some(t), speakers: speakers.clone(), text });
            }
        }
    }

    for line in lines {
        if let Some((t, speaker, text)) = transcript_line(line) {
            flush_block(&mut out, &mut block, &heading);
            let full = if speaker.is_empty() { text.to_string() } else { format!("{speaker}: {text}") };
            match &mut win {
                Some((start, speakers, buf)) if t.saturating_sub(*start) < WINDOW_S && buf.split_whitespace().count() < MAX_WORDS => {
                    if !speaker.is_empty() && !speakers.iter().any(|s| s == speaker) {
                        speakers.push(speaker.to_string());
                    }
                    buf.push(' ');
                    buf.push_str(&full);
                }
                _ => {
                    flush_win(&mut out, &mut win);
                    let speakers = if speaker.is_empty() { Vec::new() } else { vec![speaker.to_string()] };
                    win = Some((t, speakers, full));
                }
            }
        } else if let Some(h) = line.strip_prefix('#') {
            flush_block(&mut out, &mut block, &heading);
            flush_win(&mut out, &mut win);
            heading = h.trim_start_matches('#').trim().to_string();
        } else if line.trim().is_empty() {
            flush_block(&mut out, &mut block, &heading);
        } else {
            flush_win(&mut out, &mut win);
            block.push(line);
        }
    }
    flush_block(&mut out, &mut block, &heading);
    flush_win(&mut out, &mut win);
    (meta, out)
}

/// `**[mm:ss] Speaker:** text` (or `[h:mm:ss]`) into (seconds, speaker, text).
fn transcript_line(line: &str) -> Option<(u32, &str, &str)> {
    let rest = line.trim_start().strip_prefix("**[")?;
    let (stamp, rest) = rest.split_once(']')?;
    let mut t = 0u32;
    for part in stamp.split(':') {
        t = t.checked_mul(60)?.checked_add(part.trim().parse().ok()?)?;
    }
    let (speaker, text) = rest.split_once(":**")?;
    Some((t, speaker.trim(), text.trim()))
}

fn split_words(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.chunks(MAX_WORDS).map(|c| c.join(" ")).collect()
}

// ---- ranking ----

/// Reciprocal rank fusion of ranked lists of ids (best first).
pub fn rrf(lists: &[Vec<usize>]) -> Vec<(usize, f32)> {
    let mut score: HashMap<usize, f32> = HashMap::new();
    for list in lists {
        for (rank, &id) in list.iter().enumerate() {
            *score.entry(id).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut v: Vec<(usize, f32)> = score.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "is", "it", "that", "this", "with", "was", "are", "be", "we", "i", "you", "at", "as", "by",
];

fn terms(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

/// BM25-ish keyword score of each document for the query terms; IDF comes from the documents themselves.
pub fn keyword_scores(query: &str, docs: &[&str]) -> Vec<f32> {
    let q: HashSet<String> = terms(query).into_iter().collect();
    let tokenised: Vec<Vec<String>> = docs.iter().map(|d| terms(d)).collect();
    let n = docs.len() as f32;
    let avg = (tokenised.iter().map(Vec::len).sum::<usize>() as f32 / n.max(1.0)).max(1.0);
    let mut df: HashMap<&str, f32> = HashMap::new();
    for d in &tokenised {
        for t in d.iter().map(String::as_str).collect::<HashSet<_>>() {
            if q.contains(t) {
                *df.entry(t).or_default() += 1.0;
            }
        }
    }
    tokenised
        .iter()
        .map(|d| {
            let mut tf: HashMap<&str, f32> = HashMap::new();
            for t in d {
                if q.contains(t.as_str()) {
                    *tf.entry(t.as_str()).or_default() += 1.0;
                }
            }
            tf.iter()
                .map(|(t, &f)| {
                    let idf = ((n - df[t] + 0.5) / (df[t] + 0.5) + 1.0).ln();
                    idf * f * 2.2 / (f + 1.2 * (0.25 + 0.75 * d.len() as f32 / avg))
                })
                .sum()
        })
        .collect()
}

// ---- index ----

pub struct Index {
    db: Connection,
}

struct Row {
    note: String,
    t: Option<u32>,
    speakers: Vec<String>,
    text: String,
    vec: Vec<f32>,
    meta: Meta,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let db = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS notes (path TEXT PRIMARY KEY, mtime INTEGER NOT NULL, date TEXT, kind TEXT);
             CREATE TABLE IF NOT EXISTS passages (
                 id INTEGER PRIMARY KEY, note TEXT NOT NULL REFERENCES notes(path) ON DELETE CASCADE,
                 t INTEGER, speakers TEXT NOT NULL, text TEXT NOT NULL, vec BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS passages_note ON passages(note);",
        )?;
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { db })
    }

    pub fn refresh(&self, notes_dir: &Path, embedder: &mut TextEmbedder) -> Result<()> {
        self.refresh_with(notes_dir, &mut |t| embedder.embed(t))
    }

    pub fn refresh_with(&self, notes_dir: &Path, embed: &mut dyn FnMut(&[&str]) -> Result<Vec<Vec<f32>>>) -> Result<()> {
        let mut files = Vec::new();
        walk(notes_dir, 0, &mut files);
        let known: HashMap<String, i64> = {
            let mut st = self.db.prepare("SELECT path, mtime FROM notes")?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let mut seen = HashSet::new();
        for (path, mtime) in files {
            let key = path.to_string_lossy().into_owned();
            seen.insert(key.clone());
            if known.get(&key) == Some(&mtime) {
                continue;
            }
            let Ok(md) = std::fs::read_to_string(&path) else { continue };
            let (meta, passages) = chunk(&md);
            let mut vecs = Vec::with_capacity(passages.len());
            for batch in passages.chunks(16) {
                let texts: Vec<&str> = batch.iter().map(|p| p.text.as_str()).collect();
                vecs.extend(embed(&texts)?);
            }
            let tx = self.db.unchecked_transaction()?;
            tx.execute("DELETE FROM passages WHERE note = ?1", [&key])?;
            tx.execute(
                "INSERT OR REPLACE INTO notes (path, mtime, date, kind) VALUES (?1, ?2, ?3, ?4)",
                params![key, mtime, meta.date.map(|d| d.to_string()), meta.kind],
            )?;
            for (p, v) in passages.iter().zip(&vecs) {
                let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                tx.execute(
                    "INSERT INTO passages (note, t, speakers, text, vec) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![key, p.t, serde_json::to_string(&p.speakers)?, p.text, blob],
                )?;
            }
            tx.commit()?;
        }
        for gone in known.keys().filter(|k| !seen.contains(*k)) {
            self.db.execute("DELETE FROM passages WHERE note = ?1", [gone])?;
            self.db.execute("DELETE FROM notes WHERE path = ?1", [gone])?;
        }
        Ok(())
    }

    pub fn search(&self, query: &str, embedder: &mut TextEmbedder, filters: &Filters, limit: usize) -> Result<Vec<Hit>> {
        self.search_with(query, &mut |t| embedder.embed(t), filters, limit)
    }

    pub fn search_with(
        &self,
        query: &str,
        embed: &mut dyn FnMut(&[&str]) -> Result<Vec<Vec<f32>>>,
        filters: &Filters,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let qv = embed(&[&format!("{QUERY_PREFIX}{}", query.trim())])?.pop().context("no query embedding")?;

        let mut st = self.db.prepare(
            "SELECT p.note, p.t, p.speakers, p.text, p.vec, n.date, n.kind FROM passages p JOIN notes n ON n.path = p.note",
        )?;
        let rows: Vec<Row> = st
            .query_map([], |r| {
                let blob: Vec<u8> = r.get(4)?;
                let speakers: String = r.get(2)?;
                let date: Option<String> = r.get(5)?;
                Ok(Row {
                    note: r.get(0)?,
                    t: r.get(1)?,
                    speakers: serde_json::from_str(&speakers).unwrap_or_default(),
                    text: r.get(3)?,
                    vec: blob.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                    meta: Meta { date: date.and_then(|d| d.parse().ok()), kind: r.get(6)? },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let rows: Vec<Row> = rows.into_iter().filter(|r| passes(r, filters)).collect();
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let sim: Vec<f32> = rows.iter().map(|r| earshot_core::speakers::dot(&qv, &r.vec)).collect();
        let mut semantic: Vec<usize> = (0..rows.len()).collect();
        semantic.sort_by(|&a, &b| sim[b].total_cmp(&sim[a]));
        semantic.truncate(POOL);

        let kw = keyword_scores(query, &rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
        let mut keyword: Vec<usize> = (0..rows.len()).filter(|&i| kw[i] > 0.0).collect();
        keyword.sort_by(|&a, &b| kw[b].total_cmp(&kw[a]));
        keyword.truncate(POOL);

        let mut per_note: HashMap<&str, usize> = HashMap::new();
        let mut hits = Vec::new();
        for (i, score) in rrf(&[semantic, keyword]) {
            let r = &rows[i];
            let n = per_note.entry(r.note.as_str()).or_default();
            if *n >= PER_NOTE {
                continue;
            }
            *n += 1;
            hits.push(Hit { path: r.note.clone(), t: r.t, speakers: r.speakers.clone(), text: r.text.clone(), score });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }
}

fn passes(r: &Row, f: &Filters) -> bool {
    if let Some(k) = &f.kind
        && r.meta.kind.as_deref() != Some(k.as_str())
    {
        return false;
    }
    if f.after.is_some() || f.before.is_some() {
        let Some(d) = r.meta.date else { return false };
        if f.after.is_some_and(|a| d < a) || f.before.is_some_and(|b| d > b) {
            return false;
        }
    }
    if let Some(p) = f.person.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        let p = p.to_lowercase();
        let hit = if r.speakers.is_empty() {
            r.text.to_lowercase().contains(&p)
        } else {
            r.speakers.iter().any(|s| s.to_lowercase() == p)
        };
        if !hit {
            return false;
        }
    }
    true
}

/// Markdown files up to `MAX_DEPTH` folders down, with their mtime in milliseconds.
fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, i64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(&p, depth + 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")) {
            let ms = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            out.push((p, ms));
        }
    }
}

// ---- searcher ----

struct Shared {
    model_dir: PathBuf,
    threads: usize,
    index: Mutex<Index>,
    embedder: Mutex<Option<(TextEmbedder, Instant)>>,
}

/// The index plus a lazily loaded embedding model that is dropped after five idle minutes.
#[derive(Clone)]
pub struct Searcher {
    inner: Arc<Shared>,
}

impl Searcher {
    pub fn new(model_dir: PathBuf, db_path: &Path, threads: usize) -> Result<Self> {
        let index = Index::open(db_path)?;
        Ok(Self { inner: Arc::new(Shared { model_dir, threads, index: Mutex::new(index), embedder: Mutex::new(None) }) })
    }

    pub fn search(&self, notes_dir: &Path, query: &str, filters: &Filters, limit: usize) -> Result<Vec<Hit>> {
        self.with(|index, emb| {
            index.refresh(notes_dir, emb)?;
            index.search(query, emb, filters, limit)
        })
    }

    /// Call after a note is saved so the next search is quick.
    pub fn refresh(&self, notes_dir: &Path) -> Result<()> {
        self.with(|index, emb| index.refresh(notes_dir, emb))
    }

    fn with<T>(&self, f: impl FnOnce(&Index, &mut TextEmbedder) -> Result<T>) -> Result<T> {
        let mut slot = self.inner.embedder.lock();
        if slot.is_none() {
            *slot = Some((TextEmbedder::load(&self.inner.model_dir, self.inner.threads)?, Instant::now()));
            let shared = self.inner.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(30));
                    let mut slot = shared.embedder.lock();
                    if slot.as_ref().is_none_or(|(_, last)| last.elapsed() >= IDLE) {
                        *slot = None;
                        return;
                    }
                }
            });
        }
        let (emb, last) = slot.as_mut().expect("loaded above");
        let out = f(&self.inner.index.lock(), emb);
        *last = Instant::now();
        out
    }
}

// ---- reading a query ----

const MEETING_WORDS: &[&str] = &["meetings", "meeting", "calls", "call", "standups", "standup", "syncs", "sync", "1:1s", "1:1", "one-to-ones", "one-to-one"];
const NOTE_WORDS: &[&str] = &["voice notes", "voice note", "voice memos", "voice memo", "memos", "memo"];

/// Pulls a person, a time and a kind of note out of a search, leaving the topic. People are only those
/// who appear in the notes (`names`), so nothing is guessed.
pub fn read_query(query: &str, names: &[String], today: NaiveDate) -> (String, Filters) {
    let mut rest = format!(" {} ", query.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "));
    let mut filters = Filters::default();
    let take = |rest: &mut String, phrase: &str| {
        let p = format!(" {phrase} ");
        let hit = rest.contains(&p);
        if hit {
            *rest = rest.replacen(&p, " ", 1);
        }
        hit
    };
    let mut by_length: Vec<&String> = names.iter().filter(|n| n.chars().count() >= 2).collect();
    by_length.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in by_length {
        let lower = name.to_lowercase();
        if take(&mut rest, &lower) || take(&mut rest, &format!("{lower}'s")) {
            filters.person = Some(name.clone());
            break;
        }
    }
    if let Some((phrase, from, to)) = crate::when::find_range(&rest, today) {
        take(&mut rest, &phrase);
        (filters.after, filters.before) = (Some(from), Some(to));
    }
    if let Some(w) = NOTE_WORDS.iter().find(|w| rest.contains(&format!(" {w} "))) {
        take(&mut rest, w);
        filters.kind = Some("voice-note".into());
    } else if let Some(w) = MEETING_WORDS.iter().find(|w| rest.contains(&format!(" {w} "))) {
        take(&mut rest, w);
        filters.kind = Some("meeting".into());
    }
    let topic = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    // "notes from yesterday" leaves nothing worth embedding; the filters carry it
    let filler = ["my", "the", "notes", "note", "from", "with", "in", "on", "about", "all", "any", "show", "me", "find"];
    let empty = topic.split_whitespace().all(|w| filler.contains(&w));
    (if empty { query.to_string() } else { topic }, filters)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntype: meeting\ndate: 2026-10-22T10:00:00\n---\n\n# Meeting\n\n## Summary\n\n### Decisions\n- Ship the beta on Friday.\n\n## Transcript\n\n**[00:00] Alice:** Hello everyone.\n\n**[00:20] Bob:** The cost feels too high.\n\n**[00:50] Alice:** Let us revisit pricing.\n\n**[01:05] Bob:** Agreed.\n";

    #[test]
    fn chunks_front_matter_summary_and_transcript_windows() {
        let (meta, ps) = chunk(NOTE);
        assert_eq!(meta.date, NaiveDate::from_ymd_opt(2026, 10, 22));
        assert_eq!(meta.kind.as_deref(), Some("meeting"));
        assert_eq!(ps[0], Passage { t: None, speakers: vec![], text: "Decisions: - Ship the beta on Friday.".into() });
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[1].t, Some(0));
        assert_eq!(ps[1].speakers, ["Alice", "Bob"]);
        assert!(ps[1].text.contains("Bob: The cost feels too high."));
        assert_eq!(ps[2].t, Some(50));
        assert_eq!(ps[2].speakers, ["Alice", "Bob"]);
    }

    #[test]
    fn long_text_is_split() {
        let md = format!("{}\n", "word ".repeat(300));
        let (_, ps) = chunk(&md);
        assert_eq!(ps.len(), 3);
        assert!(ps.iter().all(|p| p.text.split_whitespace().count() <= MAX_WORDS));
    }

    #[test]
    fn rrf_rewards_agreement() {
        let r = rrf(&[vec![1, 2, 3], vec![3, 1]]);
        assert_eq!(r.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 3, 2]);
        assert!((r[0].1 - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-6);
    }

    #[test]
    fn keywords_prefer_matching_passages() {
        let s = keyword_scores("pricing worries", &["we talked about pricing and worries", "sunny weather", "pricing only"]);
        assert!(s[0] > s[2] && s[2] > s[1] && s[1] == 0.0);
    }

    /// Bag-of-words hash vectors stand in for the model so the index can be tested without it.
    fn fake(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; 64];
                for w in terms(t.trim_start_matches(QUERY_PREFIX)) {
                    v[w.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % 64] += 1.0;
                }
                earshot_core::speakers::normalized(v)
            })
            .collect())
    }

    #[test]
    fn index_refreshes_searches_and_filters() {
        let dir = std::env::temp_dir().join(format!("earshot-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("a.md"), NOTE).unwrap();
        std::fs::write(dir.join("b.md"), "---\ntype: voice-note\ndate: 2026-09-01T08:00:00\n---\n\nRemember to buy milk and eggs.\n").unwrap();
        let idx = Index::open(&dir.join("search.db")).unwrap();
        idx.refresh_with(&dir, &mut fake).unwrap();

        let all = idx.search_with("pricing cost", &mut fake, &Filters::default(), 5).unwrap();
        assert!(all[0].path.ends_with("a.md") && all[0].text.contains("pricing"));
        assert!(all.iter().filter(|h| h.path.ends_with("a.md")).count() <= PER_NOTE);

        let voice = Filters { kind: Some("voice-note".into()), ..Default::default() };
        let hits = idx.search_with("milk", &mut fake, &voice, 5).unwrap();
        assert!(hits.iter().all(|h| h.path.ends_with("b.md")) && !hits.is_empty());

        let late = Filters { after: NaiveDate::from_ymd_opt(2026, 10, 1), ..Default::default() };
        assert!(idx.search_with("milk", &mut fake, &late, 5).unwrap().iter().all(|h| h.path.ends_with("a.md")));
        let bob = Filters { person: Some("bob".into()), ..Default::default() };
        let hits = idx.search_with("cost", &mut fake, &bob, 5).unwrap();
        assert!(!hits.is_empty() && hits.iter().all(|h| h.speakers.contains(&"Bob".to_string())));

        // unchanged notes are not re-embedded; removed ones disappear
        let mut calls = 0;
        idx.refresh_with(&dir, &mut |t| {
            calls += 1;
            fake(t)
        })
        .unwrap();
        assert_eq!(calls, 0);
        std::fs::remove_file(dir.join("b.md")).unwrap();
        idx.refresh_with(&dir, &mut fake).unwrap();
        assert!(idx.search_with("milk", &mut fake, &voice, 5).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_a_query() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let names = vec!["Sarah".to_string(), "Daniel Reyes".to_string(), "Me".to_string()];
        let (q, f) = read_query("what did Sarah say about pricing last week", &names, today);
        assert_eq!(q, "what did say about pricing");
        assert_eq!(f.person.as_deref(), Some("Sarah"));
        assert_eq!((f.after, f.before), (NaiveDate::from_ymd_opt(2026, 10, 12), NaiveDate::from_ymd_opt(2026, 10, 18)));
        let (q, f) = read_query("meetings with Daniel Reyes in September about the launch", &names, today);
        assert_eq!((q.as_str(), f.kind.as_deref(), f.person.as_deref()), ("with about the launch", Some("meeting"), Some("Daniel Reyes")));
        let (q, f) = read_query("my voice notes about app ideas", &names, today);
        assert_eq!((q.as_str(), f.kind.as_deref(), f.person), ("my about app ideas", Some("voice-note"), None));
        let (q, f) = read_query("notes from yesterday", &names, today);
        assert_eq!((q.as_str(), f.after), ("notes from yesterday", NaiveDate::from_ymd_opt(2026, 10, 22)));
        let (q, f) = read_query("refund policy", &names, today);
        assert_eq!((q.as_str(), f.person, f.after, f.kind), ("refund policy", None, None, None));
    }
}
