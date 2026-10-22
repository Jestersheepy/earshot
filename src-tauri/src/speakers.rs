//! Splits a meeting's "Me" and "Others" tracks into individual voices, labels them (you, voices you've
//! named before, Speaker 1, 2, …) and renames speakers in a saved note.

use std::collections::HashMap;

use earshot_core::segment::Line;
use earshot_core::speakers::{Embedder, MIN_S, SAME_VOICE, centroid, cluster, dot};
use earshot_core::stt::SR;
use regex::Regex;

/// A known voice must match this well to be suggested (stricter than clustering).
const KNOWN_VOICE: f32 = 0.6;
/// Your own voice varies more (distance to the mic, headset or laptop), so the mic track splits less readily.
const SAME_VOICE_MIC: f32 = SAME_VOICE - 0.1;
/// A voice heard for less than this, in at most one utterance, is noise: it joins the nearest voice.
const MIN_VOICE_S: f32 = 3.0;

pub struct Voice {
    pub label: String,
    pub centroid: Vec<f32>,
    /// Utterances it was learned from.
    pub n: usize,
    /// Labelled from voice memory, not confirmed by you for this meeting.
    pub guessed: bool,
}

/// Relabel `lines` (speakers "Me" / "Others", before turns are merged) by voice. `me_name` labels your own
/// voice: the one heard most on the mic track.
pub fn diarize(
    lines: &mut [Line],
    me: &[f32],
    others: Option<&[f32]>,
    emb: &mut Embedder,
    known: &[(String, Vec<f32>)],
    me_name: &str,
) -> Vec<Voice> {
    let mut voices: Vec<Voice> = Vec::new();
    let mut next = 1;
    let mut used: Vec<String> = Vec::new();
    for (track, audio, threshold) in [("Me", Some(me), SAME_VOICE_MIC), ("Others", others, SAME_VOICE)] {
        let Some(audio) = audio else { continue };
        let idx: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].speaker == track).collect();
        let embs: Vec<Option<Vec<f32>>> = idx
            .iter()
            .map(|&i| {
                let l = &lines[i];
                let (a, b) = ((l.start * SR as f32) as usize, ((l.end * SR as f32) as usize).min(audio.len()));
                (b > a && (b - a) as f32 >= 0.4 * SR as f32).then(|| emb.embed(&audio[a..b]).ok()).flatten()
            })
            .collect();
        let long: Vec<usize> = (0..idx.len()).filter(|&k| embs[k].is_some() && dur(&lines[idx[k]]) >= MIN_S).collect();
        if long.is_empty() {
            if track == "Me" {
                idx.iter().for_each(|&i| lines[i].speaker = me_name.to_string());
            }
            continue;
        }
        let labels = cluster(&long.iter().map(|&k| embs[k].clone().unwrap()).collect::<Vec<_>>(), threshold);
        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); labels.iter().max().unwrap() + 1];
        for (j, &g) in labels.iter().enumerate() {
            groups[g].push(long[j]);
        }
        // fold away voices heard too briefly to be a person
        let seconds = |g: &Vec<usize>| g.iter().map(|&k| dur(&lines[idx[k]])).sum::<f32>();
        let big: Vec<Vec<usize>> = groups.iter().filter(|g| g.len() > 1 || seconds(g) >= MIN_VOICE_S).cloned().collect();
        let mut groups = if big.is_empty() { groups } else { big };
        let cents: Vec<Vec<f32>> = groups.iter().map(|g| centroid(g.iter().map(|&k| embs[k].as_ref().unwrap()))).collect();
        // every utterance goes to its nearest voice; ones too short to embed follow the one before
        let mut assign: Vec<Option<usize>> = embs
            .iter()
            .map(|e| e.as_ref().map(|e| (0..cents.len()).max_by(|&a, &b| dot(e, &cents[a]).total_cmp(&dot(e, &cents[b]))).unwrap()))
            .collect();
        let mut last = assign.iter().flatten().next().copied().unwrap_or(0);
        for a in assign.iter_mut() {
            last = *a.get_or_insert(last);
        }
        groups.iter_mut().for_each(Vec::clear);
        for (k, a) in assign.iter().enumerate() {
            groups[a.unwrap()].push(k);
        }
        // name each voice
        let you = (track == "Me").then(|| (0..groups.len()).max_by(|&a, &b| seconds(&groups[a]).total_cmp(&seconds(&groups[b]))).unwrap());
        let mut order: Vec<usize> = (0..groups.len()).filter(|&g| !groups[g].is_empty()).collect();
        order.sort_by_key(|&g| groups[g][0]);
        for g in order {
            let c = centroid(groups[g].iter().filter_map(|&k| embs[k].as_ref()));
            let (label, guessed) = if Some(g) == you {
                (me_name.to_string(), false)
            } else if let Some((name, _)) = known
                .iter()
                .filter(|(n, _)| !used.contains(n))
                .map(|(n, v)| (n, dot(&c, v)))
                .filter(|(_, s)| *s >= KNOWN_VOICE)
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                (name.clone(), true)
            } else {
                next += 1;
                (format!("Speaker {}", next - 1), false)
            };
            used.push(label.clone());
            for &k in &groups[g] {
                lines[idx[k]].speaker = label.clone();
            }
            voices.push(Voice { label, centroid: c, n: groups[g].len(), guessed });
        }
    }
    voices
}

fn dur(l: &Line) -> f32 {
    l.end - l.start
}

/// Rename a speaker throughout a note: transcript labels, and names in the summaries. "Me" and "Others" are
/// common words, so for those only bold mentions and labels change.
pub fn rename_in_note(md: &str, from: &str, to: &str) -> String {
    let (front, body) = match md.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")) {
        Some((f, b)) => (Some(f), b),
        None => (None, md),
    };
    let label = Regex::new(&format!(r"(?m)^(\*\*\[[0-9:]+\] ){}(:\*\*)", regex::escape(from))).unwrap();
    let mut body = label.replace_all(body, |c: &regex::Captures| format!("{}{to}{}", &c[1], &c[2])).into_owned();
    body = body.replace(&format!("**{from}**"), &format!("**{to}**"));
    if !matches!(from, "Me" | "Others") {
        let word = Regex::new(&format!(r"\b{}\b", regex::escape(from))).unwrap();
        body = word.replace_all(&body, regex::NoExpand(to)).into_owned();
    }
    match front {
        Some(f) => {
            let f = f
                .lines()
                .map(|l| match l.split_once(':') {
                    Some((k, v)) if matches!(k.trim(), "participants" | "unconfirmed") => {
                        let mut names: Vec<String> = Vec::new();
                        for n in v.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                            let n = if n == from { to } else { n };
                            if !names.iter().any(|x| x == n) && !(k.trim() == "unconfirmed" && n == to) {
                                names.push(n.to_string());
                            }
                        }
                        format!("{k}: {}", names.join(", "))
                    }
                    _ => l.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("---\n{f}\n---\n{body}")
        }
        None => body,
    }
}

/// Speakers in transcript order, for the participants list.
pub fn participants(lines: &[Line]) -> Vec<String> {
    let mut seen: HashMap<&str, ()> = HashMap::new();
    lines.iter().filter(|l| seen.insert(&l.speaker, ()).is_none()).map(|l| l.speaker.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three TTS voices taking turns on the call side (models/speakers_test, from tools/make_speaker_test.ps1).
    #[test]
    fn diarizes_a_call() {
        let models = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models");
        let (Ok(dir), true) = (std::fs::read_dir(models.join("speakers_test")), models.join("speaker").exists()) else {
            eprintln!("skipping: no speaker test audio");
            return;
        };
        let mut wavs: Vec<_> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
        wavs.sort();
        let (mut audio, mut lines, mut truth) = (Vec::new(), Vec::new(), Vec::new());
        for w in &wavs {
            let pcm = earshot_core::audio::read_wav(w).unwrap();
            let start = audio.len() as f32 / SR as f32;
            audio.extend_from_slice(&pcm);
            audio.extend(std::iter::repeat_n(0.0, SR * 2 / 5));
            lines.push(Line { start, end: start + pcm.len() as f32 / SR as f32, speaker: "Others".into(), text: "…".into() });
            truth.push(w.file_stem().unwrap().to_string_lossy().rsplit('_').next().unwrap().to_string());
        }
        let mut emb = Embedder::load(&models.join("speaker"), 4).unwrap();
        let mut got = lines.clone();
        let voices = diarize(&mut got, &[], Some(&audio), &mut emb, &[], "Me");
        assert_eq!(voices.len(), 3);
        let label = |name: &str| got[truth.iter().position(|t| t == name).unwrap()].speaker.clone();
        assert_eq!((label("David"), label("Zira"), label("Hazel")), ("Speaker 1".into(), "Speaker 2".into(), "Speaker 3".into()));
        for (l, t) in got.iter().zip(&truth) {
            assert_eq!(l.speaker, label(t));
        }
        // a voice named before is recognised
        let zira = voices.iter().find(|v| v.label == "Speaker 2").unwrap().centroid.clone();
        let mut again = lines.clone();
        let voices = diarize(&mut again, &[], Some(&audio), &mut emb, &[("Zira".into(), zira)], "Me");
        assert!(voices.iter().any(|v| v.label == "Zira" && v.guessed));
        assert_eq!(again[1].speaker, "Zira");
        assert_eq!(again[2].speaker, "Speaker 2");
    }

    #[test]
    fn renames_everywhere() {
        let md = "---\ntype: meeting\nparticipants: Me, Speaker 1, Speaker 2\nunconfirmed: Speaker 2\n---\n\n## Summary\n\n- **Speaker 2** — send deck\nSpeaker 2 will follow up; Speaker 20 won't.\n\n## Transcript\n\n**[00:01] Speaker 2:** Hi.\n\n**[00:03] Me:** Hello me.\n";
        let out = rename_in_note(md, "Speaker 2", "Sarah");
        assert!(out.contains("participants: Me, Speaker 1, Sarah\nunconfirmed: \n"));
        assert!(out.contains("- **Sarah** — send deck\nSarah will follow up; Speaker 20 won't."));
        assert!(out.contains("**[00:01] Sarah:** Hi."));
        let out = rename_in_note(&out, "Me", "Shaun");
        assert!(out.contains("**[00:03] Shaun:** Hello me."));
        assert!(out.contains("participants: Shaun, Speaker 1, Sarah"));
    }
}
