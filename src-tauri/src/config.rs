//! Settings in %APPDATA%\Earshot\config.json (same file as v1; unknown keys are ignored, missing ones default).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Hotkeys {
    /// Hold to dictate; double-tap to lock hands-free.
    pub dictate: String,
    /// Hold (dictate + one more key) to edit the selected text by voice.
    pub command: String,
    pub voice_note: String,
    pub meeting: String,
    pub paste_last: String,
    pub toggle_pill: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            dictate: "ctrl+win".into(),
            command: "ctrl+win+alt".into(),
            voice_note: "ctrl+alt+n".into(),
            meeting: "ctrl+alt+m".into(),
            paste_last: "alt+shift+v".into(),
            toggle_pill: "ctrl+alt+space".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Llm {
    pub server: Option<PathBuf>,
    pub summary_model: String,
    pub format_model: String,
    pub ctx: u32,
    pub threads: u32,
    /// Minutes a model stays loaded after its last use.
    pub keep_warm_min: u64,
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            server: None,
            summary_model: "LFM2-2.6B-Transcript-Q4_K_M.gguf".into(),
            format_model: "LFM2-1.2B-Q4_K_M.gguf".into(),
            ctx: 32768,
            threads: 8,
            keep_warm_min: 5,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Pill {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub notes_dir: PathBuf,
    pub models_dir: Option<PathBuf>,
    pub hotkeys: Hotkeys,
    pub stt_threads: usize,
    pub idle_unload_min: u64,
    pub preload: bool,
    pub keep_wav: bool,
    /// Tidy dictation with the small local LLM (fillers, backtracking, lists).
    pub smart_format: bool,
    /// Apps (exe names) that get dictation pasted verbatim, no smart formatting.
    pub literal_apps: Vec<String>,
    pub llm: Llm,
    pub pill: Pill,
    /// Names and jargon: fix their spelling and tell the formatter about them.
    pub dictionary: Vec<String>,
    pub snippets: Vec<Snippet>,
    /// Meeting detection per app: "ask" | "always" | "never". Unlisted apps: ask.
    pub meeting_apps: BTreeMap<String, String>,
    pub detect_meetings: bool,
}

impl Default for Config {
    fn default() -> Self {
        let docs = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Documents");
        Self {
            notes_dir: docs.join("Earshot"),
            models_dir: None,
            hotkeys: Hotkeys::default(),
            stt_threads: 8,
            idle_unload_min: 15,
            preload: true,
            keep_wav: true,
            smart_format: true,
            literal_apps: ["code.exe", "windowsterminal.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "cursor.exe"]
                .map(String::from)
                .to_vec(),
            llm: Llm::default(),
            pill: Pill::default(),
            dictionary: Vec::new(),
            snippets: Vec::new(),
            meeting_apps: BTreeMap::new(),
            detect_meetings: true,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Earshot").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_vec_pretty(self).unwrap());
    }

    /// %LOCALAPPDATA%\Earshot\models, or the repo's models/ during development.
    pub fn models_dir(&self) -> PathBuf {
        if let Some(d) = &self.models_dir {
            return d.clone();
        }
        if let Some(d) = std::env::var_os("EARSHOT_MODELS") {
            return d.into();
        }
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Earshot").join("models");
        if local.join("bark-onnx").exists() {
            return local;
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("models")
    }

    pub fn llama_server(&self) -> PathBuf {
        self.llm.server.clone().unwrap_or_else(|| self.models_dir().join("llama.cpp").join("llama-server.exe"))
    }

    pub fn llm_model(&self, name: &str) -> PathBuf {
        self.models_dir().join("llm").join(name)
    }
}
