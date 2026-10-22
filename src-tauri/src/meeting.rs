//! A meeting recording: mic ("Me") + system audio ("Others") on separate tracks, transcribed utterance
//! by utterance while it runs so the transcript is ready almost as soon as it stops.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use earshot_core::audio::{Capture, Source, Track};
use earshot_core::segment::{Line, Segmenter, capitalize, drop_echo, merge_turns};
use earshot_core::stt::{Engine, SR};
use parking_lot::Mutex;

pub struct Meeting {
    mic: Capture,
    system: Option<Capture>,
    lines: Arc<Mutex<Vec<Line>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub started: Instant,
    pub app: Option<String>,
}

pub struct Finished {
    pub duration_s: f32,
    pub lines: Vec<Line>,
    pub mix: Vec<f32>,
    pub had_system: bool,
}

impl Meeting {
    /// Returns the meeting and a warning if system audio couldn't be captured.
    pub fn start(engine: Arc<Engine>, app: Option<String>) -> Result<(Self, Option<String>)> {
        let t0 = Instant::now();
        let mic = Capture::start(Source::Mic, t0)?;
        let (system, warning) = match Capture::start(Source::Loopback, t0) {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(format!("System audio unavailable ({e}); recording your mic only."))),
        };
        let me = mic.track.clone();
        let others = system.as_ref().map(|c| c.track.clone());
        let mut segs = vec![(Segmenter::new(me, others.clone()), "Me")];
        if let Some(o) = others {
            segs.push((Segmenter::new(o, None), "Others"));
        }
        let (lines, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (l, s) = (lines.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(800));
                drain(&mut segs, &engine, &l, false);
            }
            drain(&mut segs, &engine, &l, true);
        });
        Ok((Self { mic, system, lines, stop, worker: Some(worker), started: t0, app }, warning))
    }

    pub fn tracks(&self) -> Vec<Arc<Track>> {
        std::iter::once(self.mic.track.clone()).chain(self.system.as_ref().map(|c| c.track.clone())).collect()
    }

    pub fn finish(mut self) -> Finished {
        let duration_s = self.started.elapsed().as_secs_f32();
        let me = self.mic.track.clone();
        let others = self.system.as_ref().map(|c| c.track.clone());
        drop(std::mem::replace(&mut self.system, None).map(Capture::stop));
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        let mut lines = std::mem::take(&mut *self.lines.lock());
        lines.sort_by(|a, b| a.start.total_cmp(&b.start));
        let lines = merge_turns(drop_echo(lines));
        let mut mix = me.read(0, None);
        if let Some(o) = &others {
            let o = o.read(0, None);
            if o.len() > mix.len() {
                mix.resize(o.len(), 0.0);
            }
            for (m, v) in mix.iter_mut().zip(o) {
                *m = (*m + v).clamp(-1.0, 1.0);
            }
        }
        Finished { duration_s, lines, mix, had_system: others.is_some() }
    }
}

fn drain(segs: &mut [(Segmenter, &str)], engine: &Engine, lines: &Mutex<Vec<Line>>, last: bool) {
    for (seg, speaker) in segs.iter_mut() {
        for (a, b) in seg.poll(last) {
            match engine.transcribe(&seg.read(a, Some(b))) {
                Ok(t) if !t.trim().is_empty() => lines.lock().push(Line {
                    start: a as f32 / SR as f32,
                    end: b as f32 / SR as f32,
                    speaker: speaker.to_string(),
                    text: capitalize(&t),
                }),
                Ok(_) => {}
                Err(e) => eprintln!("meeting transcription failed: {e}"),
            }
        }
    }
}

pub fn worth_summarizing(lines: &[Line]) -> bool {
    lines.iter().map(|l| l.text.split_whitespace().count()).sum::<usize>() >= crate::text::SUMMARY_MIN_WORDS
}

/// Executive summary, decisions and action items. Long transcripts are summarized in parts first.
pub fn summarize(pool: &crate::llm::Pool, model: &std::path::Path, ctx: u32, lines: &[Line], head: &str) -> Result<String> {
    use crate::text::{SUMMARY_ASK, SUMMARY_PART, SUMMARY_SYSTEM};
    let transcript: Vec<String> = lines.iter().map(|l| format!("**{}**: {}", l.speaker, l.text)).collect();
    let budget = (ctx as f32 * 0.6) as usize * 3; // ~3 chars/token; room for the prompt and the answer
    let mut parts: Vec<Vec<&str>> = vec![Vec::new()];
    let mut size = 0;
    for line in &transcript {
        if size + line.len() > budget && !parts.last().unwrap().is_empty() {
            parts.push(Vec::new());
            size = 0;
        }
        parts.last_mut().unwrap().push(line);
        size += line.len() + 1;
    }
    let ask = |q: &str, body: &str| pool.chat(model, ctx, SUMMARY_SYSTEM, &format!("{q}\n\n{head}\n----------\n{body}"), 1024, 0.3);
    if parts.len() == 1 {
        return ask(SUMMARY_ASK, &parts[0].join("\n"));
    }
    let partials = parts.iter().map(|p| ask(SUMMARY_PART, &p.join("\n"))).collect::<Result<Vec<_>>>()?;
    let merged = partials.iter().enumerate().map(|(i, p)| format!("Part {}:\n{p}", i + 1)).collect::<Vec<_>>().join("\n\n");
    ask(&format!("{SUMMARY_ASK}\n\nThe transcript was long, so here are summaries of its parts in order."), &merged)
}
