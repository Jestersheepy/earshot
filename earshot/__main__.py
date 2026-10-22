"""Earshot: floating local dictation, voice notes and meeting notes.

    pythonw -m earshot                     # run (no console)
    python -m earshot --install-startup    # start with Windows
"""
import ctypes
import os
import queue
import subprocess
import sys
import threading
import time
import tkinter as tk
from pathlib import Path

from earshot import config, notes, win32
from earshot.audio import Mic, save_wav
from earshot.hotkeys import Hotkeys
from earshot.meeting import Meeting, mix, transcript_md
from earshot.stt.engine import SR, Engine
from earshot.summarize import available, summarize
from earshot.ui.circle import Circle
from earshot.ui.tray import Tray

MIN_SPEECH_S = 0.3


class App:
    def __init__(self):
        self.cfg = config.load()
        self.root = tk.Tk()
        self.root.title("Earshot")
        self.q = queue.Queue()
        self.mode = None  # None | "dictate" | "note" | "meeting"
        self.busy = 0
        self.mic, self.meeting = Mic(), None
        self.engine = Engine(self.cfg["stt_model_dir"], self.cfg["idle_unload_min"] * 60, self.cfg["stt_threads"])

        self.circle = Circle(self.root, self.cfg, on_click=self.click, on_menu=self.menu, on_moved=self.moved)
        if not self.cfg["circle"]["visible"]:
            self.circle.show(False)
        actions = {"toggle_circle": self.toggle_circle, "voice_note": self.toggle_note,
                   "meeting": self.toggle_meeting, "open_notes": self.open_notes, "quit": self.quit}
        labels = {"toggle_circle": lambda: "Hide circle" if self.root.winfo_viewable() else "Show circle",
                  "voice_note": lambda: "Stop voice note" if self.mode == "note" else "New voice note",
                  "meeting": lambda: "Stop meeting" if self.mode == "meeting" else "Start meeting notes",
                  "open_notes": lambda: "Open notes folder", "quit": lambda: "Quit"}
        self.actions = actions
        self.tray = Tray(self.post, actions, labels)
        self.hotkeys = Hotkeys(self.cfg["hotkeys"], self.post, self.dictate_start, self.dictate_stop,
                               {k: actions[k] for k in ("voice_note", "meeting", "toggle_circle")})
        if self.cfg["preload"]:
            threading.Thread(target=self.engine.preload, daemon=True).start()
        self.root.after(20, self._drain)

    # --- plumbing ------------------------------------------------------------------------------
    def post(self, fn):
        self.q.put(fn)

    def _drain(self):
        while not self.q.empty():
            try:
                self.q.get_nowait()()
            except Exception as e:
                self.tray.notify(f"Error: {e}")
        self.root.after(20, self._drain)

    def background(self, work, done=None):
        """Run work() off the UI thread with the circle showing 'busy'; done(result) back on the UI thread."""
        self.busy += 1
        self.refresh()

        def run():
            try:
                result, err = work(), None
            except Exception as e:
                result, err = None, e

            def finish():
                self.busy -= 1
                self.refresh()
                if err:
                    self.tray.notify(f"Error: {err}")
                elif done:
                    done(result)
            self.post(finish)
        threading.Thread(target=run, daemon=True).start()

    def refresh(self):
        if self.mode == "meeting":
            self.circle.set_state("meeting", self.meeting.level)
        elif self.mode in ("dictate", "note"):
            self.circle.set_state(self.mode, lambda: self.mic.track.level)
        else:
            self.circle.set_state("busy" if self.busy else "idle")

    # --- dictation -----------------------------------------------------------------------------
    def dictate_start(self):
        if self.mode:
            return
        self.mode = "dictate"
        self.mic.start()
        self.refresh()

    def dictate_stop(self):
        if self.mode != "dictate":
            return
        wave = self.mic.stop().read()
        self.mode = None
        if len(wave) < MIN_SPEECH_S * SR:
            self.refresh()
            return

        def work():
            text = notes.clean(self.engine.transcribe(wave), self.cfg["drop_fillers"])
            deadline = time.monotonic() + 2  # Win still held + our Ctrl+V = Win+V clipboard panel
            while not self.hotkeys.hold_keys_released() and time.monotonic() < deadline:
                time.sleep(0.02)
            if text:
                win32.paste_text(text)
        self.background(work)

    # --- voice notes ---------------------------------------------------------------------------
    def toggle_note(self):
        if self.mode == "note":
            return self._finish_note()
        if self.mode:
            return
        self.mode, self.note_started = "note", notes.now()
        self.mic.start()
        self.refresh()

    def _finish_note(self):
        wave, started = self.mic.stop().read(), self.note_started
        self.mode = None
        if len(wave) < MIN_SPEECH_S * SR:
            return self.refresh()
        cfg = self.cfg

        def work():
            text = notes.clean(self.engine.transcribe(wave), cfg["drop_fillers"])
            path = notes.new_note_path(cfg["notes_dir"], "Voice Notes", started)
            extra = {}
            if cfg["keep_wav"]:
                save_wav(path.with_suffix(".wav"), wave)
                extra["audio"] = f"[[{path.with_suffix('.wav').name}]]"
            return notes.write_note(path, "voice-note", started, len(wave) / SR, text or "_(no speech detected)_", extra)
        self.background(work, lambda p: self.tray.notify(f"Saved {p.name}", "Voice note"))

    # --- meetings ------------------------------------------------------------------------------
    def toggle_meeting(self):
        if self.mode == "meeting":
            return self._finish_meeting()
        if self.mode:
            return
        self.meeting, self.meeting_started = Meeting(self.engine, self.tray.notify), notes.now()
        self.meeting.start()
        self.mode = "meeting"
        self.refresh()

    def _finish_meeting(self):
        meeting, started, cfg = self.meeting, self.meeting_started, self.cfg
        self.mode, self.meeting = None, None

        def work():
            duration, lines, me, others = meeting.stop()
            path = notes.new_note_path(cfg["notes_dir"], "Meetings", started)
            extra = {"participants": "Me, Others" if others else "Me"}
            if cfg["keep_wav"]:
                save_wav(path.with_suffix(".wav"), mix(me, others))
                extra["audio"] = f"[[{path.with_suffix('.wav').name}]]"
            body = "## Transcript\n\n" + (transcript_md(lines) or "_(no speech detected)_")
            notes.write_note(path, "meeting", started, duration, body, extra)  # saved before the slow part
            if lines and available(cfg["llm"]):
                header = {"Title": "Meeting", "Date": started.strftime("%B %d, %Y"),
                          "Time": started.strftime("%I:%M %p"), "Duration": notes.fmt_duration(duration),
                          "Participants": extra["participants"]}
                summary = summarize(cfg["llm"], lines, header)
                notes.write_note(path, "meeting", started, duration, f"{summary}\n\n{body}", extra)
            return path
        self.background(work, lambda p: self.tray.notify(f"Saved {p.name}", "Meeting notes"))

    # --- circle / tray -------------------------------------------------------------------------
    def click(self):
        if self.mode == "meeting":
            self.toggle_meeting()
        else:
            self.toggle_note()

    def menu(self, x, y):
        m = tk.Menu(self.root, tearoff=0)
        m.add_command(label="Stop voice note" if self.mode == "note" else "New voice note", command=self.toggle_note)
        m.add_command(label="Stop meeting" if self.mode == "meeting" else "Start meeting notes",
                      command=self.toggle_meeting)
        m.add_separator()
        m.add_command(label="Open notes folder", command=self.open_notes)
        m.add_command(label="Hide to tray", command=self.toggle_circle)
        m.add_command(label="Quit", command=self.quit)
        m.tk_popup(x, y)

    def moved(self, x, y):
        self.cfg["circle"].update(x=x, y=y)
        config.save(self.cfg)

    def toggle_circle(self):
        visible = not self.root.winfo_viewable()
        self.circle.show(visible)
        self.cfg["circle"]["visible"] = visible
        config.save(self.cfg)

    def open_notes(self):
        Path(self.cfg["notes_dir"]).mkdir(parents=True, exist_ok=True)
        os.startfile(self.cfg["notes_dir"])

    def quit(self):
        if self.mode == "meeting":
            self.tray.notify("Stop the meeting first so its notes are saved.")
            return
        self.hotkeys.stop()
        self.tray.stop()
        self.root.destroy()

    def run(self):
        self.root.mainloop()


def single_instance():
    global _mutex  # held for the life of the process
    _mutex = win32.kernel32.CreateMutexW(None, False, "Local\\EarshotSingleInstance")
    return ctypes.get_last_error() != 183  # ERROR_ALREADY_EXISTS


def install_startup():
    startup = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Startup/Earshot.lnk"
    pyw = Path(sys.executable).with_name("pythonw.exe")
    ps = (f"$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{startup}');$s.TargetPath='{pyw}';"
          f"$s.Arguments='-m earshot';$s.WorkingDirectory='{config.ROOT}';$s.Save()")
    subprocess.run(["powershell", "-NoProfile", "-Command", ps], check=True)
    print("Added", startup)


def main():
    if "--install-startup" in sys.argv:
        return install_startup()
    if not single_instance():
        return
    ctypes.windll.shcore.SetProcessDpiAwareness(1)
    App().run()


if __name__ == "__main__":
    main()
