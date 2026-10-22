# Earshot

A small floating circle for Windows that keeps your voice on your machine:

- **Dictate**: hold **Ctrl+Win**, speak, then release. The text is pasted into whatever field has focus.
- **Voice note**: press **Ctrl+Alt+N** (or click the circle) to start, and press it again to stop. The note is saved as Markdown, with the audio alongside.
- **Meeting notes**: press **Ctrl+Alt+M** to start and stop. Your mic is recorded as **Me** and the system audio (the call) as **Others**. You get a timestamped transcript and a local summary with decisions and action items.
- **The circle**: drag it anywhere. Right-click for the menu, double-click to hide it to the tray, and press **Ctrl+Alt+Space** to show or hide it.

Everything runs on the CPU with no network calls. Only the one-time model download needs the internet.

| Part | What | Disk | RAM |
|---|---|---|---|
| Speech-to-text | [Underdog Bark 3.0 Plus](https://huggingface.co/ConwayResearch/Underdog-Bark-3.0-Plus), exported to int8 ONNX, with a numpy TDT decoder | 890 MB | about 1 GB while loaded; unloaded after 15 minutes idle |
| Summaries | [LFM2-2.6B-Transcript](https://huggingface.co/LiquidAI/LFM2-2.6B-Transcript) Q4_K_M through llama.cpp's `llama-server` (CPU) | 1.6 GB | about 2–3 GB, only while summarizing |
| App | Python + tkinter + onnxruntime + numpy | ~100 MB | – |

Speed on a Ryzen 9 7950X3D:
- Dictation: 16 s of speech transcribes in about 0.3 s.
- Meetings: a 90 s meeting is fully transcribed in about 3.5 s, and the summary takes about 12 s.

## Setup

```bash
uv venv -p 3.12 .venv
uv pip install -p .venv -e .
.venv/Scripts/python tools/download_models.py
```

**One-time Bark export.** This step needs torch once; the app never uses it. Run it in a throwaway venv you can delete afterwards:

```bash
uv venv -p 3.11 .venv-export
uv pip install -p .venv-export --index-url https://download.pytorch.org/whl/cpu --extra-index-url https://pypi.org/simple torch==2.8.0 transformers==5.19.0 constriction==0.5.0 numpy soundfile soxr librosa==0.11.0 onnx onnxruntime
.venv-export/Scripts/python tools/export_bark_onnx.py models/bark-src models/bark-onnx
```

## Run

```bash
.venv/Scripts/pythonw -m earshot
```

To start it with Windows, run this once:

```bash
.venv/Scripts/python -m earshot --install-startup
```

- Settings are in `%APPDATA%\Earshot\config.json`: hotkeys, notes folder (default `Documents\Earshot`), threads, idle unload, and the LLM model path.
- Notes are saved to `Voice Notes/` and `Meetings/` as `YYYY-MM-DD HHMM.md`, which works with Obsidian.

## Checks

- `python -m earshot.stt.engine file.wav` transcribes a file and reports timings.
- `python -m earshot.summarize meeting.md` re-summarizes a saved meeting.
- `python tools/selftest.py <dir of NN_Me.wav / NN_Others.wav>` runs an offline meeting pipeline test, including mic echo of the other side.

## Notes and limits

- Bark is English only.
- Without headphones, the mic hears the call. Earshot mutes mic frames while the other side is louder, and drops "Me" lines that repeat an "Others" line.
- Elevated (admin) windows only accept pasted text if Earshot also runs elevated.
- Only text clipboard content is restored after a dictation paste.
- Licenses: Bark is CC-BY-4.0 (see `models/bark-onnx/NOTICE`). LFM2 uses the LFM Open License v1.0.
