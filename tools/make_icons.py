"""Render the app icon and installer artwork from src-tauri/icons/*.svg (needs Microsoft Edge and Pillow).

    python tools/make_icons.py

earshot.svg is the detailed icon (48 px and up); earshot-small.svg is drawn for 16-32 px, where shadows and
fine edges turn to mush. Writes the PNGs and icon.ico Tauri bundles, and the NSIS header and sidebar BMPs.
"""
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from PIL import Image

ICONS = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
EDGE = Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
SMALL_UP_TO = 32

SIDEBAR = """<!doctype html><html><body style="margin:0;width:164px;height:314px;overflow:hidden;
background:#fbfaf7;font-family:'Segoe UI Variable Display','Segoe UI',sans-serif">
<div style="position:absolute;left:-58px;top:-40px;width:280px;height:260px;border-radius:50%;
background:radial-gradient(circle,rgba(20,163,139,.16),rgba(20,163,139,0) 68%)"></div>
<img src="{icon}" style="position:absolute;left:44px;top:62px;width:76px;height:76px">
<div style="position:absolute;top:154px;left:0;width:164px;text-align:center;color:#1e1c19;font-size:21px;font-weight:600;
letter-spacing:-.01em">Earshot</div>
<div style="position:absolute;top:184px;left:0;width:164px;text-align:center;color:#75706a;font-size:11px;
line-height:1.45">Dictation and meeting notes<br>that stay on your PC</div>
<div style="position:absolute;right:0;top:0;width:1px;height:314px;background:#e7e3da"></div>
</body></html>"""

HEADER = """<!doctype html><html><body style="margin:0;width:150px;height:57px;overflow:hidden;background:#fff">
<img src="{icon}" style="position:absolute;left:97px;top:8px;width:41px;height:41px">
</body></html>"""


def edge(src: Path, out: Path, w: int, h: int, profile: Path):
    subprocess.run([str(EDGE), "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                    "--default-background-color=00000000", f"--window-size={w},{h}", f"--user-data-dir={profile}",
                    f"--screenshot={out}", src.as_uri()], capture_output=True)
    # the msedge.exe we start can hand off to another process and return before the screenshot is written
    for _ in range(100):
        if out.exists() and out.stat().st_size > 0:
            time.sleep(0.3)
            return
        time.sleep(0.2)
    raise SystemExit(f"Edge didn't render {src.name}")


def main():
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        big, small = tmp / "big.png", tmp / "small.png"
        edge(ICONS / "earshot.svg", big, 1024, 1024, tmp / "edge")
        edge(ICONS / "earshot-small.svg", small, 1024, 1024, tmp / "edge")
        big_im, small_im = Image.open(big).convert("RGBA"), Image.open(small).convert("RGBA")

        def at(n: int) -> Image.Image:
            return (small_im if n <= SMALL_UP_TO else big_im).resize((n, n), Image.LANCZOS)

        for name, n in [("32x32.png", 32), ("64x64.png", 64), ("128x128.png", 128), ("128x128@2x.png", 256),
                        ("icon.png", 512)]:
            at(n).save(ICONS / name)
        sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
        frames = [at(n) for n in sizes]
        frames[-1].save(ICONS / "icon.ico", sizes=[(n, n) for n in sizes], append_images=frames[:-1])

        shutil.copy(big, tmp / "icon.png")
        for name, html, (w, h) in [("nsis-sidebar.bmp", SIDEBAR, (164, 314)), ("nsis-header.bmp", HEADER, (150, 57))]:
            page = tmp / (name + ".html")
            page.write_text(html.format(icon="icon.png"), encoding="utf-8")
            shot = tmp / (name + ".png")
            edge(page, shot, w, h, tmp / "edge")
            Image.open(shot).convert("RGB").save(ICONS / name)
    print("icons written to", ICONS)


if __name__ == "__main__":
    main()
