//! Meeting segmentation: energy VAD over a growing track, mic echo gating, transcript assembly.
//! Port of earshot/meeting.py.

use std::sync::Arc;

use crate::audio::Track;
use crate::stt::SR;

const FRAME: usize = SR * 3 / 100;
const END_SILENCE_S: f32 = 1.2;
const MAX_UTT_S: f32 = 30.0;
const PRE_ROLL_S: f32 = 0.2;
const MERGE_GAP_S: f32 = 4.0; // a speaker's back-to-back utterances closer than this are one turn
const ECHO_RATIO: f32 = 0.5; // mic frame counts as echo when system audio is this much louder

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Half-duplex: silence mic frames while the other side is talking louder (speakers, no headphones).
pub fn gate_echo(mic: &[f32], reference: &[f32]) -> Vec<f32> {
    let mut out = mic.to_vec();
    for (i, frame) in out.chunks_exact_mut(FRAME).enumerate() {
        let r = reference.get(i * FRAME..(i + 1) * FRAME).map(rms).unwrap_or(0.0);
        if r > 0.01 && rms(frame) < r * ECHO_RATIO {
            frame.fill(0.0);
        }
    }
    out
}

pub struct Segmenter {
    track: Arc<Track>,
    reference: Option<Arc<Track>>,
    pos: usize,
    start: Option<usize>,
    silence: usize,
    noise: f32,
}

impl Segmenter {
    pub fn new(track: Arc<Track>, reference: Option<Arc<Track>>) -> Self {
        Self { track, reference, pos: 0, start: None, silence: 0, noise: 0.002 }
    }

    /// Track audio with mic echo gated out (when this is the mic and a reference exists).
    pub fn read(&self, a: usize, b: Option<usize>) -> Vec<f32> {
        let data = self.track.read(a, b);
        match &self.reference {
            Some(r) => gate_echo(&data, &r.read(a, Some(a + data.len()))),
            None => data,
        }
    }

    /// New finished utterances as (start, end) sample ranges.
    pub fn poll(&mut self, is_final: bool) -> Vec<(usize, usize)> {
        let data = self.read(self.pos, None);
        let mut out = Vec::new();
        let n = data.len() / FRAME;
        for i in 0..n {
            let e = rms(&data[i * FRAME..(i + 1) * FRAME]);
            let speech = e > (self.noise * 3.0).max(0.008);
            if !speech {
                self.noise = 0.98 * self.noise + 0.02 * e;
            }
            let p = self.pos + (i + 1) * FRAME;
            if speech && self.start.is_none() {
                self.start = Some(p.saturating_sub(FRAME + (PRE_ROLL_S * SR as f32) as usize));
            }
            if let Some(s) = self.start {
                self.silence = if speech { 0 } else { self.silence + FRAME };
                if self.silence as f32 >= END_SILENCE_S * SR as f32 || (p - s) as f32 >= MAX_UTT_S * SR as f32 {
                    out.push((s, p));
                    (self.start, self.silence) = (None, 0);
                }
            }
        }
        self.pos += n * FRAME;
        if is_final {
            if let Some(s) = self.start.take() {
                out.push((s, self.track.len()));
            }
        }
        out
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub start: f32,
    pub end: f32,
    pub speaker: String,
    pub text: String,
}

/// Without headphones the mic hears the call; drop "Me" lines that repeat an "Others" line.
pub fn drop_echo(lines: Vec<Line>) -> Vec<Line> {
    let others: Vec<&Line> = lines.iter().filter(|l| l.speaker == "Others").collect();
    let dup = |l: &Line| {
        l.speaker == "Me"
            && others.iter().any(|o| (l.start - o.start).abs() < 4.0 && similarity(&l.text, &o.text) > 0.6)
    };
    let keep: Vec<bool> = lines.iter().map(|l| !dup(l)).collect();
    lines.into_iter().zip(keep).filter_map(|(l, k)| k.then_some(l)).collect()
}

/// Join a speaker's back-to-back utterances into turns.
pub fn merge_turns(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for l in lines {
        if let Some(last) = out.last_mut() {
            if last.speaker == l.speaker && l.start - last.end < MERGE_GAP_S {
                let sep = if last.text.ends_with(['.', '?', '!', ',']) { " " } else { ". " };
                last.text = format!("{}{sep}{}", last.text, l.text);
                last.end = l.end;
                continue;
            }
        }
        out.push(l);
    }
    out
}

pub fn capitalize(s: &str) -> String {
    let s = s.trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Ratcliff/Obershelp-style ratio on lowercase characters (like Python's difflib, close enough here).
fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * matching(&a, &b) as f32 / (a.len() + b.len()) as f32
}

fn matching(a: &[char], b: &[char]) -> usize {
    // longest common substring, then recurse on both sides
    let (mut best, mut ai, mut bi) = (0, 0, 0);
    let mut prev = vec![0usize; b.len() + 1];
    for i in 0..a.len() {
        let mut cur = vec![0usize; b.len() + 1];
        for j in 0..b.len() {
            if a[i] == b[j] {
                cur[j + 1] = prev[j] + 1;
                if cur[j + 1] > best {
                    (best, ai, bi) = (cur[j + 1], i + 1 - cur[j + 1], j + 1 - cur[j + 1]);
                }
            }
        }
        prev = cur;
    }
    if best == 0 {
        return 0;
    }
    best + matching(&a[..ai], &b[..bi]) + matching(&a[ai + best..], &b[bi + best..])
}

pub fn transcript_md(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| {
            let s = l.start as u32;
            format!("**[{:02}:{:02}] {}:** {}", s / 60, s % 60, l.speaker, l.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
