"""System tray icon (pystray, own thread). Menu callbacks are marshalled onto the tk thread via `post`."""
import pystray
from PIL import Image, ImageDraw


def _icon_image(color=(79, 140, 255)):
    s = 64
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.ellipse((4, 4, s - 4, s - 4), fill=(31, 32, 36, 255))
    d.ellipse((22, 22, s - 22, s - 22), fill=color + (255,))
    return img


class Tray:
    def __init__(self, post, actions, labels):
        """post(fn): run fn on the UI thread. actions: name -> fn. labels: name -> fn() -> menu text."""
        def item(name, default=False):
            return pystray.MenuItem(lambda _: labels[name](), lambda: post(actions[name]), default=default)

        menu = pystray.Menu(
            item("toggle_circle", default=True),  # left-click on the tray icon
            item("voice_note"),
            item("meeting"),
            pystray.Menu.SEPARATOR,
            item("open_notes"),
            item("quit"),
        )
        self.icon = pystray.Icon("earshot", _icon_image(), "Earshot", menu)
        self.icon.run_detached()

    def notify(self, message, title="Earshot"):
        try:
            self.icon.notify(message, title)
        except Exception:
            pass

    def stop(self):
        self.icon.stop()
