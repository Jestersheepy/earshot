//! Meeting summaries at three depths. Transcripts too long for one prompt are first condensed part by part
//! into notes that keep every detail; each mode then works from those.

use std::path::Path;

use anyhow::Result;
use earshot_core::segment::Line;
use serde::{Deserialize, Serialize};

use crate::llm::Pool;

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

const CONCISE: &str = "Summarize this meeting for someone with 20 seconds to spare. Use the heading '### TL;DR' with at most 3 \
short bullets on the outcomes, then '### Action items' with one bullet per task as '- **Owner** — task'. Stay under 120 words. \
If there are no action items, write 'None mentioned.' under that heading.";

const EXTRACT: &str = "List every action item in this meeting: tasks, commitments (\"I'll look into it\"), requests, follow-ups \
and deadlines, including implicit ones. One per line as '- [mm:ss] Owner: task (due: date if stated)'. The owner is whoever \
will do it; if unclear, write 'Unassigned'. List nothing else. If there are none, write 'None.'";

const STANDARD: &str = "Write meeting notes that give someone who missed the meeting every detail they need. Use these \
headings in this order:\n\
'### Summary': 2-4 sentences on the purpose and the outcomes.\n\
'### Key points': bullets grouped by topic (bold the topic first), keeping specifics: numbers, dates, names, options considered.\n\
'### Decisions': what was agreed.\n\
'### Action items': every item from the list after the transcript, as '- **Owner** — task (due date if stated)'. Do not drop any.\n\
'### Open questions': what was left unresolved.\n\
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

pub fn summarize(pool: &Pool, model: &Path, ctx: u32, lines: &[Line], head: &str, mode: Mode) -> Result<String> {
    let ask = |task: &str, material: &str, max_tokens: u32, temperature: f32| {
        let user = format!("{task}\n\n{head}\n----------\n{material}");
        pool.chat(model, ctx, SYSTEM, &user, max_tokens, temperature).map(|s| tidy(&s))
    };
    let source = source(lines, ctx, |part| ask(CONDENSE, part, 1024, 0.2))?;
    if mode == Mode::Concise {
        return ask(CONCISE, &source, 400, 0.3);
    }
    let actions = ask(EXTRACT, &source, 768, 0.1)?;
    let notes = ask(STANDARD, &format!("{source}\n----------\nAction items found in the transcript:\n{actions}"), 1600, 0.3)?;
    if mode == Mode::Standard {
        return Ok(notes);
    }
    let between = with_heading(ask(BETWEEN, &source, 700, 0.5)?, "### Between the lines");
    let suggested = ask(SUGGEST, &format!("{between}\n\nAgreed action items:\n{actions}"), 500, 0.4)?;
    Ok(format!("{notes}\n\n{between}\n\n{}", with_heading(suggested, "### Suggested actions")))
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
}
