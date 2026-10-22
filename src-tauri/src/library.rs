//! Reads the notes folder for the Notes window: front matter, summary, transcript lines. The Markdown
//! files stay the source of truth (Obsidian-friendly); nothing here is cached.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\*\*\[(\d+):(\d+)(?::(\d+))?\] ([^:*]+):\*\* ?(.*)$").unwrap());

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NoteMeta {
    pub path: String,
    pub kind: String,
    pub title: String,
    pub date: String,
    pub duration: String,
    pub app: Option<String>,
    pub audio: Option<String>,
    pub preview: String,
    pub words: usize,
}

#[derive(Serialize)]
pub struct TranscriptLine {
    pub t: u32,
    pub speaker: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct NoteDetail {
    pub meta: NoteMeta,
    pub summary: Option<String>,
    pub lines: Vec<TranscriptLine>,
    pub body: String,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub meta: NoteMeta,
    pub snippet: String,
}

struct Parsed {
    front: Vec<(String, String)>,
    summary: Option<String>,
    lines: Vec<TranscriptLine>,
    body: String,
}

fn parse(text: &str) -> Parsed {
    let text = text.replace("\r\n", "\n");
    let (front, rest) = match text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")) {
        Some((f, r)) => (
            f.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect(),
            r.to_string(),
        ),
        None => (Vec::new(), text.clone()),
    };
    let mut summary: Option<String> = None;
    let mut lines = Vec::new();
    let mut body = Vec::new();
    let mut section = "";
    for l in rest.lines() {
        if l.starts_with("# ") {
            continue;
        }
        if let Some(h) = l.strip_prefix("## ") {
            section = match h.trim().to_lowercase().as_str() {
                "summary" => "summary",
                "transcript" => "transcript",
                _ => "body",
            };
            if section == "summary" {
                summary = Some(String::new());
            }
            continue;
        }
        if let Some(c) = LINE.captures(l) {
            let n = |i: usize| c.get(i).map_or(0, |m| m.as_str().parse::<u32>().unwrap_or(0));
            let t = if c.get(3).is_some() { n(1) * 3600 + n(2) * 60 + n(3) } else { n(1) * 60 + n(2) };
            lines.push(TranscriptLine { t, speaker: c[4].trim().to_string(), text: c[5].trim().to_string() });
            continue;
        }
        match section {
            "summary" => {
                let s = summary.as_mut().unwrap();
                s.push_str(l);
                s.push('\n');
            }
            "transcript" => {}
            _ => body.push(l),
        }
    }
    let summary = summary.map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && !s.starts_with("_Too short"));
    Parsed { front, summary, lines, body: body.join("\n").trim().to_string() }
}

fn get<'a>(front: &'a [(String, String)], key: &str) -> Option<&'a str> {
    front.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

fn first_words(s: &str, n: usize) -> String {
    let w: Vec<&str> = s.split_whitespace().collect();
    let mut out = w[..w.len().min(n)].join(" ");
    out = out.trim_end_matches([',', '.', ';', ':']).to_string();
    if w.len() > n {
        out.push('…');
    }
    out
}

fn meta(path: &Path, p: &Parsed) -> NoteMeta {
    let folder = path.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let kind = get(&p.front, "type")
        .map(String::from)
        .unwrap_or_else(|| if folder.contains("meeting") { "meeting".into() } else { "voice-note".into() });
    let app = get(&p.front, "app").map(String::from);
    let text_for_preview = if kind == "meeting" {
        p.summary.clone().unwrap_or_else(|| p.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "))
    } else {
        p.body.clone()
    };
    let preview_src: String =
        text_for_preview.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join(" ").replace("**", "").replace("- ", "");
    let title = get(&p.front, "title").map(String::from).unwrap_or_else(|| match kind.as_str() {
        "meeting" => match &app {
            Some(a) if !a.ends_with(".exe") => format!("{a} call"),
            _ => "Meeting".into(),
        },
        _ => first_words(&p.body, 8),
    });
    let audio = get(&p.front, "audio")
        .map(|a| a.trim_start_matches("[[").trim_end_matches("]]"))
        .map(|a| path.with_file_name(a))
        .filter(|a| a.exists())
        .map(|a| a.to_string_lossy().into_owned());
    let date = get(&p.front, "date").map(String::from).unwrap_or_else(|| {
        let m = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        m.map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default()
    });
    let words = if p.lines.is_empty() { p.body.split_whitespace().count() } else { p.lines.iter().map(|l| l.text.split_whitespace().count()).sum() };
    NoteMeta {
        path: path.to_string_lossy().into_owned(),
        kind,
        title: if title.is_empty() { "Untitled".into() } else { title },
        date,
        duration: get(&p.front, "duration").unwrap_or("").to_string(),
        app,
        audio,
        preview: first_words(&preview_src, 40),
        words,
    }
}

fn note_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 2 {
                stack.push((p, depth + 1));
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")) {
                out.push(p);
            }
        }
    }
    out
}

fn load(dir: &Path) -> Vec<(NoteMeta, String)> {
    let mut notes: Vec<(NoteMeta, String)> = note_files(dir)
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let parsed = parse(&text);
            Some((meta(&p, &parsed), text))
        })
        .collect();
    notes.sort_by(|a, b| b.0.date.cmp(&a.0.date));
    notes
}

pub fn list(dir: &Path) -> Vec<NoteMeta> {
    load(dir).into_iter().map(|(m, _)| m).collect()
}

/// Every term must appear (case-insensitive) in the title or text.
pub fn search(dir: &Path, query: &str) -> Vec<SearchHit> {
    let terms: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    load(dir)
        .into_iter()
        .filter_map(|(meta, text)| {
            let hay = format!("{}\n{}", meta.title, text).to_lowercase();
            if !terms.iter().all(|t| hay.contains(t)) {
                return None;
            }
            let at = hay.find(&terms[0]).unwrap_or(0);
            let flat = format!("{}\n{}", meta.title, text);
            let chars: Vec<char> = flat.chars().collect();
            let at_char = hay[..at].chars().count();
            let from = at_char.saturating_sub(60);
            let to = (at_char + 140).min(chars.len());
            let snippet: String = chars[from..to].iter().collect::<String>().replace("**", "").split_whitespace().collect::<Vec<_>>().join(" ");
            Some(SearchHit { meta, snippet: format!("{}{}{}", if from > 0 { "…" } else { "" }, snippet, if to < chars.len() { "…" } else { "" }) })
        })
        .collect()
}

/// Refuse anything outside the notes folder: paths come from the webview.
pub fn checked(dir: &Path, path: &str) -> Result<PathBuf> {
    let p = std::fs::canonicalize(path).with_context(|| format!("no such note: {path}"))?;
    let d = std::fs::canonicalize(dir).context("notes folder missing")?;
    if !p.starts_with(&d) || !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")) {
        bail!("not a note: {path}");
    }
    Ok(p)
}

pub fn detail(path: &Path) -> Result<NoteDetail> {
    let text = std::fs::read_to_string(path)?;
    let p = parse(&text);
    let meta = meta(path, &p);
    Ok(NoteDetail { meta, summary: p.summary, lines: p.lines, body: p.body })
}

/// Set (or add) a front-matter key, keeping everything else byte for byte.
pub fn set_front(path: &Path, key: &str, value: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    let value = value.replace('\n', " ");
    let out = match text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")) {
        Some((front, rest)) => {
            let mut lines: Vec<String> = front.lines().map(String::from).collect();
            match lines.iter_mut().find(|l| l.split_once(':').is_some_and(|(k, _)| k.trim() == key)) {
                Some(l) => *l = format!("{key}: {value}"),
                None => lines.push(format!("{key}: {value}")),
            }
            format!("---\n{}\n---\n{rest}", lines.join("\n"))
        }
        None => format!("---\n{key}: {value}\n---\n\n{text}"),
    };
    std::fs::write(path, out)?;
    Ok(())
}

/// Replace the "## Summary" section's content (adding the section if missing).
pub fn set_summary(path: &Path, summary: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    let out = match (text.find("\n## Summary\n"), text.find("\n## Transcript\n")) {
        (Some(a), Some(b)) if a < b => format!("{}\n## Summary\n\n{}\n{}", &text[..a], summary.trim(), &text[b..]),
        (None, Some(b)) => format!("{}\n## Summary\n\n{}\n{}", &text[..b], summary.trim(), &text[b..]),
        _ => bail!("this note has no transcript to summarize"),
    };
    std::fs::write(path, out)?;
    Ok(())
}

/// Transcript lines as segment Lines, for re-summarizing.
pub fn transcript(path: &Path) -> Result<Vec<earshot_core::segment::Line>> {
    let p = parse(&std::fs::read_to_string(path)?);
    Ok(p.lines
        .into_iter()
        .map(|l| earshot_core::segment::Line { start: l.t as f32, end: l.t as f32, speaker: l.speaker, text: l.text })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meeting() {
        let md = "---\ntype: meeting\ndate: 2026-10-22T03:01:28\nduration: 0:56\napp: Zoom\n---\n\n# Meeting · Thu\n\n## Summary\n\n### Summary\nShipped it.\n\n## Transcript\n\n**[00:00] Others:** Hi team.\n\n**[01:05] Me:** Thanks.\n";
        let p = parse(md);
        let m = meta(Path::new("C:/n/Meetings/x.md"), &p);
        assert_eq!(m.title, "Zoom call");
        assert_eq!(m.kind, "meeting");
        assert_eq!(p.lines.len(), 2);
        assert_eq!(p.lines[1].t, 65);
        assert_eq!(p.summary.as_deref(), Some("### Summary\nShipped it."));
        let v = parse("---\ntype: voice-note\naudio: [[a.wav]]\n---\n\nSo anyone could be talking right now, and it should pick up.");
        assert_eq!(meta(Path::new("C:/n/Voice notes/a.md"), &v).title, "So anyone could be talking right now, and…");
    }
}
