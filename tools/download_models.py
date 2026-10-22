"""Download everything Earshot needs into ./models (one-time, the only network use).

    python tools/download_models.py            # Bark source + ONNX Runtime + llama.cpp (CPU) + Qwen3-4B-Instruct + LFM2-1.2B (Q4_K_M)
    python tools/download_models.py --no-llm   # skip the summarizer (~1.7 GB)

The Bark ONNX export itself is a separate step (needs torch once): see README.
"""
import json
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

MODELS = Path(__file__).resolve().parent.parent / "models"
BARK = "https://huggingface.co/ConwayResearch/Underdog-Bark-3.0-Plus/resolve/main/"
BARK_FILES = ["model.bark", "bark_loader.py", "NOTICE"]
SPEAKER = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx"
LLMS = [
    "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",  # summaries
    "https://huggingface.co/LiquidAI/LFM2-1.2B-GGUF/resolve/main/LFM2-1.2B-Q4_K_M.gguf",  # smart formatting, command mode
]


def fetch(url, dest):
    dest = Path(dest)
    if dest.exists():
        print("have", dest.name)
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    print("get ", url)
    tmp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f, 1 << 20)
    tmp.rename(dest)
    return dest


def llama_cpp():
    out = MODELS / "llama.cpp"
    if (out / "llama-server.exe").exists():
        print("have llama-server.exe")
        return
    # "latest" isn't always a bNNNN build with binaries, so take the newest release that has the CPU zip
    rels = json.load(urllib.request.urlopen("https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10"))
    asset = next(a for r in rels for a in r["assets"] if a["name"].endswith("bin-win-cpu-x64.zip"))
    z = fetch(asset["browser_download_url"], MODELS / asset["name"])
    with zipfile.ZipFile(z) as zf:
        zf.extractall(out)
    z.unlink()


def onnxruntime_dll():
    """The app loads onnxruntime.dll at runtime; take it from the onnxruntime wheel in this venv."""
    import onnxruntime

    if (MODELS / "onnxruntime.dll").exists():
        print("have onnxruntime.dll")
        return
    src = Path(onnxruntime.__file__).parent / "capi" / "onnxruntime.dll"
    shutil.copy2(src, MODELS / "onnxruntime.dll")
    print("got  onnxruntime.dll", onnxruntime.__version__)


if __name__ == "__main__":
    MODELS.mkdir(exist_ok=True)
    onnxruntime_dll()
    fetch(SPEAKER, MODELS / "speaker" / Path(SPEAKER).name)
    for f in BARK_FILES:
        fetch(BARK + f, MODELS / "bark-src" / f)
    if "--no-llm" not in sys.argv:
        llama_cpp()
        for url in LLMS:
            fetch(url, MODELS / "llm" / Path(url).name)
