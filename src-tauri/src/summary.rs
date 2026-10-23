//! Meeting summaries at three depths. Transcripts too long for one prompt are first condensed part by part
//! into notes that keep every detail; each mode then works from those.

use std::sync::LazyLock;

use anyhow::Result;
use earshot_core::segment::Line;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::backend::Summarizer;
use crate::tasks::{self, Task};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// TL;DR and action items.
    Concise,
    /// Everything someone who missed the meeting needs, every action item.
    #[default]
    Standard,
    /// Standard, plus what was said between the lines and actions that follow from it.
    Nuanced,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Concise, Mode::Standard, Mode::Nuanced];

    /// This mode's section in the note. Standard keeps the plain "Summary" older notes have.
    pub fn heading(self) -> &'static str {
        match self {
            Mode::Concise => "Summary · Concise",
            Mode::Standard => "Summary",
            Mode::Nuanced => "Summary · Nuanced",
        }
    }

    pub fn from_heading(h: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.heading().eq_ignore_ascii_case(h.trim()))
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Concise => "concise",
            Mode::Standard => "standard",
            Mode::Nuanced => "nuanced",
        }
    }
}

const SYSTEM: &str = "You are an expert meeting analyst. You write clear, specific notes in Markdown. Only use what is in the \
transcript; never invent names, dates, numbers or tasks.";

const CONDENSE: &str = "Write detailed notes on this part of a meeting transcript. Keep every topic, decision, number, date, \
name, commitment and open question, each with its [mm:ss] time and who said it. Do not drop details.";

// Action items aren't part of the summaries: they're extracted once into the note's checklist (see tasks.rs).
const CONCISE: &str = "Summarize this meeting for someone with 20 seconds to spare. Use the heading '### TL;DR' with at most 3 \
short bullets on the outcomes. Stay under 80 words.";

const EXTRACT: &str = "List every action item in this meeting: tasks, commitments (\"I'll look into it\"), requests, follow-ups \
and deadlines, including implicit ones. One per line as '- [mm:ss] Owner: task (due: YYYY-MM-DD)'. Give a due date only when \
a day is stated or clearly implied, worked out from the meeting date in the header ('Friday' is the first Friday after it); \
otherwise leave out the '(due: …)' part. The owner is whoever will do it; if unclear, write 'Unassigned'. List nothing else. \
If there are none, write 'None.'";

const TITLE: &str = "Name this meeting and sum it up. Reply with exactly two lines:\n\
Title: 3-7 words on what it was about, like 'Pricing tiers and the launch date' (no date, app or quotes)\n\
Gist: one sentence under 25 words on the outcome.";

const CHAPTERS: &str = "Split this meeting into its topics, in order: 2 to 8 chapters, fewer for a short meeting. One per \
line as '[mm:ss] Topic', where the time is when the topic starts and the topic is 2-6 words. List nothing else.";

const STANDARD: &str = "Write meeting notes that give someone who missed the meeting every detail they need. Use these \
headings in this order:\n\
'### Summary': 2-4 sentences on the purpose and the outcomes.\n\
'### Key points': bullets grouped by topic (bold the topic first), keeping specifics: numbers, dates, names, options considered.\n\
'### Decisions': what was agreed.\n\
'### Open questions': what was left unresolved.\n\
Don't list action items; they're kept separately.\n\
If a section has nothing, write 'None mentioned.'";

const BETWEEN: &str = "Read between the lines of this meeting. Under the heading '### Between the lines', write 3-6 bullets on \
what was not said outright: hesitation or concern, disagreement or tension, enthusiasm, risks, unstated assumptions, and ideas \
mentioned in passing that deserve follow-up. End each bullet with the [mm:ss] times it rests on. Do not repeat the action \
items, and do not speculate beyond what the words support.";

const SUGGEST: &str = "From these observations and the agreed action items, propose 2-5 follow-up actions that nobody committed \
to but that would address what was observed. Under the heading '### Suggested actions', one bullet each as \
'- task — why ([mm:ss])'. Do not repeat agreed action items. Name an owner only if the transcript makes it obvious.";

/// One line per utterance, with the times the nuanced pass cites.
pub fn transcript_text(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            let s = l.start as u32;
            format!("[{:02}:{:02}] {}: {}", s / 60, s % 60, l.speaker, l.text)
        })
        .collect()
}

/// What a note needs besides the summary. Each is a short extra pass while the model is loaded anyway.
#[derive(Clone, Copy, Default)]
pub struct Want {
    pub title: bool,
    pub chapters: bool,
    pub tasks: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Chapter {
    pub t: u32,
    pub title: String,
}

#[derive(Default)]
pub struct Written {
    pub summary: String,
    pub title: Option<String>,
    pub gist: Option<String>,
    pub chapters: Vec<Chapter>,
    pub tasks: Option<Vec<Task>>,
}

/// Meetings shorter than this get no chapters: one topic isn't worth a list.
const CHAPTERS_MIN_S: f32 = 180.0;

pub fn summarize(by: &Summarizer, lines: &[Line], head: &str, mode: Mode, want: Want) -> Result<Written> {
    let ask = |task: &str, material: &str, max_tokens: u32, temperature: f32| {
        let user = format!("{task}\n\n{head}\n----------\n{material}");
        by.chat(SYSTEM, &user, max_tokens, temperature).map(|s| tidy(&s))
    };
    let source = source(lines, by.ctx(), |part| ask(CONDENSE, part, 1024, 0.2))?;
    let actions = if want.tasks || mode == Mode::Nuanced { Some(ask(EXTRACT, &source, 768, 0.1)?) } else { None };
    let summary = match mode {
        Mode::Concise => ask(CONCISE, &source, 300, 0.3)?,
        Mode::Standard => ask(STANDARD, &source, 1600, 0.3)?,
        Mode::Nuanced => {
            let notes = ask(STANDARD, &source, 1600, 0.3)?;
            let between = with_heading(ask(BETWEEN, &source, 700, 0.5)?, "### Between the lines");
            let agreed = actions.as_deref().unwrap_or_default();
            let suggested = ask(SUGGEST, &format!("{between}\n\nAgreed action items:\n{agreed}"), 500, 0.4)?;
            format!("{notes}\n\n{between}\n\n{}", with_heading(suggested, "### Suggested actions"))
        }
    };
    // the extras never cost the summary: if one fails, the note just goes without it
    let (title, gist) = if want.title { ask(TITLE, &source, 100, 0.2).map(|s| title_and_gist(&s)).unwrap_or_default() } else { (None, None) };
    let length = lines.last().map_or(0.0, |l| l.end.max(l.start));
    let chapters = if want.chapters && length >= CHAPTERS_MIN_S {
        ask(CHAPTERS, &source, 300, 0.2).map(|s| chapters(&s, length as u32)).unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(Written { summary, title, gist, chapters, tasks: if want.tasks { actions.as_deref().map(tasks::from_extracted) } else { None } })
}

fn title_and_gist(s: &str) -> (Option<String>, Option<String>) {
    let field = |key: &str| {
        s.lines()
            .find_map(|l| l.trim().trim_start_matches(['*', '#', ' ']).strip_prefix(key))
            .map(|v| v.trim_start_matches(['*', ':', ' ']).trim().trim_matches(['"', '\'', '*']).trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let title = field("Title").map(|t| t.trim_end_matches('.').chars().take(80).collect());
    (title, field("Gist").map(|g| g.chars().take(240).collect()))
}

/// "[mm:ss] Topic" lines, in order and within the meeting; fewer than two is no use.
fn chapters(s: &str, length: u32) -> Vec<Chapter> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(\d+):(\d{2})(?::(\d{2}))?\]\s*[-–—:]?\s*(.+)").unwrap());
    let mut out: Vec<Chapter> = Vec::new();
    for c in s.lines().filter_map(|l| RE.captures(l)) {
        let n = |i: usize| c.get(i).map_or(0, |m| m.as_str().parse::<u32>().unwrap_or(0));
        let t = if c.get(3).is_some() { n(1) * 3600 + n(2) * 60 + n(3) } else { n(1) * 60 + n(2) };
        let title = c[4].trim().trim_matches(['*', '"']).trim().to_string();
        if t <= length && out.last().is_none_or(|p| t > p.t) && !title.is_empty() {
            out.push(Chapter { t, title });
        }
    }
    if out.len() < 2 { Vec::new() } else { out }
}

/// The transcript itself when it fits, else detailed notes on each part in order.
fn source(lines: &[Line], ctx: u32, condense: impl Fn(&str) -> Result<String>) -> Result<String> {
    let text = transcript_text(lines);
    // ~3 chars per token; half the context leaves room for the instructions, extracted items and the answer
    let budget = ctx as usize / 2 * 3;
    let mut parts: Vec<Vec<&str>> = vec![Vec::new()];
    let mut size = 0;
    for line in &text {
        if size + line.len() > budget && !parts.last().unwrap().is_empty() {
            parts.push(Vec::new());
            size = 0;
        }
        parts.last_mut().unwrap().push(line);
        size += line.len() + 1;
    }
    if parts.len() == 1 {
        return Ok(parts[0].join("\n"));
    }
    let notes = parts.iter().map(|p| condense(&p.join("\n"))).collect::<Result<Vec<_>>>()?;
    Ok(notes.iter().enumerate().map(|(i, n)| format!("Notes on part {} of the meeting:\n{n}", i + 1)).collect::<Vec<_>>().join("\n\n"))
}

/// Drop code fences and any chatter before the first heading.
fn tidy(s: &str) -> String {
    let s = s.trim().trim_start_matches("```markdown").trim_start_matches("```").trim_end_matches("```").trim();
    match s.find("###") {
        Some(i) if i > 0 && !s[..i].contains("\n- ") => s[i..].trim().to_string(),
        _ => s.to_string(),
    }
}

fn with_heading(s: String, heading: &str) -> String {
    if s.contains(heading) { s } else { format!("{heading}\n{s}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_heading(m.heading()), Some(m));
        }
        assert_eq!(Mode::from_heading("Transcript"), None);
    }

    #[test]
    fn tidy_strips_chatter() {
        assert_eq!(tidy("Sure! Here are the notes:\n\n### Summary\nShipped."), "### Summary\nShipped.");
        assert_eq!(tidy("```markdown\n### TL;DR\n- ok\n```"), "### TL;DR\n- ok");
        assert_eq!(tidy("- [00:10] Tom: send deck"), "- [00:10] Tom: send deck");
    }

    #[test]
    fn reads_title_and_chapters() {
        let (t, g) = title_and_gist("Title: \"Pricing tiers and the launch date.\"\n**Gist:** Two tiers, launch on 4 November.");
        assert_eq!(t.as_deref(), Some("Pricing tiers and the launch date"));
        assert_eq!(g.as_deref(), Some("Two tiers, launch on 4 November."));
        let c = chapters("[00:00] Onboarding results\n- [00:36] — Pricing\n[00:30] Out of order\n[09:00] Too late", 300);
        assert_eq!(c, vec![Chapter { t: 0, title: "Onboarding results".into() }, Chapter { t: 36, title: "Pricing".into() }]);
        assert!(chapters("[00:00] Just one", 300).is_empty());
    }
}
