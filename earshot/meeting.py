"""Meeting capture: mic ("Me") + system audio ("Others") as separate tracks, transcribed utterance by
utterance while the meeting runs, so the transcript is ready almost as soon as you stop."""
import threading
import time
from difflib import SequenceMatcher

import numpy as np

from earshot.audio import Loopback, Mic
from earshot.stt.engine import SR

FRAME = int(0.03 * SR)
END_SILENCE_S, MAX_UTT_S, PRE_ROLL_S = 1.2, 30.0, 0.2
MERGE_GAP_S = 4.0  # consecutive lines from the same speaker closer than this read as one turn


ECHO_RATIO = 0.5  # mic frame counts as echo when system audio is this much louder


def frame_rms(wave):
    n = len(wave) // FRAME
    return np.sqrt(np.mean(wave[: n * FRAME].reshape(n, FRAME) ** 2, axis=1))


def gate_echo(mic, ref):
    """Half-duplex: silence mic frames while the other side is talking louder (speakers, no headphones)."""
    ref = np.pad(ref, (0, max(0, len(mic) - len(ref))))[: len(mic)]
    m, r = frame_rms(mic), frame_rms(ref)
    out = mic.copy()
    for i in np.nonzero((r > 0.01) & (m < r * ECHO_RATIO))[0]:
        out[i * FRAME:(i + 1) * FRAME] = 0
    return out


class Segmenter:
    """Energy VAD over a growing track: yields (start, end) sample ranges of speech.
    With `ref` (the system-audio track), mic echo of the other side is gated out first."""

    def __init__(self, track, ref=None):
        self.track, self.ref = track, ref
        self.pos, self.start, self.silence, self.noise = 0, None, 0, 0.002

    def read(self, a, b=None):
        data = self.track.read(a, b)
        return gate_echo(data, self.ref.read(a, a + len(data))) if self.ref else data

    def poll(self, final=False):
        data = self.read(self.pos)
        out = []
        for i in range(len(data) // FRAME):
            e = float(np.sqrt(np.mean(data[i * FRAME:(i + 1) * FRAME] ** 2)))
            speech = e > max(0.008, self.noise * 3)
            if not speech:
                self.noise = 0.98 * self.noise + 0.02 * e
            p = self.pos + (i + 1) * FRAME
            if speech and self.start is None:
                self.start = max(0, p - FRAME - int(PRE_ROLL_S * SR))
            if self.start is not None:
                self.silence = 0 if speech else self.silence + FRAME
                if self.silence >= END_SILENCE_S * SR or p - self.start >= MAX_UTT_S * SR:
                    out.append((self.start, p))
                    self.start, self.silence = None, 0
        self.pos += len(data) // FRAME * FRAME
        if final and self.start is not None:
            out.append((self.start, len(self.track)))
            self.start = None
        return out


class Meeting:
    def __init__(self, engine, on_error=print):
        self.engine, self.on_error = engine, on_error
        self.mic, self.loop = Mic(), Loopback()
        self.lines = []  # (start_s, speaker, text)
        self._stop = threading.Event()

    def start(self):
        me = self.mic.start()
        try:
            others = self.loop.start(t0=me.t0)
        except Exception as e:  # no output device etc.: carry on with the mic only
            self.on_error(f"System audio unavailable ({e}); recording mic only.")
            others = None
        self.started = time.monotonic()
        self.segs = [(Segmenter(me, ref=others), "Me", me)]
        if others:
            self.segs.append((Segmenter(others), "Others", others))
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._worker.start()
        return me

    def level(self):
        return max(t.level for _, _, t in self.segs)

    def _run(self):
        while not self._stop.wait(1.0):
            self._drain(final=False)

    def _drain(self, final):
        for seg, speaker, _ in self.segs:
            for a, b in seg.poll(final):
                text = clean(self.engine.transcribe(seg.read(a, b)))
                if text:
                    self.lines.append((a / SR, b / SR, speaker, text))

    def stop(self):
        """Stops capture and returns (duration_s, lines, me_track, others_track)."""
        self.mic.stop()
        self.loop.stop()
        self._stop.set()
        self._worker.join()
        self._drain(final=True)
        lines = merge_turns(drop_echo(sorted(self.lines)))
        tracks = [t for _, _, t in self.segs]
        return time.monotonic() - self.started, lines, tracks[0], tracks[1] if len(tracks) > 1 else None


def drop_echo(lines, window_s=4.0, similar=0.6):
    """Without headphones the mic hears the speakers too; drop 'Me' lines that repeat an 'Others' line.
    lines: sorted (start_s, end_s, speaker, text)."""
    others = [(a, x) for a, _, s, x in lines if s == "Others"]
    keep = []
    for a, b, s, x in lines:
        if s == "Me" and any(abs(a - oa) < window_s and SequenceMatcher(None, x.lower(), ox.lower()).ratio() > similar
                             for oa, ox in others):
            continue
        keep.append((a, b, s, x))
    return keep


def merge_turns(lines):
    """(start, end, speaker, text) -> (start, speaker, text), joining a speaker's back-to-back utterances."""
    out = []
    for a, b, s, x in lines:
        if out and out[-1][2] == s and a - out[-1][1] < MERGE_GAP_S:
            pa, _, _, px = out[-1]
            out[-1] = (pa, b, s, f"{px}{'' if px[-1:] in '.?!,' else '.'} {x}")
        else:
            out.append((a, b, s, x))
    return [(a, s, x) for a, _, s, x in out]


def clean(text):
    text = text.strip()
    return text[:1].upper() + text[1:]


def transcript_md(lines):
    out = []
    for t, s, x in lines:
        m, sec = divmod(int(t), 60)
        out.append(f"**[{m:02d}:{sec:02d}] {s}:** {x}")
    return "\n\n".join(out)


def mix(a, b):
    if b is None:
        return a.read()
    x, y = a.read(), b.read()
    n = max(len(x), len(y))
    return np.clip(np.pad(x, (0, n - len(x))) + np.pad(y, (0, n - len(y))), -1, 1)
