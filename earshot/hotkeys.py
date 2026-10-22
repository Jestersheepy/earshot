"""Global hotkeys. Hook callbacks run on keyboard's thread, so everything is handed to `post`."""
import keyboard


class Hotkeys:
    def __init__(self, cfg, post, on_hold_start, on_hold_end, toggles):
        """cfg: config["hotkeys"]. toggles: name -> fn for press-to-toggle combos."""
        self.post = post
        self.hold_combo = cfg["dictate"]
        self.hold_keys = [k.strip() for k in self.hold_combo.split("+")]
        self.on_hold_start, self.on_hold_end = on_hold_start, on_hold_end
        self.holding = False
        for name, fn in toggles.items():
            keyboard.add_hotkey(cfg[name], lambda fn=fn: post(fn))
        keyboard.hook(self._on_event)

    def _held(self):
        return all(keyboard.is_pressed(k) for k in self.hold_keys)

    def _on_event(self, e):
        if not self.holding and e.event_type == keyboard.KEY_DOWN and self._held():
            self.holding = True
            self.post(self.on_hold_start)
        elif self.holding and e.event_type == keyboard.KEY_UP and not self._held():
            self.holding = False
            self.post(self.on_hold_end)

    def hold_keys_released(self):
        return not any(keyboard.is_pressed(k) for k in self.hold_keys)

    def stop(self):
        keyboard.unhook_all()
