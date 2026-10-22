"""Speaker embeddings + clustering, the reference the Rust port (crates/core/src/speakers.rs) is checked against.

Kaldi-compatible 80-bin fbank (what the sherpa-onnx speaker models expect), an ONNX embedding model, and
average-linkage agglomerative clustering on cosine distance.

    python tools/reference_speakers.py models/speakers_test/*.wav     # similarity matrix + clusters
"""
import sys
import wave
from pathlib import Path

import numpy as np

SR = 16000
FRAME, SHIFT, NFFT, BINS = 400, 160, 512, 80


def mel_banks():
    mel = lambda f: 1127.0 * np.log(1.0 + f / 700.0)
    lo, hi = mel(20.0), mel(SR / 2)
    centers = np.linspace(lo, hi, BINS + 2)
    fft_mel = mel(np.arange(NFFT // 2) * SR / NFFT)
    banks = np.zeros((BINS, NFFT // 2 + 1), np.float32)
    for b in range(BINS):
        left, center, right = centers[b : b + 3]
        up = (fft_mel - left) / (center - left)
        down = (right - fft_mel) / (right - center)
        banks[b, : NFFT // 2] = np.maximum(0.0, np.minimum(up, down))
    return banks


BANKS = mel_banks()
WINDOW = (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(FRAME) / (FRAME - 1))) ** 0.85


def fbank(pcm: np.ndarray) -> np.ndarray:
    """pcm: float32 in [-1, 1] at 16 kHz -> (frames, 80) log mel energies, mean-normalised per bin."""
    n = 1 + (len(pcm) - FRAME) // SHIFT
    if n < 1:
        return np.zeros((0, BINS), np.float32)
    idx = np.arange(FRAME)[None, :] + SHIFT * np.arange(n)[:, None]
    frames = pcm[idx].astype(np.float64)
    frames -= frames.mean(axis=1, keepdims=True)
    frames[:, 1:] -= 0.97 * frames[:, :-1].copy()
    frames[:, 0] -= 0.97 * frames[:, 0]
    frames *= WINDOW
    power = np.abs(np.fft.rfft(frames, NFFT)) ** 2
    feats = np.log(np.maximum(power @ BANKS.T, np.finfo(np.float32).eps)).astype(np.float32)
    return feats - feats.mean(axis=0, keepdims=True)


class Embedder:
    def __init__(self, path):
        import onnxruntime as ort

        self.s = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
        self.input = self.s.get_inputs()[0].name

    def __call__(self, pcm):
        e = self.s.run(None, {self.input: fbank(pcm)[None]})[0][0]
        return e / np.linalg.norm(e)


def cluster(embs: np.ndarray, threshold: float) -> list[int]:
    """Average-linkage agglomerative clustering; merge while cosine similarity > threshold."""
    groups = [[i] for i in range(len(embs))]
    sim = embs @ embs.T
    while len(groups) > 1:
        best, pair = -1.0, None
        for a in range(len(groups)):
            for b in range(a + 1, len(groups)):
                s = sim[np.ix_(groups[a], groups[b])].mean()
                if s > best:
                    best, pair = s, (a, b)
        if best < threshold:
            break
        a, b = pair
        groups[a] += groups.pop(b)
    labels = [0] * len(embs)
    for k, g in enumerate(sorted(groups, key=lambda g: min(g))):
        for i in g:
            labels[i] = k
    return labels


def load(p):
    with wave.open(str(p)) as f:
        assert f.getframerate() == SR and f.getnchannels() == 1
        return np.frombuffer(f.readframes(f.getnframes()), np.int16).astype(np.float32) / 32768


if __name__ == "__main__":
    models = Path(__file__).resolve().parent.parent / "models" / "speaker"
    wavs = [Path(a) for a in sys.argv[1:]]
    names = [w.stem.split("_")[-1] for w in wavs]
    for m in sorted(models.glob("*.onnx")):
        emb = Embedder(m)
        E = np.stack([emb(load(w)) for w in wavs])
        S = E @ E.T
        same = [S[i, j] for i in range(len(wavs)) for j in range(i + 1, len(wavs)) if names[i] == names[j]]
        diff = [S[i, j] for i in range(len(wavs)) for j in range(i + 1, len(wavs)) if names[i] != names[j]]
        print(f"\n{m.name}: same-speaker sim min {min(same):.3f} mean {np.mean(same):.3f}; "
              f"different max {max(diff):.3f} mean {np.mean(diff):.3f}")
        for t in (0.3, 0.4, 0.5, 0.6):
            print(f"  threshold {t}: {cluster(E, t)}")
    print("truth:", names)
