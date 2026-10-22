//! Markdown notes on disk: one .md (plus optional .wav) per voice note or meeting.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_fillers() {
        assert_eq!(clean("um so uh we ship, um, friday", true), "So we ship, friday");
        assert_eq!(fmt_duration(3725.0), "1:02:05");
    }
}
