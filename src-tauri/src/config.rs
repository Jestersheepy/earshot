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
            summary_model: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf".into(),
            format_model: "LFM2-1.2B-Q4_K_M.gguf".into(),
            ctx: 32768,
            threads: 8,
            keep_warm_min: 5,
        }
    }
}

/// Meeting summaries from a cloud model instead of the local one. The API key lives in Windows Credential
/// Manager (earshot_core::secret), keyed by provider, never in this file.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Cloud {
    pub enabled: bool,
    /// "anthropic" | "openai" | "custom" (any OpenAI-compatible server: OpenRouter, Groq, Ollama, LM Studio…)
    pub provider: String,
    /// Empty: the provider's default.
    pub model: String,
    /// Base URL for "custom", e.g. http://localhost:11434/v1
    pub url: String,
}

impl Default for Cloud {
    fn default() -> Self {
        Self { enabled: false, provider: "anthropic".into(), model: String::new(), url: String::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Pill {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub hidden: bool,
    /// "small" | "medium" | "large"
    pub size: String,
    /// "dot" | "mic" | "breathing"
    pub idle: String,
    /// "auto" (the Windows accent) or "#RRGGBB".
    pub accent: String,
    /// Grow into a hint bar on hover.
    pub peek: bool,
    /// What a left click does: "dictate" | "note" | "meeting" | "panel".
    pub click: String,
}

impl Default for Pill {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            hidden: false,
            size: "medium".into(),
            idle: "mic".into(),
            accent: "#14a38b".into(),
            peek: true,
            click: "dictate".into(),
        }
    }
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
    /// Depth of meeting summaries made when a meeting is saved.
    pub summary_mode: crate::summary::Mode,
    pub cloud: Cloud,
    /// Tell voices apart in meetings (Speaker 1, 2, … and voices you've named before).
    pub detect_speakers: bool,
    /// Your label in meeting transcripts.
    pub your_name: String,
    /// "system" | "light" | "dark"
    pub theme: String,
    /// A soft chime when listening starts and stops.
    pub sounds: bool,
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
            summary_mode: Default::default(),
            cloud: Cloud::default(),
            detect_speakers: true,
            your_name: "Me".into(),
            theme: "system".into(),
            sounds: true,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Earshot").join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let mut cfg: Self = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // v2.1 moved summaries to a stronger model; follow the default unless someone picked their own
        if cfg.llm.summary_model == "LFM2-2.6B-Transcript-Q4_K_M.gguf" {
            cfg.llm.summary_model = Llm::default().summary_model;
        }
        cfg
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_vec_pretty(self).unwrap());
    }

    /// %LOCALAPPDATA%\Earshot\models (see tools/install_models.py). Debug builds fall back to the repo's
    /// models/ so development works before installing.
    pub fn models_dir(&self) -> PathBuf {
        if let Some(d) = &self.models_dir {
            return d.clone();
        }
        if let Some(d) = std::env::var_os("EARSHOT_MODELS") {
            return d.into();
        }
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Earshot").join("models");
        #[cfg(debug_assertions)]
        if !local.join("bark-onnx").exists() {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("models");
        }
        local
    }

    /// The installer ships llama.cpp next to the app; development builds use the repo's models/llama.cpp.
    pub fn llama_server(&self) -> PathBuf {
        if let Some(p) = &self.llm.server {
            return p.clone();
        }
        let bundled = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("llama.cpp").join("llama-server.exe")));
        bundled.filter(|p| p.exists()).unwrap_or_else(|| self.models_dir().join("llama.cpp").join("llama-server.exe"))
    }

    pub fn speaker_dir(&self) -> PathBuf {
        self.models_dir().join("speaker")
    }

    pub fn llm_model(&self, name: &str) -> PathBuf {
        self.models_dir().join("llm").join(name)
    }
}
