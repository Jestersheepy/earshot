"""Put the models the app needs at runtime into %LOCALAPPDATA%\\Earshot\\models.

    python tools/install_models.py

Files are hard-linked from the repo's models/ when both are on the same drive (no extra disk space; the repo
copies stay usable for the golden test and the format spike), copied otherwise.
"""
import os
import shutil
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / "models"
DST = Path(os.environ["LOCALAPPDATA"]) / "Earshot" / "models"

RUNTIME = [
    "bark-onnx/encoder.int8.onnx",
    "bark-onnx/decoder.safetensors",
    "bark-onnx/config.json",
    "bark-onnx/NOTICE",
    "llm/LFM2-2.6B-Transcript-Q4_K_M.gguf",  # meeting summaries
    "llm/LFM2-1.2B-Q4_K_M.gguf",  # smart formatting, command mode
    "onnxruntime.dll",
]


def place(src: Path, dst: Path) -> str:
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.exists():
        if dst.stat().st_size == src.stat().st_size:
            return "ok"
        dst.unlink()
    try:
        os.link(src, dst)
        return "linked"
    except OSError:
        shutil.copy2(src, dst)
        return "copied"


def main():
    files = [SRC / f for f in RUNTIME] + sorted((SRC / "llama.cpp").glob("*"))
    missing = [f for f in files if not f.is_file()]
    if missing:
        sys.exit("missing (run tools/download_models.py and tools/export_bark_onnx.py first):\n  "
                 + "\n  ".join(str(m) for m in missing))
    for f in files:
        rel = f.relative_to(SRC)
        print(f"{place(f, DST / rel):7} {rel}")
    print(f"\nModels ready in {DST}")


if __name__ == "__main__":
    main()
