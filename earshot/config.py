"""Settings in %APPDATA%\\Earshot\\config.json. Missing keys fall back to DEFAULTS."""
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS_DIR = ROOT / "models"
CONFIG_PATH = Path(os.environ.get("APPDATA", Path.home())) / "Earshot" / "config.json"

DEFAULTS = {
    "notes_dir": str(Path.home() / "Documents" / "Earshot"),
    "hotkeys": {
        "dictate": "ctrl+windows",  # hold
        "voice_note": "ctrl+alt+n",  # toggle
        "meeting": "ctrl+alt+m",  # toggle
        "toggle_circle": "ctrl+alt+space",
    },
    "stt_model_dir": str(MODELS_DIR / "bark-onnx"),
    "stt_threads": 8,
    "idle_unload_min": 15,
    "preload": True,
    "keep_wav": True,
    "drop_fillers": True,
    "llm": {
        "server": str(MODELS_DIR / "llama.cpp" / "llama-server.exe"),
        "model": str(MODELS_DIR / "llm" / "LFM2-2.6B-Transcript-Q4_K_M.gguf"),
        "ctx": 32768,
        "threads": 8,
        "port": 8737,
    },
    "circle": {"x": None, "y": None, "size": 56, "visible": True},
}


def _merge(base, over):
    out = dict(base)
    for k, v in over.items():
        out[k] = _merge(base[k], v) if isinstance(v, dict) and isinstance(base.get(k), dict) else v
    return out


def load():
    try:
        return _merge(DEFAULTS, json.loads(CONFIG_PATH.read_text()))
    except (OSError, ValueError):
        return _merge(DEFAULTS, {})


def save(cfg):
    CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(cfg, indent=2))
