//! Markdown notes on disk: one .md (plus optional .wav) per voice note or meeting.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate};

use crate::tasks::Task;

pub fn clean(text: &str, drop_fillers: bool) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if drop_fillers {
        words.retain(|w| {
            let bare = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            !matches!(bare.as_str(), "um" | "umm" | "uh" | "uhh" | "erm" | "uhm" | "hmm" | "mm")
        });
    }
    earshot_core::segment::capitalize(&words.join(" "))
}

pub fn new_note_path(notes_dir: &Path, folder: &str, started: DateTime<Local>) -> PathBuf {
    let dir = notes_dir.join(folder);
    let _ = std::fs::create_dir_all(&dir);
    let base = started.format("%Y-%m-%d %H%M").to_string();
    let mut path = dir.join(format!("{base}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).md"));
        n += 1;
    }
    path
}

pub fn fmt_duration(secs: f32) -> String {
    let s = secs as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn write_note(
    path: &Path,
    kind: &str,
    started: DateTime<Local>,
    duration_s: f32,
    body: &str,
    extra: &[(&str, String)],
) -> std::io::Result<()> {
    let mut front = format!(
        "type: {kind}\ndate: {}\nduration: {}\n",
        started.format("%Y-%m-%dT%H:%M:%S"),
        fmt_duration(duration_s)
    );
    for (k, v) in extra {
        front.push_str(&format!("{k}: {v}\n"));
    }
    std::fs::write(path, format!("---\n{front}---\n\n{}\n", body.trim()))
}

pub const TRIAGE_SYSTEM: &str = "You file voice notes. Reply in JSON with: title (3-6 words on what it is about), type (todo or reminder if it asks for something to be done, idea for an idea, otherwise note), task (for a todo or reminder, the thing to do as a short command, else empty), when (the exact words saying when it's due, like 'by Monday', else empty).";

pub fn triage_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "type": {"enum": ["note", "todo", "idea", "reminder"]},
            "task": {"type": "string"},
            "when": {"type": "string"},
        },
        "required": ["title", "type", "task", "when"],
        "additionalProperties": false,
    })
}

#[derive(Debug, PartialEq)]
pub struct Triage {
    pub title: Option<String>,
    /// todo, idea or reminder; a plain note has none.
    pub kind: Option<String>,
    pub task: Option<Task>,
}

/// The model's reply, trusting a due date only when its words are in the note.
pub fn parse_triage(reply: &serde_json::Value, body: &str, owner: &str, today: NaiveDate) -> Triage {
    let field = |k: &str| reply[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    let title = field("title")
        .map(|t| {
            let t = t.trim_end_matches('.');
            let t = t.split_once(": ").filter(|(head, _)| head.split_whitespace().count() <= 2).map_or(t, |(_, rest)| rest);
            t.chars().take(70).collect::<String>()
        })
        // "Todo" or "Voice note" says nothing; the note's first words do better
        .filter(|t| t.split_whitespace().count() <= 10 && !["", "note", "todo", "to-do", "idea", "reminder", "task", "voice note"].contains(&t.to_lowercase().as_str()));
    let kind = field("type").filter(|k| matches!(*k, "todo" | "idea" | "reminder"));
    let said = body.to_lowercase();
    let when = field("when").map(|w| w.to_lowercase()).filter(|w| said.contains(w.trim_start_matches("by ").trim()));
    let task = match (kind, field("task")) {
        (Some("todo" | "reminder"), Some(t)) => Some(Task {
            done: false,
            owner: Some(owner.to_string()),
            text: t.trim_end_matches('.').to_string(),
            // "after work" names no day, but the task itself may ("…tomorrow after work")
            due: when.and_then(|w| crate::when::due(&w, today)).or_else(|| crate::when::due(t, today)).map(|d| d.to_string()),
            t: None,
        }),
        _ => None,
    };
    Triage { title, kind: kind.map(String::from), task }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_fillers() {
        assert_eq!(clean("um so uh we ship, um, friday", true), "So we ship, friday");
        assert_eq!(fmt_duration(3725.0), "1:02:05");
    }

    #[test]
    fn reads_triage() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let body = "I need to call the plumber about the leak, ideally by Monday.";
        let reply = serde_json::json!({"title": "Reminder: Call Plumber", "type": "reminder", "task": "Call the plumber about the leak", "when": "by Monday"});
        let t = parse_triage(&reply, body, "Me", today);
        assert_eq!(t.title.as_deref(), Some("Call Plumber"));
        assert_eq!(t.kind.as_deref(), Some("reminder"));
        let task = t.task.unwrap();
        assert_eq!((task.due.as_deref(), task.owner.as_deref()), (Some("2026-10-26"), Some("Me")));
        // a time the note never mentions is dropped
        let reply = serde_json::json!({"title": "Groceries", "type": "todo", "task": "Buy milk", "when": "by end of day tomorrow"});
        assert_eq!(parse_triage(&reply, "Buy milk on the way home", "Me", today).task.unwrap().due, None);
        let reply = serde_json::json!({"title": "Lisbon in spring", "type": "idea", "task": "Book a trip", "when": ""});
        let idea = parse_triage(&reply, "Lisbon was lovely", "Me", today);
        assert_eq!((idea.kind.as_deref(), idea.task), (Some("idea"), None));
        assert_eq!(parse_triage(&serde_json::json!({}), "x", "Me", today), Triage { title: None, kind: None, task: None });
        let body = "Pick up the dry cleaning tomorrow after work.";
        let reply = serde_json::json!({"title": "Todo", "type": "todo", "task": "Pick up dry cleaning tomorrow after work", "when": "after work"});
        let t = parse_triage(&reply, body, "Me", today);
        assert_eq!((t.title, t.task.unwrap().due.as_deref()), (None, Some("2026-10-24")));
    }
}
