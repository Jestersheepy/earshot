# Earshot

Local dictation, voice notes and meeting notes for Windows. A small floating pill that keeps your voice on your machine.

- **Dictate**: hold **Ctrl+Win**, speak, release. The text is pasted wherever your cursor is, tidied by a small local model (fillers, "no wait, Thursday", spoken lists). Double-tap to keep it listening hands-free.
- **Command mode**: select text, hold **Ctrl+Win+Alt** and say what to do ("make this friendlier", "turn this into bullets"). With nothing selected, it writes new text.
- **Paste last**: **Alt+Shift+V** pastes your latest dictation again. Every dictation is kept in History.
- **Voice note**: **Ctrl+Alt+N** (or click the pill) to start and stop. Saved as Markdown with the audio alongside.
- **Meeting notes**: **Ctrl+Alt+M**, or say Record when Earshot spots Zoom, Teams, Slack, Discord, Webex or a browser call using your mic. Your mic is **Me**, the system audio is **Others**. You get a timestamped transcript and a local summary with decisions and action items; recording stops by itself when the call ends.
- **Earshot window**: double-click the pill (or use the tray) to browse and search notes, play back audio from any line, see dictation history, and change settings, the dictionary and snippets.

Everything runs on the CPU with no network calls. Only the one-time model download needs the internet.

| Part | What | Disk | RAM |
|---|---|---|---|
| Speech-to-text | [Underdog Bark 3.0 Plus](https://huggingface.co/ConwayResearch/Underdog-Bark-3.0-Plus), int8 ONNX encoder, Rust TDT decoder | 890 MB | about 900 MB while loaded; unloaded after 15 minutes idle |
| Formatting, command mode | [LFM2-1.2B](https://huggingface.co/LiquidAI/LFM2-1.2B-GGUF) Q4_K_M through llama.cpp's `llama-server` | 730 MB | about 1 GB, kept warm 5 minutes after use |
| Summaries | [LFM2-2.6B-Transcript](https://huggingface.co/LiquidAI/LFM2-2.6B-Transcript) Q4_K_M, same server | 1.6 GB | about 2–3 GB, only while summarizing |
| App | Rust + Tauri; the pill, menu and pop-ups are drawn natively, the window is WebView2 only while open | ~20 MB | about 30 MB idle |

## Install

1. Get the models (one-time, see below), then put them where the app looks:

   ```bash
   .venv/Scripts/python tools/install_models.py
   ```

   This hard-links (or copies) the runtime files into `%LOCALAPPDATA%\Earshot\models`.

2. Run `Earshot_<version>_x64-setup.exe` from `target/release/bundle/nsis/`. It installs per user (no admin), adds a Start menu entry, and Earshot starts with Windows (turn that off in Settings).

## Models

```bash
uv venv -p 3.12 .venv
uv pip install -p .venv -r tools/requirements.txt
.venv/Scripts/python tools/download_models.py
```

**One-time Bark export.** This needs torch once; the app never uses it. Run it in a throwaway venv you can delete afterwards:

```bash
uv venv -p 3.11 .venv-export
uv pip install -p .venv-export --index-url https://download.pytorch.org/whl/cpu --extra-index-url https://pypi.org/simple torch==2.8.0 transformers==5.19.0 constriction==0.5.0 numpy soundfile soxr librosa==0.11.0 onnx onnxruntime safetensors
.venv-export/Scripts/python tools/export_bark_onnx.py models/bark-src models/bark-onnx
```

## Build

Needs Rust (stable MSVC), Node and pnpm.

```bash
pnpm install
pnpm tauri dev                     # debug build; uses ./models if AppData has none
pnpm tauri build                   # installer in target/release/bundle/nsis/
```

Checks:

- `cargo test -p earshot-core` includes the golden test: the Rust transcripts must match `tools/reference_stt.py` exactly (`tools/make_golden.py` writes the reference).
- `cargo test -p earshot --lib` covers formatting, history and the notes library; `pnpm check` type-checks the window.
- `cargo run -p earshot --example format_spike --release` scores formatting models on `tools/format_cases.json`.

## Files

- Settings: `%APPDATA%\Earshot\config.json` (edit them in the window).
- Notes: `Documents\Earshot\Voice Notes` and `Meetings`, as `YYYY-MM-DD HHMM.md` plus audio. They work with Obsidian.
- Dictation history: `%APPDATA%\Earshot\history.db`.

## Notes and limits

- Bark is English only.
- Without headphones, the mic hears the call. Earshot mutes mic frames while the other side is louder, and drops "Me" lines that repeat an "Others" line.
- Elevated (admin) windows only accept pasted text if Earshot also runs elevated.
- Only text clipboard content is restored after a dictation paste.
