"""Tiny ctypes layer: window styles, clipboard, Ctrl+V via SendInput. No pywin32."""
import ctypes
import time
from ctypes import wintypes as wt

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

GWL_EXSTYLE = -20
WS_EX_TOPMOST, WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE = 0x8, 0x80, 0x08000000
CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x2
INPUT_KEYBOARD, KEYEVENTF_KEYUP = 1, 0x2
VK_CONTROL, VK_V = 0x11, 0x56

user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
user32.SetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.SetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int, ctypes.c_ssize_t]
user32.GetParent.restype = wt.HWND
user32.GetClipboardData.restype = wt.HANDLE
user32.SetClipboardData.restype = wt.HANDLE
user32.SetClipboardData.argtypes = [wt.UINT, wt.HANDLE]
kernel32.GlobalAlloc.restype = wt.HGLOBAL
kernel32.GlobalAlloc.argtypes = [wt.UINT, ctypes.c_size_t]
kernel32.GlobalLock.restype = ctypes.c_void_p
kernel32.GlobalLock.argtypes = [wt.HGLOBAL]
kernel32.GlobalUnlock.argtypes = [wt.HGLOBAL]


def make_floating(tk_window):
    """Topmost, no taskbar/Alt-Tab entry, and never takes focus from the app you are typing in."""
    hwnd = user32.GetParent(tk_window.winfo_id()) or tk_window.winfo_id()
    style = user32.GetWindowLongPtrW(hwnd, GWL_EXSTYLE)
    user32.SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
    return hwnd


def _open_clipboard():
    for _ in range(20):  # another app may hold it briefly
        if user32.OpenClipboard(None):
            return True
        time.sleep(0.01)
    return False


def get_clipboard_text():
    if not _open_clipboard():
        return None
    try:
        h = user32.GetClipboardData(CF_UNICODETEXT)
        if not h:
            return None
        p = kernel32.GlobalLock(h)
        try:
            return ctypes.wstring_at(p)
        finally:
            kernel32.GlobalUnlock(h)
    finally:
        user32.CloseClipboard()


def set_clipboard_text(text):
    if not _open_clipboard():
        return False
    try:
        user32.EmptyClipboard()
        data = ctypes.create_unicode_buffer(text)
        h = kernel32.GlobalAlloc(GMEM_MOVEABLE, ctypes.sizeof(data))
        ctypes.memmove(kernel32.GlobalLock(h), data, ctypes.sizeof(data))
        kernel32.GlobalUnlock(h)
        return bool(user32.SetClipboardData(CF_UNICODETEXT, h))
    finally:
        user32.CloseClipboard()


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]

    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def _key(vk, up=False):
    i = INPUT(type=INPUT_KEYBOARD)
    i.ki = KEYBDINPUT(vk, 0, KEYEVENTF_KEYUP if up else 0, 0, 0)
    return i


def send_ctrl_v():
    seq = (INPUT * 4)(_key(VK_CONTROL), _key(VK_V), _key(VK_V, True), _key(VK_CONTROL, True))
    user32.SendInput(4, seq, ctypes.sizeof(INPUT))


def paste_text(text, restore_after=0.3):
    """Put text into the focused field via the clipboard, then give the user's clipboard back."""
    previous = get_clipboard_text()
    if not set_clipboard_text(text):
        return False
    send_ctrl_v()
    if previous is not None:
        time.sleep(restore_after)
        set_clipboard_text(previous)
    return True
