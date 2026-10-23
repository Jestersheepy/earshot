//! Reads the notes folder for the Notes window: front matter, summary, transcript lines. The Markdown
//! files stay the source of truth (Obsidian-friendly); nothing here is cached.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

use crate::summary::{Chapter, Mode};
use crate::tasks::Task;

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
    /// One sentence on the outcome, written with the summary.
    pub gist: Option<String>,
    /// For voice notes: note, todo, idea or reminder.
    pub tags: Vec<String>,
    pub participants: Vec<String>,
    pub open_tasks: usize,
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
    /// Generated summaries by mode name ("concise", "standard", "nuanced").
    pub summaries: BTreeMap<String, String>,
    pub lines: Vec<TranscriptLine>,
    pub body: String,
    /// Speakers named from voice memory that you haven't confirmed in this note.
    pub unconfirmed: Vec<String>,
    pub tasks: Vec<Task>,
    pub chapters: Vec<Chapter>,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub meta: NoteMeta,
    pub snippet: String,
}

struct Parsed {
    front: Vec<(String, String)>,
    summaries: BTreeMap<String, String>,
    /// The one shown in previews: standard if there is one.
    summary: Option<String>,
    lines: Vec<TranscriptLine>,
    body: String,
    tasks: Vec<Task>,
    chapters: Vec<Chapter>,
}

static CHAPTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- \[(\d+):(\d+)(?::(\d+))?\] (.+)$").unwrap());

fn parse(text: &str) -> Parsed {
    let text = text.replace("\r\n", "\n");
    let (front, rest) = match text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")) {
        Some((f, r)) => (
            f.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect(),
            r.to_string(),
        ),
        None => (Vec::new(), text.clone()),
    };
    let mut summaries: BTreeMap<String, String> = BTreeMap::new();
    let mut mode = Mode::Standard;
    let mut lines = Vec::new();
    let mut body = Vec::new();
    let (mut tasks, mut chapters) = (Vec::new(), Vec::new());
    let mut section = "";
    for l in rest.lines() {
        if l.starts_with("# ") {
            continue;
        }
        if let Some(h) = l.strip_prefix("## ") {
            section = match (Mode::from_heading(h), h.trim().to_lowercase().as_str()) {
                (Some(m), _) => {
                    mode = m;
                    summaries.insert(m.name().into(), String::new());
                    "summary"
                }
                (None, "transcript") => "transcript",
                (None, "action items") => "tasks",
                (None, "chapters") => "chapters",
                _ => "body",
            };
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
                let s = summaries.get_mut(mode.name()).unwrap();
                s.push_str(l);
                s.push('\n');
            }
            "transcript" => {}
            "tasks" => tasks.extend(Task::parse(l)),
            "chapters" => {
                if let Some(c) = CHAPTER.captures(l) {
                    let n = |i: usize| c.get(i).map_or(0, |m| m.as_str().parse::<u32>().unwrap_or(0));
                    let t = if c.get(3).is_some() { n(1) * 3600 + n(2) * 60 + n(3) } else { n(1) * 60 + n(2) };
                    chapters.push(Chapter { t, title: c[4].trim().to_string() });
                }
            }
            _ => body.push(l),
        }
    }
    let summaries: BTreeMap<String, String> = summaries
        .into_iter()
        .map(|(k, s)| (k, s.trim().to_string()))
        .filter(|(_, s)| !s.is_empty() && !s.starts_with("_Too short"))
        .collect();
    let summary = ["standard", "concise", "nuanced"].iter().find_map(|k| summaries.get(*k).cloned());
    Parsed { front, summaries, summary, lines, body: body.join("\n").trim().to_string(), tasks, chapters }
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
    let text_for_preview = if let Some(g) = get(&p.front, "gist") {
        g.to_string()
    } else if kind == "meeting" {
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
        gist: get(&p.front, "gist").map(String::from),
        tags: list_field(get(&p.front, "tags")),
        participants: list_field(get(&p.front, "participants")),
        open_tasks: p.tasks.iter().filter(|t| !t.done).count(),
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

fn list_field(v: Option<&str>) -> Vec<String> {
    v.map_or(Vec::new(), |t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
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
    let unconfirmed = get(&p.front, "unconfirmed").map_or(Vec::new(), |u| u.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    Ok(NoteDetail { meta, summaries: p.summaries, lines: p.lines, body: p.body, unconfirmed, tasks: p.tasks, chapters: p.chapters })
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

/// Replace one mode's summary section, adding it before the transcript if missing.
pub fn set_summary(path: &Path, mode: Mode, summary: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    std::fs::write(path, with_summary(&text, mode, summary)?)?;
    Ok(())
}

fn with_summary(text: &str, mode: Mode, summary: &str) -> Result<String> {
    let heading = format!("\n## {}\n", mode.heading());
    let Some(transcript) = text.find("\n## Transcript\n") else { bail!("this note has no transcript to summarize") };
    Ok(match text.find(&heading) {
        Some(a) => {
            // the section runs to the next "## " heading
            let start = a + heading.len();
            let end = text[start..].find("\n## ").map_or(text.len(), |i| start + i);
            format!("{}{heading}\n{}\n{}", &text[..a], summary.trim(), &text[end..])
        }
        None => format!("{}{heading}\n{}\n{}", &text[..transcript], summary.trim(), &text[transcript..]),
    })
}

/// Replace a "## heading" section, or add it before the transcript (at the end for notes without one).
pub fn set_section(path: &Path, heading: &str, content: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    std::fs::write(path, with_section(&text, heading, content))?;
    Ok(())
}

fn with_section(text: &str, heading: &str, content: &str) -> String {
    let h = format!("\n## {heading}\n");
    match text.find(&h) {
        Some(a) => {
            let start = a + h.len();
            let end = text[start..].find("\n## ").map_or(text.len(), |i| start + i);
            let tail = &text[end..];
            format!("{}{h}\n{}\n{}", &text[..a], content.trim(), if tail.is_empty() { "" } else { tail })
        }
        None => match text.find("\n## Transcript\n") {
            Some(t) => format!("{}{h}\n{}\n{}", &text[..t], content.trim(), &text[t..]),
            None => format!("{}\n{h}\n{}\n", text.trim_end(), content.trim()),
        },
    }
}

/// The title, gist, action items and chapters written alongside a summary. Action items are written even when
/// there are none, so re-summarizing knows they've been looked for.
pub fn save_extras(path: &Path, w: &crate::summary::Written) -> Result<()> {
    if let Some(t) = &w.title {
        set_front(path, "title", t)?;
    }
    if let Some(g) = &w.gist {
        set_front(path, "gist", g)?;
    }
    if let Some(tasks) = &w.tasks {
        let md = if tasks.is_empty() { "None mentioned.".to_string() } else { crate::tasks::to_md(tasks) };
        set_section(path, crate::tasks::HEADING, &md)?;
    }
    if !w.chapters.is_empty() {
        let md = w.chapters.iter().map(|c| format!("- [{:02}:{:02}] {}", c.t / 60, c.t % 60, c.title)).collect::<Vec<_>>().join("\n");
        set_section(path, "Chapters", &md)?;
    }
    Ok(())
}

/// What a note still lacks that a summary pass can add: notes from before titles, chapters and action items.
pub fn missing_extras(path: &Path) -> Result<crate::summary::Want> {
    let text = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    let p = parse(&text);
    Ok(crate::summary::Want {
        title: get(&p.front, "title").is_none(),
        chapters: !text.contains("\n## Chapters\n"),
        tasks: !text.contains(&format!("\n## {}\n", crate::tasks::HEADING)),
    })
}

/// Tick or untick the `index`th action item.
pub fn set_task_done(path: &Path, index: usize, done: bool) -> Result<()> {
    let p = parse(&std::fs::read_to_string(path)?);
    let mut tasks = p.tasks;
    let Some(t) = tasks.get_mut(index) else { bail!("no such task") };
    t.done = done;
    set_section(path, crate::tasks::HEADING, &crate::tasks::to_md(&tasks))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub note: NoteMeta,
    pub index: usize,
    pub task: Task,
}

/// Every action item in every note, newest note first.
pub fn all_tasks(dir: &Path) -> Vec<TaskRow> {
    load(dir)
        .into_iter()
        .flat_map(|(meta, text)| {
            parse(&text).tasks.into_iter().enumerate().map(move |(index, task)| TaskRow { note: meta.clone(), index, task })
        })
        .collect()
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

        let md = with_summary(md, Mode::Concise, "### TL;DR\n- Shipped.").unwrap();
        let md = with_summary(&md, Mode::Standard, "### Summary\nShipped it, twice.").unwrap();
        let p = parse(&md);
        assert_eq!(p.summaries["concise"], "### TL;DR\n- Shipped.");
        assert_eq!(p.summaries["standard"], "### Summary\nShipped it, twice.");
        assert_eq!(p.lines.len(), 2);
        let md = with_section(&md, "Action items", "- [ ] **Sarah** — Send it to legal (due 2026-10-24) [00:36]");
        let md = with_section(&md, "Chapters", "- [00:00] Onboarding\n- [01:05] Pricing");
        let md = with_section(&md, "Action items", "- [x] **Sarah** — Send it to legal (due 2026-10-24) [00:36]");
        let p = parse(&md);
        assert_eq!(p.tasks.len(), 1);
        assert!(p.tasks[0].done);
        assert_eq!(p.chapters[1], Chapter { t: 65, title: "Pricing".into() });
        assert_eq!(p.lines.len(), 2);
        assert_eq!(p.summaries.len(), 2);
        assert!(md.find("## Chapters").unwrap() < md.find("## Transcript").unwrap());
        let v = with_section("---\ntype: voice-note\n---\n\nCall the bank.\n", "Action items", "- [ ] Call the bank");
        assert_eq!(parse(&v).body, "Call the bank.");
        assert_eq!(parse(&v).tasks.len(), 1);
        let v = parse("---\ntype: voice-note\naudio: [[a.wav]]\n---\n\nSo anyone could be talking right now, and it should pick up.");
        assert_eq!(meta(Path::new("C:/n/Voice notes/a.md"), &v).title, "So anyone could be talking right now, and…");
    }
}
