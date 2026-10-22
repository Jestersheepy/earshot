"""The floating circle: borderless, always on top, draggable, never steals focus."""
import math
import time
import tkinter as tk

from PIL import Image, ImageDraw, ImageFont, ImageTk

from earshot import win32

KEY = "#010203"  # transparent colour key; antialiased edges blend toward it (near-black, so reads as a shadow)
BG = (31, 32, 36)
ACCENT = {
    "idle": (138, 143, 152),
    "dictate": (79, 140, 255),
    "note": (245, 165, 36),
    "meeting": (239, 68, 68),
    "busy": (200, 200, 210),
}
SS = 4  # supersampling for smooth edges


class Circle:
    def __init__(self, root, cfg, on_click, on_menu, on_moved):
        self.root, self.cfg = root, cfg
        self.size = cfg["circle"]["size"]
        self.on_click, self.on_menu, self.on_moved = on_click, on_menu, on_moved
        self.state, self.level_fn, self.t0, self._anim = "idle", None, None, None
        self._drag, self._click_job = None, None

        root.overrideredirect(True)
        root.attributes("-topmost", True)
        root.configure(bg=KEY)
        root.attributes("-transparentcolor", KEY)
        x, y = cfg["circle"]["x"], cfg["circle"]["y"]
        if x is None:
            x, y = root.winfo_screenwidth() - self.size - 24, root.winfo_screenheight() // 2
        root.geometry(f"{self.size}x{self.size}+{x}+{y}")
        self.label = tk.Label(root, bg=KEY, bd=0, highlightthickness=0)
        self.label.pack()
        self.font = _font(max(9, self.size // 5) * SS)
        self._render()

        self.label.bind("<ButtonPress-1>", self._press)
        self.label.bind("<B1-Motion>", self._motion)
        self.label.bind("<ButtonRelease-1>", self._release)
        self.label.bind("<Double-Button-1>", self._double)
        self.label.bind("<Button-3>", lambda e: self.on_menu(e.x_root, e.y_root))
        root.update_idletasks()
        win32.make_floating(root)

    # --- state ---------------------------------------------------------------------------------
    def set_state(self, state, level_fn=None):
        if state != self.state:
            self.t0 = time.monotonic()
        self.state, self.level_fn = state, level_fn
        if self._anim:
            self.root.after_cancel(self._anim)
            self._anim = None
        self._tick()

    def _tick(self):
        self._render()
        if self.state != "idle":  # idle draws once; no timer, no CPU
            self._anim = self.root.after(50, self._tick)

    def show(self, visible=True):
        self.root.deiconify() if visible else self.root.withdraw()
        if visible:
            self.root.attributes("-topmost", True)

    # --- drawing -------------------------------------------------------------------------------
    def _render(self):
        s = self.size * SS
        img = Image.new("RGB", (s, s), (1, 2, 3))
        d = ImageDraw.Draw(img)
        pad = 2 * SS
        d.ellipse((pad, pad, s - pad, s - pad), fill=BG, outline=(60, 62, 70), width=SS)
        c, col = s / 2, ACCENT[self.state]
        t = time.monotonic() - (self.t0 or 0)
        if self.state == "idle":
            r = s * 0.12
            d.ellipse((c - r, c - r, c + r, c + r), fill=col)
        elif self.state == "busy":
            r = s * 0.28
            start = (t * 360) % 360
            d.arc((c - r, c - r, c + r, c + r), start, start + 270, fill=col, width=3 * SS)
        else:
            level = min(1.0, (self.level_fn() if self.level_fn else 0.0) * 12)
            r = s * (0.16 + 0.16 * level)
            d.ellipse((c - r, c - r, c + r, c + r), fill=col)
            ring = s * 0.36 + math.sin(t * 4) * SS
            d.ellipse((c - ring, c - ring, c + ring, c + ring), outline=col, width=2 * SS)
            if self.state == "meeting":
                m, sec = divmod(int(t), 60)
                txt = f"{m}:{sec:02d}"
                w = d.textlength(txt, font=self.font)
                d.text((c - w / 2, s * 0.68), txt, fill=(235, 235, 240), font=self.font)
        img = img.resize((self.size, self.size), Image.LANCZOS)
        self._photo = ImageTk.PhotoImage(img)
        self.label.configure(image=self._photo)

    # --- mouse ---------------------------------------------------------------------------------
    def _press(self, e):
        self._drag = (e.x_root, e.y_root, self.root.winfo_x(), self.root.winfo_y(), False)

    def _motion(self, e):
        x0, y0, wx, wy, moved = self._drag
        dx, dy = e.x_root - x0, e.y_root - y0
        if moved or abs(dx) + abs(dy) > 4:
            self._drag = (x0, y0, wx, wy, True)
            self.root.geometry(f"+{wx + dx}+{wy + dy}")

    def _release(self, e):
        moved = self._drag and self._drag[4]
        self._drag = None
        if moved:
            self.on_moved(self.root.winfo_x(), self.root.winfo_y())
        else:  # wait to see if this is the first half of a double-click
            self._click_job = self.root.after(250, self._single)

    def _single(self):
        self._click_job = None
        self.on_click()

    def _double(self, e):
        if self._click_job:
            self.root.after_cancel(self._click_job)
            self._click_job = None
        self.show(False)


def _font(px):
    for name in ("segoeuisb.ttf", "segoeui.ttf", "arial.ttf"):
        try:
            return ImageFont.truetype(name, px)
        except OSError:
            pass
    return ImageFont.load_default()
