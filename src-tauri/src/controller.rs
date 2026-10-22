//! The app's brain: one thread that turns hotkey, pill, tray and job events into recordings,
//! transcriptions, pastes and notes. Slow work (STT, LLM, paste) runs on job threads that report back.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use earshot_core::audio::{Capture, Source, write_wav};
use earshot_core::segment::transcript_md;
use earshot_core::stt::{Engine, SR};
use earshot_core::{clipboard, hotkeys};
use earshot_overlay::menu::{Icon, MenuItem, ToastSpec};
use earshot_overlay::pill::{Accent, PillState};
use earshot_overlay::{Overlay, UiEvent};

use crate::config::Config;
use crate::llm::Pool;
use crate::meeting::{Meeting, summarize, worth_summarizing};
use crate::text::FORMAT_CTX;
use crate::{notes, text};

const TAP_MS: u64 = 300;
const DOUBLE_TAP: Duration = Duration::from_millis(600);
const MIN_AUDIO_S: f32 = 0.3;

pub enum AppEvent {
    Hotkey(hotkeys::Event),
    Ui(UiEvent),
    /// A tray or pill menu item id.
    Action(String),
    Job(JobDone),
}

pub enum JobDone {
    Dictated(String),
    Saved { title: String, body: String, path: PathBuf },
    Failed(String),
}

struct Dictation {
    cap: Capture,
    command: bool,
    hands_free: bool,
    /// Hands-free: the next hold's release stops it.
    stopping: bool,
    /// A short tap: becomes hands-free if a second press follows quickly, else it's discarded.
    tap_at: Option<Instant>,
}

enum Recording {
    Note { cap: Capture, started: DateTime<Local>, t0: Instant },
    Meeting { meeting: Meeting, started: DateTime<Local> },
}

/// What job threads need. Cheap to clone.
#[derive(Clone)]
struct Shared {
    cfg: Arc<Config>,
    engine: Arc<Engine>,
    pool: Pool,
    tx: Sender<AppEvent>,
}

pub struct Controller {
    sh: Shared,
    overlay: Overlay,
    app: tauri::AppHandle,
    dict: Option<Dictation>,
    rec: Option<Recording>,
    jobs: usize,
    flash: Option<(PillState, Instant)>,
    shown: PillState,
    last_text: Option<String>,
    next_toast: u64,
    toast_paths: HashMap<u64, PathBuf>,
    quitting: bool,
}

impl Controller {
    pub fn new(cfg: Config, engine: Arc<Engine>, pool: Pool, tx: Sender<AppEvent>, overlay: Overlay, app: tauri::AppHandle) -> Self {
        Self {
            sh: Shared { cfg: Arc::new(cfg), engine, pool, tx },
            overlay,
            app,
            dict: None,
            rec: None,
            jobs: 0,
            flash: None,
            shown: PillState::Idle,
            last_text: None,
            next_toast: 1,
            toast_paths: HashMap::new(),
            quitting: false,
        }
    }

    pub fn run(mut self, rx: Receiver<AppEvent>) {
        loop {
            let ev = match self.wait() {
                None => rx.recv().ok(),
                Some(t) => match rx.recv_timeout(t) {
                    Ok(ev) => Some(ev),
                    Err(RecvTimeoutError::Timeout) => {
                        self.tick();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                },
            };
            let Some(ev) = ev else { break };
            match ev {
                AppEvent::Hotkey(e) => self.on_hotkey(e),
                AppEvent::Ui(e) => self.on_ui(e),
                AppEvent::Action(a) => self.action(&a),
                AppEvent::Job(j) => self.on_job(j),
            }
            self.tick();
        }
    }

    /// How long to sleep: short while audio is live (level meter, timer), else until the next deadline.
    fn wait(&self) -> Option<Duration> {
        if self.dict.is_some() || self.rec.is_some() {
            return Some(Duration::from_millis(50));
        }
        self.flash.as_ref().map(|(_, until)| until.saturating_duration_since(Instant::now()) + Duration::from_millis(5))
    }

    fn tick(&mut self) {
        if let Some(d) = &self.dict {
            if d.tap_at.is_some_and(|t| t.elapsed() > DOUBLE_TAP) {
                drop(self.dict.take().unwrap().cap.stop());
            } else {
                self.overlay.set_level(d.cap.track.level());
            }
        } else if let Some(r) = &self.rec {
            let level = match r {
                Recording::Note { cap, .. } => cap.track.level(),
                Recording::Meeting { meeting, .. } => meeting.tracks().iter().map(|t| t.level()).fold(0.0, f32::max),
            };
            self.overlay.set_level(level);
        }
        if self.flash.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            self.flash = None;
        }
        let state = self.pill();
        if state != self.shown {
            self.overlay.set_state(state.clone());
            self.shown = state;
        }
        if self.quitting && self.jobs == 0 && self.rec.is_none() {
            self.overlay.quit();
            self.sh.pool.shutdown();
            self.app.exit(0);
        }
    }

    fn pill(&self) -> PillState {
        if let Some(d) = &self.dict {
            let accent = if d.command { Accent::Command } else { Accent::Dictate };
            return if d.hands_free { PillState::HandsFree(accent) } else { PillState::Listening(accent) };
        }
        if let Some(r) = &self.rec {
            let (accent, t0) = match r {
                Recording::Note { t0, .. } => (Accent::Note, *t0),
                Recording::Meeting { meeting, .. } => (Accent::Meeting, meeting.started),
            };
            return PillState::Recording { accent, secs: t0.elapsed().as_secs() as u32 };
        }
        if let Some((s, _)) = &self.flash {
            return s.clone();
        }
        if self.jobs > 0 { PillState::Thinking } else { PillState::Idle }
    }

    fn flash(&mut self, s: PillState) {
        let ms = if s == PillState::Error { 1600 } else { 900 };
        self.flash = Some((s, Instant::now() + Duration::from_millis(ms)));
    }

    // ---- dictation -------------------------------------------------------------------------

    fn on_hotkey(&mut self, e: hotkeys::Event) {
        match e {
            hotkeys::Event::HoldStart => match &mut self.dict {
                None => self.start_dictation(false),
                Some(d) if d.tap_at.is_some() => {
                    d.tap_at = None;
                    d.hands_free = true;
                }
                Some(d) if d.hands_free => d.stopping = true,
                Some(_) => {}
            },
            hotkeys::Event::HoldUpgrade => {
                if let Some(d) = &mut self.dict {
                    if !d.hands_free {
                        d.command = true;
                    }
                }
            }
            hotkeys::Event::HoldEnd { held_ms } => {
                let Some(d) = &mut self.dict else { return };
                if d.hands_free {
                    if d.stopping {
                        self.finish_dictation();
                    }
                } else if d.tap_at.is_none() && held_ms < TAP_MS {
                    if d.command {
                        drop(self.dict.take().unwrap().cap.stop());
                    } else {
                        d.tap_at = Some(Instant::now());
                    }
                } else {
                    self.finish_dictation();
                }
            }
            hotkeys::Event::Toggle(name) => match name.as_str() {
                "voice_note" => self.action("note"),
                "meeting" => self.action("meeting"),
                "paste_last" => self.action("paste_last"),
                "toggle_pill" => self.action("toggle"),
                _ => {}
            },
        }
    }

    fn start_dictation(&mut self, hands_free: bool) {
        match Capture::start(Source::Mic, Instant::now()) {
            Ok(cap) => {
                self.dict = Some(Dictation { cap, command: false, hands_free, stopping: false, tap_at: None });
                let cfg = &self.sh.cfg;
                let model = cfg.llm_model(&cfg.llm.format_model);
                if cfg.smart_format && self.sh.pool.available(&model) {
                    self.sh.pool.warm(&model, FORMAT_CTX);
                }
            }
            Err(e) => self.fail(format!("Microphone unavailable: {e}")),
        }
    }

    fn finish_dictation(&mut self) {
        let Some(d) = self.dict.take() else { return };
        let wave = d.cap.stop().read(0, None);
        if (wave.len() as f32) < SR as f32 * MIN_AUDIO_S {
            return;
        }
        let command = d.command;
        self.job(move |sh| {
            // Grab the selection first, while it's still selected.
            let selection = if command { clipboard::copy_selection() } else { None };
            let raw = sh.engine.transcribe(&wave)?;
            if raw.trim().is_empty() {
                return Ok(JobDone::Dictated(String::new()));
            }
            let out = if command { run_command(&sh, &raw, selection.as_deref())? } else { format_dictation(&sh, &raw) };
            if !out.is_empty() && !clipboard::paste(&out) {
                anyhow::bail!("Couldn't paste (clipboard busy)");
            }
            Ok(JobDone::Dictated(out))
        });
    }

    // ---- notes & meetings --------------------------------------------------------------------

    fn toggle_note(&mut self) {
        match self.rec.take() {
            None => match Capture::start(Source::Mic, Instant::now()) {
                Ok(cap) => self.rec = Some(Recording::Note { cap, started: Local::now(), t0: Instant::now() }),
                Err(e) => self.fail(format!("Microphone unavailable: {e}")),
            },
            Some(Recording::Note { cap, started, t0 }) => {
                let duration = t0.elapsed().as_secs_f32();
                let wave = cap.stop().read(0, None);
                self.job(move |sh| save_note(&sh, wave, started, duration));
            }
            Some(other) => {
                self.rec = Some(other);
                self.info("A meeting is recording", "Stop it before starting a voice note.");
            }
        }
    }

    fn toggle_meeting(&mut self) {
        match self.rec.take() {
            None => match Meeting::start(self.sh.engine.clone(), clipboard::foreground_app()) {
                Ok((meeting, warning)) => {
                    self.rec = Some(Recording::Meeting { meeting, started: Local::now() });
                    if let Some(w) = warning {
                        self.info("Recording your mic only", &w);
                    }
                }
                Err(e) => self.fail(format!("Couldn't start recording: {e}")),
            },
            Some(Recording::Meeting { meeting, started }) => {
                let with_summary = !self.quitting;
                self.job(move |sh| save_meeting(&sh, meeting, started, with_summary));
            }
            Some(other) => {
                self.rec = Some(other);
                self.info("A voice note is recording", "Stop it before starting a meeting.");
            }
        }
    }

    // ---- jobs, toasts, actions ---------------------------------------------------------------

    fn job(&mut self, f: impl FnOnce(Shared) -> Result<JobDone> + Send + 'static) {
        self.jobs += 1;
        let sh = self.sh.clone();
        std::thread::spawn(move || {
            let tx = sh.tx.clone();
            let done = f(sh).unwrap_or_else(|e| JobDone::Failed(format!("{e:#}")));
            let _ = tx.send(AppEvent::Job(done));
        });
    }

    fn on_job(&mut self, j: JobDone) {
        self.jobs = self.jobs.saturating_sub(1);
        match j {
            JobDone::Dictated(t) if t.is_empty() => {}
            JobDone::Dictated(t) => {
                self.last_text = Some(t);
                self.flash(PillState::Done);
            }
            JobDone::Saved { title, body, path } => {
                self.flash(PillState::Done);
                let id = self.toast(&title, &body, Icon::Check, Accent::Dictate, vec![("open", "Open")], 8.0);
                self.toast_paths.insert(id, path);
            }
            JobDone::Failed(e) => self.fail(e),
        }
    }

    fn toast(&mut self, title: &str, body: &str, icon: Icon, accent: Accent, buttons: Vec<(&str, &str)>, timeout_s: f32) -> u64 {
        let id = self.next_toast;
        self.next_toast += 1;
        self.overlay.toast(ToastSpec {
            id,
            title: title.into(),
            body: body.into(),
            icon,
            accent: accent.color(),
            buttons: buttons.into_iter().map(|(a, l)| (a.into(), l.into())).collect(),
            timeout_s,
        });
        id
    }

    fn info(&mut self, title: &str, body: &str) {
        self.toast(title, body, Icon::Mic, Accent::Dictate, Vec::new(), 6.0);
    }

    fn fail(&mut self, msg: String) {
        eprintln!("{msg}");
        self.flash(PillState::Error);
        self.toast("Something went wrong", &msg, Icon::Power, Accent::Meeting, Vec::new(), 8.0);
    }

    fn on_ui(&mut self, e: UiEvent) {
        match e {
            UiEvent::Click => match &self.dict {
                Some(d) if d.hands_free => self.finish_dictation(),
                Some(_) => {}
                None if self.rec.is_none() => self.start_dictation(true),
                None => {}
            },
            UiEvent::Stop => {
                if self.dict.as_ref().is_some_and(|d| d.hands_free) {
                    self.finish_dictation();
                } else {
                    match self.rec {
                        Some(Recording::Note { .. }) => self.toggle_note(),
                        Some(Recording::Meeting { .. }) => self.toggle_meeting(),
                        None => {}
                    }
                }
            }
            UiEvent::DoubleClick => self.action("open"),
            UiEvent::Moved { x, y } => self.update_cfg(|c| (c.pill.x, c.pill.y) = (Some(x), Some(y))),
            UiEvent::ContextMenu { x, y } => self.overlay.menu(self.menu_items(), x, y),
            UiEvent::Menu(id) => self.action(&id),
            UiEvent::ToastButton { id, action } => {
                if action == "open" {
                    if let Some(p) = self.toast_paths.remove(&id) {
                        open(&p);
                    }
                }
                self.overlay.close_toast(id);
            }
            UiEvent::ToastClosed { id } => {
                self.toast_paths.remove(&id);
            }
        }
    }

    pub fn action(&mut self, a: &str) {
        match a {
            "note" => self.toggle_note(),
            "meeting" => self.toggle_meeting(),
            "handsfree" => match &self.dict {
                None => self.start_dictation(true),
                Some(d) if d.hands_free => self.finish_dictation(),
                Some(_) => {}
            },
            "paste_last" => {
                if let Some(t) = self.last_text.clone() {
                    std::thread::spawn(move || clipboard::paste(&t));
                }
            }
            "toggle" | "hide" | "show" => {
                let visible = match a {
                    "hide" => false,
                    "show" => true,
                    _ => !self.overlay.is_visible(),
                };
                self.overlay.set_visible(visible);
                self.update_cfg(|c| c.pill.hidden = !visible);
            }
            "open" => {
                let dir = self.sh.cfg.notes_dir.clone();
                let _ = std::fs::create_dir_all(&dir);
                open(&dir);
            }
            "settings" => {
                if !crate::config::path().exists() {
                    self.sh.cfg.save();
                }
                let _ = std::process::Command::new("notepad").arg(crate::config::path()).spawn();
            }
            "quit" => {
                self.quitting = true;
                if let Some(d) = self.dict.take() {
                    drop(d.cap.stop());
                }
                match self.rec {
                    Some(Recording::Note { .. }) => self.toggle_note(),
                    Some(Recording::Meeting { .. }) => self.toggle_meeting(),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn update_cfg(&mut self, f: impl FnOnce(&mut Config)) {
        let mut cfg = (*self.sh.cfg).clone();
        f(&mut cfg);
        cfg.save();
        self.sh.cfg = Arc::new(cfg);
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        let hk = &self.sh.cfg.hotkeys;
        let note = matches!(self.rec, Some(Recording::Note { .. }));
        let meeting = matches!(self.rec, Some(Recording::Meeting { .. }));
        let item = |id: &str, label: &str, hint: &str, icon: Icon| MenuItem {
            id: id.into(),
            label: label.into(),
            hint: pretty_combo(hint),
            icon,
            separator_before: false,
            danger: false,
        };
        let mut items = vec![
            MenuItem { danger: note, ..item("note", if note { "Stop voice note" } else { "New voice note" }, &hk.voice_note, Icon::Notes) },
            MenuItem {
                danger: meeting,
                ..item("meeting", if meeting { "Stop meeting" } else { "Start meeting" }, &hk.meeting, if meeting { Icon::Record } else { Icon::People })
            },
            item("paste_last", "Paste last dictation", &hk.paste_last, Icon::Clipboard),
            item("handsfree", "Hands-free dictation", "", Icon::Lock),
            MenuItem { separator_before: true, ..item("open", "Open notes", "", Icon::Mic) },
            item("settings", "Settings", "", Icon::Gear),
            MenuItem { separator_before: true, ..item("hide", "Hide pill", &hk.toggle_pill, Icon::Hide) },
            item("quit", "Quit Earshot", "", Icon::Power),
        ];
        if self.last_text.is_none() {
            items.retain(|i| i.id != "paste_last");
        }
        items
    }
}

// ---- job bodies -----------------------------------------------------------------------------

fn format_dictation(sh: &Shared, raw: &str) -> String {
    text::smart_format(&sh.pool, &sh.cfg, raw, clipboard::foreground_app().as_deref()).0
}

fn run_command(sh: &Shared, instruction: &str, selection: Option<&str>) -> Result<String> {
    let cfg = &sh.cfg;
    let model = cfg.llm_model(&cfg.llm.format_model);
    anyhow::ensure!(sh.pool.available(&model), "Command mode needs the local model {}", model.display());
    let ctx = FORMAT_CTX.max(selection.map_or(0, |s| s.len() as u32 / 2 + 1024)).min(cfg.llm.ctx);
    let max_tokens = selection.map_or(512, |s| s.len() as u32 / 2 + 256);
    let out = sh.pool.chat(&model, ctx, text::COMMAND_SYSTEM, &text::command_user(instruction, selection), max_tokens, 0.3)?;
    Ok(text::tidy_llm(&out))
}

fn save_note(sh: &Shared, wave: Vec<f32>, started: DateTime<Local>, duration: f32) -> Result<JobDone> {
    let raw = sh.engine.transcribe(&wave)?;
    let body = notes::clean(&text::apply_dictionary(&raw, &sh.cfg.dictionary), true);
    anyhow::ensure!(!body.is_empty(), "Didn't catch anything in that voice note");
    let path = notes::new_note_path(&sh.cfg.notes_dir, "Voice notes", started);
    let mut extra = Vec::new();
    if sh.cfg.keep_wav {
        let wav = path.with_extension("wav");
        write_wav(&wav, &wave)?;
        extra.push(("audio", file_name(&wav)));
    }
    notes::write_note(&path, "voice-note", started, duration, &body, &extra).context("saving voice note")?;
    Ok(JobDone::Saved { title: "Voice note saved".into(), body: preview(&body), path })
}

fn save_meeting(sh: &Shared, meeting: Meeting, started: DateTime<Local>, with_summary: bool) -> Result<JobDone> {
    let app = meeting.app.clone();
    let done = meeting.finish();
    anyhow::ensure!(!done.lines.is_empty(), "No speech was recorded in that meeting");
    let cfg = &sh.cfg;
    let path = notes::new_note_path(&cfg.notes_dir, "Meetings", started);
    let mut extra = vec![("participants", if done.had_system { "Me, Others" } else { "Me" }.to_string())];
    if let Some(a) = &app {
        extra.push(("app", a.clone()));
    }
    if cfg.keep_wav {
        let wav = path.with_extension("wav");
        write_wav(&wav, &done.mix)?;
        extra.push(("audio", file_name(&wav)));
    }
    let title = format!("# Meeting · {}", started.format("%a %d %b %Y, %H:%M"));
    let transcript = transcript_md(&done.lines);
    let write = |summary: &str| {
        let body = format!("{title}\n\n## Summary\n\n{summary}\n\n## Transcript\n\n{transcript}");
        notes::write_note(&path, "meeting", started, done.duration_s, &body, &extra)
    };
    // Save the transcript first so nothing is lost if summarizing fails or the app quits.
    let model = cfg.llm_model(&cfg.llm.summary_model);
    let can_summarize = with_summary && worth_summarizing(&done.lines) && sh.pool.available(&model);
    write(if can_summarize { "_Summarizing…_" } else { "_Too short to summarize._" })?;
    let mut note = "Transcript saved.".to_string();
    if can_summarize {
        let head = format!(
            "Title: Meeting\nDate: {}\nDuration: {}\nParticipants: {}",
            started.format("%Y-%m-%d %H:%M"),
            notes::fmt_duration(done.duration_s),
            extra[0].1
        );
        match summarize(&sh.pool, &model, cfg.llm.ctx, &done.lines, &head) {
            Ok(s) => {
                write(&s)?;
                note = preview(&s.replace("### Summary", ""));
            }
            Err(e) => {
                write(&format!("_Summary failed: {e}_"))?;
                note = "Transcript saved; summary failed.".into();
            }
        }
    }
    Ok(JobDone::Saved { title: "Meeting notes saved".into(), body: note, path })
}

// ---- helpers ------------------------------------------------------------------------------

fn preview(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > 110 { format!("{}…", s.chars().take(110).collect::<String>().trim_end()) } else { s }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn open(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(p).spawn();
}

/// "ctrl+alt+n" -> "Ctrl+Alt+N"
pub fn pretty_combo(s: &str) -> String {
    s.split('+')
        .filter(|k| !k.is_empty())
        .map(|k| {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("+")
}
