"""Mic and system-audio (WASAPI loopback) capture into time-aligned 16 kHz mono buffers."""
import threading
import time

import numpy as np

from earshot.stt.engine import SR, to_16k_mono


class Track:
    """Thread-safe growing buffer. Gaps (loopback sends nothing during silence) are padded with
    zeros against the wall clock, so mic and system tracks stay aligned for the meeting timeline."""

    def __init__(self):
        self._chunks, self._n, self._lock = [], 0, threading.Lock()
        self.t0 = time.monotonic()
        self.level = 0.0

    def push(self, wave16k, pad_to_clock=False):
        with self._lock:
            if pad_to_clock:
                gap = int((time.monotonic() - self.t0) * SR) - self._n - len(wave16k)
                if gap > SR // 10:
                    self._chunks.append(np.zeros(gap, np.float32))
                    self._n += gap
            self._chunks.append(wave16k)
            self._n += len(wave16k)
        self.level = float(np.sqrt(np.mean(wave16k ** 2))) if len(wave16k) else 0.0

    def __len__(self):
        return self._n

    def read(self, start=0, end=None):
        with self._lock:
            if len(self._chunks) > 1:
                self._chunks = [np.concatenate(self._chunks)]
            data = self._chunks[0] if self._chunks else np.zeros(0, np.float32)
        return data[start:end]


class Mic:
    def __init__(self):
        self.track, self._stream = None, None

    def start(self):
        import sounddevice as sd

        self.track = Track()
        self._stream = sd.InputStream(samplerate=SR, channels=1, dtype="float32", blocksize=SR // 20,
                                      callback=lambda d, *_: self.track.push(d[:, 0].copy()))
        self._stream.start()
        return self.track

    def stop(self):
        if self._stream:
            self._stream.stop()
            self._stream.close()
            self._stream = None
        return self.track


class Loopback:
    """What the speakers play (the other side of a call). Needs PyAudioWPatch's WASAPI loopback."""

    def __init__(self):
        self.track, self._pa, self._stream = None, None, None

    def start(self, t0=None):
        import pyaudiowpatch as pyaudio

        self._pa = pyaudio.PyAudio()
        dev = self._pa.get_default_wasapi_loopback()
        rate, ch = int(dev["defaultSampleRate"]), dev["maxInputChannels"]
        self.track = Track()
        if t0:
            self.track.t0 = t0

        def cb(data, frames, info, status):
            wave = np.frombuffer(data, np.float32).reshape(-1, ch)
            self.track.push(to_16k_mono(wave, rate), pad_to_clock=True)
            return None, pyaudio.paContinue

        self._stream = self._pa.open(format=pyaudio.paFloat32, channels=ch, rate=rate, input=True,
                                     input_device_index=dev["index"], frames_per_buffer=rate // 20,
                                     stream_callback=cb)
        return self.track

    def stop(self):
        if self._stream:
            self._stream.stop_stream()
            self._stream.close()
            self._pa.terminate()
            self._stream = None
        # pad the tail so both tracks end at the same wall-clock time
        if self.track:
            self.track.push(np.zeros(0, np.float32), pad_to_clock=True)
        return self.track


def save_wav(path, wave, sr=SR):
    import wave as wavmod

    pcm = (np.clip(wave, -1, 1) * 32767).astype(np.int16)
    with wavmod.open(str(path), "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(sr)
        f.writeframes(pcm.tobytes())
