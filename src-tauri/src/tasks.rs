//! Action items, kept in each note as a Markdown checklist under "## Action items" so they tick off in Obsidian
//! too: `- [ ] **Sarah** — Send the pricing page to legal (due 2026-10-24) [00:36]`.

use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const HEADING: &str = "Action items";

static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^- \[( |x|X)\] (?:\*\*(.+?)\*\* — )?(.+?)(?: \(due (\d{4}-\d{2}-\d{2})\))?(?: \[(\d+):(\d+)\])?\s*$").unwrap()
});
/// What the extraction pass writes: `- [mm:ss] Owner: task (due: 2026-10-24)`.
static EXTRACTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-*]\s*(?:\[(\d+):(\d+)\]\s*)?([^:\n]{1,40}):\s*(.+?)\s*(?:\(due:?\s*([^)]*)\))?\s*$").unwrap());

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub done: bool,
    pub owner: Option<String>,
    pub text: String,
    /// YYYY-MM-DD
    pub due: Option<String>,
    /// Seconds into the meeting where it was said.
    pub t: Option<u32>,
}

impl Task {
    pub fn to_md(&self) -> String {
        let mut s = format!("- [{}] ", if self.done { "x" } else { " " });
        if let Some(o) = &self.owner {
            s.push_str(&format!("**{o}** — "));
        }
        s.push_str(self.text.trim());
        if let Some(d) = &self.due {
            s.push_str(&format!(" (due {d})"));
        }
        if let Some(t) = self.t {
            s.push_str(&format!(" [{:02}:{:02}]", t / 60, t % 60));
        }
        s
    }

    pub fn parse(line: &str) -> Option<Task> {
        let c = LINE.captures(line.trim_end())?;
        let n = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
        Some(Task {
            done: &c[1] != " ",
            owner: c.get(2).map(|m| m.as_str().trim().to_string()),
            text: c[3].trim().to_string(),
            due: c.get(4).map(|m| m.as_str().to_string()),
            t: n(5).zip(n(6)).map(|(m, s)| m * 60 + s),
        })
    }
}

/// Tasks from the extraction pass. "Unassigned" owners are dropped; a due date is kept only when it's a real date.
pub fn from_extracted(text: &str) -> Vec<Task> {
    text.lines()
        .filter_map(|l| EXTRACTED.captures(l.trim()))
        .filter_map(|c| {
            let task = c[4].trim().trim_end_matches('.').to_string();
            if task.is_empty() || task.eq_ignore_ascii_case("none") {
                return None;
            }
            let owner = c[3].trim().trim_matches('*').trim();
            let n = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
            Some(Task {
                done: false,
                owner: (!owner.eq_ignore_ascii_case("unassigned") && !owner.is_empty()).then(|| owner.to_string()),
                text: task,
                due: c.get(5).and_then(|m| NaiveDate::parse_from_str(m.as_str().trim(), "%Y-%m-%d").ok()).map(|d| d.to_string()),
                t: n(1).zip(n(2)).map(|(m, s)| m * 60 + s),
            })
        })
        .collect()
}

pub fn to_md(tasks: &[Task]) -> String {
    tasks.iter().map(Task::to_md).collect::<Vec<_>>().join("\n")
}

/// An all-day calendar event for a task with a due date, as an .ics file the default calendar app opens.
pub fn ics(task: &Task, note_title: &str) -> Option<String> {
    let due = NaiveDate::parse_from_str(task.due.as_deref()?, "%Y-%m-%d").ok()?;
    let esc = |s: &str| s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n");
    let summary = match &task.owner {
        Some(o) => format!("{o}: {}", task.text),
        None => task.text.clone(),
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    Some(format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Earshot//EN\r\nBEGIN:VEVENT\r\nUID:{stamp}-{}@earshot\r\nDTSTAMP:{stamp}\r\n\
         DTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\nSUMMARY:{}\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        due.format("%Y%m%d"),
        due.format("%Y%m%d"),
        (due + chrono::Duration::days(1)).format("%Y%m%d"),
        esc(&summary),
        esc(&format!("From \"{note_title}\" in Earshot")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let t = Task { done: false, owner: Some("Sarah".into()), text: "Send it to legal".into(), due: Some("2026-10-24".into()), t: Some(36) };
        assert_eq!(t.to_md(), "- [ ] **Sarah** — Send it to legal (due 2026-10-24) [00:36]");
        assert_eq!(Task::parse(&t.to_md()), Some(t));
        let bare = Task { done: true, owner: None, text: "Book the dentist".into(), due: None, t: None };
        assert_eq!(Task::parse("- [x] Book the dentist"), Some(bare));
        assert_eq!(Task::parse("Just a line"), None);
    }

    #[test]
    fn reads_extraction() {
        let got = from_extracted(
            "- [00:36] Sarah: Finalize the pricing page and send it to legal (due: 2026-10-24)\n\
             - [01:10] Unassigned: Check the refund wording\n- [02:00] Daniel: Move production (due: Thursday)\nNone.",
        );
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].owner.as_deref(), Some("Sarah"));
        assert_eq!(got[0].due.as_deref(), Some("2026-10-24"));
        assert_eq!(got[0].t, Some(36));
        assert_eq!(got[1].owner, None);
        assert_eq!(got[2].due, None);
    }

    #[test]
    fn calendar_event() {
        let t = Task { done: false, owner: Some("Me".into()), text: "Ship, then rest".into(), due: Some("2026-10-24".into()), t: None };
        let s = ics(&t, "Weekly sync").unwrap();
        assert!(s.contains("DTSTART;VALUE=DATE:20261024") && s.contains("DTEND;VALUE=DATE:20261025"));
        assert!(s.contains("SUMMARY:Me: Ship\\, then rest"));
    }
}
