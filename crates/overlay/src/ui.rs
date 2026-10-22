//! Win32 side: per-pixel-alpha layered windows for the pill, menu and toasts, on one UI thread.
//!
//! The app talks to it through [`Overlay`] (commands in) and an mpsc Sender<UiEvent> (events out).
//! Nothing animates while idle: timers only run during morphs, live states and toasts.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use parking_lot::Mutex;
use tiny_skia::Pixmap;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::menu::{self, MenuItem, ToastSpec};
use crate::pill::{self, PillState, View};
use crate::theme::{self, IdleStyle, Style};

const CANVAS_W: f32 = 340.0; // pill canvas (logical px): widest state (the hint bar) + shadow room
const CANVAS_H: f32 = 64.0;
const TIMER_ANIM: usize = 1;
const TIMER_CLICK: usize = 2;
const TIMER_TOAST: usize = 3;
const TIMER_PEEK: usize = 4;
const FRAME_MS: u32 = 16;
/// The breathing idle halo is slow; 15 fps is plenty and keeps CPU near zero.
const SLOW_FRAME_MS: u32 = 66;
const PEEK_DELAY_MS: u32 = 450;
const LEVEL_HZ: f32 = 30.0;
const HISTORY: usize = 48;
const WM_MOUSELEAVE: u32 = 0x02A3; // lives in Win32_UI_Controls; not worth the feature

#[derive(Debug, Clone)]
pub enum UiEvent {
    Click,
    DoubleClick,
    Stop,
    /// Pill dragged; new centre in physical screen px.
    Moved { x: i32, y: i32 },
    ContextMenu { x: i32, y: i32 },
    Menu(String),
    ToastButton { id: u64, action: String },
    ToastClosed { id: u64 },
}

enum Cmd {
    State(PillState),
    Visible(bool),
    Menu(Vec<MenuItem>, i32, i32),
    Toast(ToastSpec),
    CloseToast(u64),
    Restyle,
    Quit,
}

/// Handle for the app. Cheap to clone.
#[derive(Clone)]
pub struct Overlay {
    tx: Sender<Cmd>,
    thread_id: u32,
    /// The pill's waveform reads this (f32 bits); the app stores the active track's level.
    pub level: Arc<AtomicU32>,
    visible: Arc<Mutex<bool>>,
    /// Pill centre, physical screen px.
    center: Arc<Mutex<(i32, i32)>>,
}

impl Overlay {
    /// `pos`: saved pill centre (physical px), or None for bottom-centre above the taskbar.
    pub fn start(events: Sender<UiEvent>, pos: Option<(i32, i32)>, visible: bool) -> Self {
        let (tx, rx) = channel();
        let (id_tx, id_rx) = channel();
        let level = Arc::new(AtomicU32::new(0));
        let lv = level.clone();
        let center = Arc::new(Mutex::new(pos.unwrap_or((0, 0))));
        let c = center.clone();
        std::thread::Builder::new()
            .name("overlay".into())
            .spawn(move || {
                let _ = id_tx.send(unsafe { GetCurrentThreadId() });
                run(rx, events, lv, pos, visible, c);
            })
            .expect("overlay thread");
        let thread_id = id_rx.recv().expect("overlay thread id");
        Self { tx, thread_id, level, visible: Arc::new(Mutex::new(visible)), center }
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0));
        }
    }

    pub fn set_state(&self, s: PillState) {
        self.send(Cmd::State(s));
    }
    pub fn set_level(&self, v: f32) {
        self.level.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_visible(&self, v: bool) {
        *self.visible.lock() = v;
        self.send(Cmd::Visible(v));
    }
    pub fn is_visible(&self) -> bool {
        *self.visible.lock()
    }
    pub fn menu(&self, items: Vec<MenuItem>, x: i32, y: i32) {
        self.send(Cmd::Menu(items, x, y));
    }
    pub fn toast(&self, t: ToastSpec) {
        self.send(Cmd::Toast(t));
    }
    pub fn close_toast(&self, id: u64) {
        self.send(Cmd::CloseToast(id));
    }
    pub fn quit(&self) {
        self.send(Cmd::Quit);
    }
    /// Look and personalisation; applies immediately.
    pub fn set_style(&self, s: Style) {
        theme::set(s.clone());
        self.send(Cmd::Restyle);
    }
    /// The pill's centre and the work area of its monitor (left, top, right, bottom), physical px.
    pub fn pill_place(&self) -> ((i32, i32), (i32, i32, i32, i32)) {
        let (x, y) = *self.center.lock();
        let wa = work_area_at(POINT { x, y });
        ((x, y), (wa.left, wa.top, wa.right, wa.bottom))
    }
}

// ---- UI thread state ---------------------------------------------------------------------------

struct Spring {
    x: f32,
    v: f32,
}

impl Spring {
    fn step(&mut self, target: f32, dt: f32) {
        let (k, c) = (420.0, 2.0 * 420f32.sqrt() * 0.78); // slightly under-damped: a gentle settle
        self.v += (k * (target - self.x) - c * self.v) * dt;
        self.x += self.v * dt;
    }
    fn settled(&self, target: f32) -> bool {
        (self.x - target).abs() < 0.05 && self.v.abs() < 0.05
    }
}

struct PillWin {
    hwnd: HWND,
    state: PillState,
    w: Spring,
    h: Spring,
    content: f32,
    changed: Instant,
    rec_start: Instant,
    hover: bool,
    hover_stop: bool,
    /// Idle and hovered a moment: showing the hint bar.
    peek: bool,
    tracking: bool,
    drag: Option<(POINT, POINT, bool)>, // cursor at press, window centre at press, moved
    center: POINT,
    shared_center: Arc<Mutex<(i32, i32)>>,
    /// Animation timer at the slow (breathing) rate.
    slow: bool,
    levels: Vec<f32>,
    level_acc: f32,
    visible: bool,
    animating: bool,
}

struct MenuWin {
    hwnd: HWND,
    items: Vec<MenuItem>,
    hover: Option<(usize, Option<usize>)>,
    opened: Instant,
    origin: POINT,
}

struct ToastWin {
    hwnd: HWND,
    spec: ToastSpec,
    shown: Instant,
    elapsed: f32,
    hover: bool,
    hover_btn: Option<usize>,
    tracking: bool,
    closing: Option<Instant>,
}

struct Ui {
    pill: PillWin,
    menu: Option<MenuWin>,
    toasts: Vec<ToastWin>,
    events: Sender<UiEvent>,
    level: Arc<AtomicU32>,
    last_tick: Instant,
    epoch: Instant,
    quit: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

fn register(name: PCWSTR, proc: WNDPROC) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: proc,
            hInstance: hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: name,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

fn create(class: PCWSTR, activatable: bool) -> HWND {
    let mut ex = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
    if !activatable {
        ex |= WS_EX_NOACTIVATE;
    }
    unsafe { CreateWindowExW(ex, class, w!("Earshot"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinstance()), None) }
        .expect("create overlay window")
}

fn scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Blit a premultiplied RGBA pixmap to a layered window at (x, y) physical px.
fn present(hwnd: HWND, pm: &Pixmap, x: i32, y: i32, alpha: u8) {
    unsafe {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]); // RGBA -> BGRA, both premultiplied
            }
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(&POINT { x, y }),
                Some(&SIZE { cx: w, cy: h }),
                Some(mem),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

fn work_area_at(p: POINT) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcWork
    }
}

fn lparam_xy(l: LPARAM) -> (i32, i32) {
    ((l.0 & 0xFFFF) as i16 as i32, ((l.0 >> 16) & 0xFFFF) as i16 as i32)
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn track_leave(hwnd: HWND) {
    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
    unsafe {
        let _ = TrackMouseEvent(&mut t);
    }
}

// ---- thread main -----------------------------------------------------------------------------

fn run(
    rx: Receiver<Cmd>,
    events: Sender<UiEvent>,
    level: Arc<AtomicU32>,
    pos: Option<(i32, i32)>,
    visible: bool,
    shared_center: Arc<Mutex<(i32, i32)>>,
) {
    register(w!("EarshotPill"), Some(pill_proc));
    register(w!("EarshotMenu"), Some(menu_proc));
    register(w!("EarshotToast"), Some(toast_proc));
    let hwnd = create(w!("EarshotPill"), false);
    let center = pos.map(|(x, y)| POINT { x, y }).unwrap_or_else(|| {
        let wa = work_area_at(POINT { x: 0, y: 0 });
        POINT { x: (wa.left + wa.right) / 2, y: wa.bottom - (36.0 * scale(hwnd)) as i32 }
    });
    *shared_center.lock() = (center.x, center.y);
    let (w0, h0) = PillState::Idle.size(false, false);
    UI.with(|u| {
        *u.borrow_mut() = Some(Ui {
            pill: PillWin {
                hwnd,
                state: PillState::Idle,
                w: Spring { x: w0, v: 0.0 },
                h: Spring { x: h0, v: 0.0 },
                content: 1.0,
                changed: Instant::now(),
                rec_start: Instant::now(),
                hover: false,
                hover_stop: false,
                peek: false,
                tracking: false,
                drag: None,
                center,
                shared_center,
                slow: false,
                levels: vec![0.0; HISTORY],
                level_acc: 0.0,
                visible,
                animating: false,
            },
            menu: None,
            toasts: Vec::new(),
            events,
            level,
            last_tick: Instant::now(),
            epoch: Instant::now(),
            quit: false,
        })
    });
    with_ui(|ui| {
        ui.render_pill();
        if visible {
            unsafe {
                let _ = ShowWindow(ui.pill.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    });
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_APP {
                let quit = with_ui(|ui| {
                    while let Ok(c) = rx.try_recv() {
                        ui.command(c);
                    }
                    ui.quit
                });
                if quit == Some(true) {
                    break;
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

impl Ui {
    fn emit(&self, e: UiEvent) {
        let _ = self.events.send(e);
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::State(s) => self.set_state(s),
            Cmd::Visible(v) => {
                self.pill.visible = v;
                unsafe {
                    let _ = ShowWindow(self.pill.hwnd, if v { SW_SHOWNOACTIVATE } else { SW_HIDE });
                }
                if v {
                    self.render_pill();
                }
            }
            Cmd::Menu(items, x, y) => self.open_menu(items, x, y),
            Cmd::Toast(t) => self.open_toast(t),
            Cmd::CloseToast(id) => {
                if let Some(t) = self.toasts.iter_mut().find(|t| t.spec.id == id) {
                    t.closing.get_or_insert(Instant::now());
                    unsafe {
                        SetTimer(Some(t.hwnd), TIMER_TOAST, FRAME_MS, None);
                    }
                }
            }
            Cmd::Restyle => {
                self.render_pill();
                self.kick();
            }
            Cmd::Quit => {
                self.quit = true;
                unsafe {
                    let _ = DestroyWindow(self.pill.hwnd);
                }
            }
        }
    }

    // --- pill ---------------------------------------------------------------------------------

    fn set_state(&mut self, s: PillState) {
        let p = &mut self.pill;
        if std::mem::discriminant(&s) != std::mem::discriminant(&p.state) {
            p.content = 0.0;
            p.changed = Instant::now();
            if let PillState::Recording { secs, .. } = &s {
                // the timer counts from here; `secs` lets the app resume an older recording's clock
                p.rec_start = Instant::now() - std::time::Duration::from_secs(*secs as u64);
            }
        }
        p.state = s;
        if p.state != PillState::Idle {
            p.peek = false;
        }
        self.kick();
    }

    /// Start the animation timer at full rate (if it isn't already).
    fn kick(&mut self) {
        if !self.pill.animating || self.pill.slow {
            if !self.pill.animating {
                self.last_tick = Instant::now();
            }
            self.pill.animating = true;
            self.pill.slow = false;
            unsafe {
                SetTimer(Some(self.pill.hwnd), TIMER_ANIM, FRAME_MS, None);
            }
        }
    }

    fn live(&self) -> bool {
        !matches!(self.pill.state, PillState::Idle | PillState::Done | PillState::Error)
    }

    /// Idle but breathing: keeps a slow timer instead of stopping.
    fn breathing(&self) -> bool {
        self.pill.state == PillState::Idle && !self.pill.peek && theme::style().idle == IdleStyle::Breathing
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let (tw, th) = self.pill.state.size(self.pill.hover, self.pill.peek);
        let p = &mut self.pill;
        p.w.step(tw, dt);
        p.h.step(th, dt);
        if p.changed.elapsed().as_secs_f32() > 0.06 {
            p.content = (p.content + dt / 0.16).min(1.0);
        }
        p.level_acc += dt;
        while p.level_acc >= 1.0 / LEVEL_HZ {
            p.level_acc -= 1.0 / LEVEL_HZ;
            p.levels.remove(0);
            p.levels.push(f32::from_bits(self.level.load(Ordering::Relaxed)));
        }
        if let PillState::Recording { secs, .. } = &mut p.state {
            *secs = p.rec_start.elapsed().as_secs() as u32;
        }
        let settled = p.w.settled(tw) && p.h.settled(th) && p.content >= 1.0;
        self.render_pill();
        if settled && !self.live() {
            if self.breathing() {
                if !self.pill.slow {
                    self.pill.slow = true;
                    unsafe {
                        SetTimer(Some(self.pill.hwnd), TIMER_ANIM, SLOW_FRAME_MS, None);
                    }
                }
            } else {
                self.pill.animating = false;
                unsafe {
                    let _ = KillTimer(Some(self.pill.hwnd), TIMER_ANIM);
                }
            }
        }
    }

    fn render_pill(&mut self) {
        let p = &self.pill;
        if !p.visible {
            return;
        }
        let s = scale(p.hwnd);
        let (pw, ph) = ((CANVAS_W * s).ceil() as u32, (CANVAS_H * s).ceil() as u32);
        let Some(mut pm) = Pixmap::new(pw, ph) else { return };
        let view = View {
            state: &p.state,
            peek: p.peek,
            w: p.w.x.max(4.0),
            h: p.h.x.max(4.0),
            content: p.content,
            hover: p.hover,
            hover_stop: p.hover_stop,
            levels: &p.levels,
            t: self.epoch.elapsed().as_secs_f32(),
            scale: s,
        };
        pill::draw(&mut pm, &view, CANVAS_W, CANVAS_H);
        present(p.hwnd, &pm, p.center.x - pw as i32 / 2, p.center.y - ph as i32 / 2, 255);
    }

    fn pill_mouse(&mut self, msg: u32, l: LPARAM) {
        let s = scale(self.pill.hwnd);
        let (lx, ly) = lparam_xy(l);
        let (x, y) = (lx as f32 / s, ly as f32 / s);
        match msg {
            WM_MOUSEMOVE => {
                if !self.pill.tracking {
                    track_leave(self.pill.hwnd);
                    self.pill.tracking = true;
                }
                if let Some((start, c0, moved)) = self.pill.drag {
                    let cur = cursor();
                    let (dx, dy) = (cur.x - start.x, cur.y - start.y);
                    if moved || dx.abs() + dy.abs() > 4 {
                        self.pill.drag = Some((start, c0, true));
                        self.pill.center = POINT { x: c0.x + dx, y: c0.y + dy };
                        *self.pill.shared_center.lock() = (self.pill.center.x, self.pill.center.y);
                        self.pill.peek = false;
                        self.render_pill();
                    }
                }
                let hover_stop = pill::hits_stop(&self.pill.state, self.pill.w.x, CANVAS_W, CANVAS_H, x, y, true);
                if !self.pill.hover && self.pill.state == PillState::Idle && theme::style().peek {
                    unsafe {
                        SetTimer(Some(self.pill.hwnd), TIMER_PEEK, PEEK_DELAY_MS, None);
                    }
                }
                if !self.pill.hover || hover_stop != self.pill.hover_stop {
                    self.pill.hover = true;
                    self.pill.hover_stop = hover_stop;
                    self.kick();
                }
            }
            WM_MOUSELEAVE => {
                self.pill.tracking = false;
                self.pill.hover = false;
                self.pill.hover_stop = false;
                self.pill.peek = false;
                unsafe {
                    let _ = KillTimer(Some(self.pill.hwnd), TIMER_PEEK);
                }
                self.kick();
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.pill.hwnd);
                    let _ = KillTimer(Some(self.pill.hwnd), TIMER_PEEK);
                }
                self.pill.drag = Some((cursor(), self.pill.center, false));
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let moved = self.pill.drag.take().is_some_and(|d| d.2);
                if moved {
                    let c = self.pill.center;
                    self.emit(UiEvent::Moved { x: c.x, y: c.y });
                } else if pill::hits_stop(&self.pill.state, self.pill.w.x, CANVAS_W, CANVAS_H, x, y, self.pill.hover) {
                    self.emit(UiEvent::Stop);
                } else {
                    unsafe {
                        SetTimer(Some(self.pill.hwnd), TIMER_CLICK, GetDoubleClickTime(), None);
                    }
                }
            }
            WM_LBUTTONDBLCLK => unsafe {
                let _ = KillTimer(Some(self.pill.hwnd), TIMER_CLICK);
                self.emit(UiEvent::DoubleClick);
            },
            WM_RBUTTONUP => {
                let c = cursor();
                self.emit(UiEvent::ContextMenu { x: c.x, y: c.y });
            }
            _ => {}
        }
    }

    // --- menu ---------------------------------------------------------------------------------

    fn open_menu(&mut self, items: Vec<MenuItem>, x: i32, y: i32) {
        self.close_menu();
        let hwnd = create(w!("EarshotMenu"), true);
        let s = scale(self.pill.hwnd);
        let (w, h) = ((menu::MENU_W + 2.0 * menu::MARGIN) * s, (menu::menu_height(&items) + 2.0 * menu::MARGIN) * s);
        let wa = work_area_at(POINT { x, y });
        let m = (menu::MARGIN * s) as i32;
        // open up/left from the cursor when there's no room below/right
        let ox = if x + w as i32 - m > wa.right { x - w as i32 + m } else { x - m };
        let oy = if y + h as i32 - m > wa.bottom { y - h as i32 + m } else { y - m };
        self.menu = Some(MenuWin { hwnd, items, hover: None, opened: Instant::now(), origin: POINT { x: ox, y: oy } });
        self.render_menu();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            SetTimer(Some(hwnd), TIMER_ANIM, FRAME_MS, None);
        }
    }

    fn render_menu(&mut self) {
        let Some(m) = &self.menu else { return };
        let s = scale(m.hwnd);
        let (w, h) = ((menu::MENU_W + 2.0 * menu::MARGIN) * s, (menu::menu_height(&m.items) + 2.0 * menu::MARGIN) * s);
        let Some(mut pm) = Pixmap::new(w.ceil() as u32, h.ceil() as u32) else { return };
        menu::draw_menu(&mut pm, &m.items, m.hover, s);
        let t = (m.opened.elapsed().as_secs_f32() / 0.12).min(1.0);
        let ease = 1.0 - (1.0 - t).powi(3);
        present(m.hwnd, &pm, m.origin.x, m.origin.y + ((1.0 - ease) * 6.0 * s) as i32, (ease * 255.0) as u8);
    }

    fn close_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            unsafe {
                let _ = DestroyWindow(m.hwnd);
            }
        }
    }

    fn menu_msg(&mut self, msg: u32, w: WPARAM, l: LPARAM) {
        let Some(m) = &mut self.menu else { return };
        let s = scale(m.hwnd);
        let (lx, ly) = lparam_xy(l);
        match msg {
            WM_TIMER => {
                self.render_menu();
                if let Some(m) = &self.menu {
                    if m.opened.elapsed().as_secs_f32() > 0.12 {
                        unsafe {
                            let _ = KillTimer(Some(m.hwnd), TIMER_ANIM);
                        }
                    }
                }
            }
            WM_MOUSEMOVE => {
                let h = menu::hit(&m.items, lx as f32 / s, ly as f32 / s);
                if h != m.hover {
                    m.hover = h;
                    self.render_menu();
                }
            }
            WM_LBUTTONUP => match menu::hit(&m.items, lx as f32 / s, ly as f32 / s) {
                // a choice row: select in place and keep the menu open
                Some((i, Some(k))) => {
                    m.items[i].selected = k;
                    let id = format!("{}:{}", m.items[i].id, m.items[i].choices[k].0);
                    self.render_menu();
                    self.emit(UiEvent::Menu(id));
                }
                Some((i, None)) if m.items[i].choices.is_empty() => {
                    let id = m.items[i].id.clone();
                    self.close_menu();
                    self.emit(UiEvent::Menu(id));
                }
                _ => {}
            },
            WM_KEYDOWN => {
                let n = m.items.len();
                match VIRTUAL_KEY(w.0 as u16) {
                    VK_ESCAPE => self.close_menu(),
                    VK_DOWN => {
                        m.hover = Some((m.hover.map_or(0, |(i, _)| (i + 1) % n), None));
                        self.render_menu();
                    }
                    VK_UP => {
                        m.hover = Some((m.hover.map_or(n - 1, |(i, _)| (i + n - 1) % n), None));
                        self.render_menu();
                    }
                    VK_RETURN => {
                        if let Some((i, _)) = m.hover.filter(|(i, _)| m.items[*i].choices.is_empty()) {
                            let id = m.items[i].id.clone();
                            self.close_menu();
                            self.emit(UiEvent::Menu(id));
                        }
                    }
                    _ => {}
                }
            }
            WM_ACTIVATE if (w.0 & 0xFFFF) as u32 == WA_INACTIVE => self.close_menu(),
            _ => {}
        }
    }

    // --- toasts -------------------------------------------------------------------------------

    fn open_toast(&mut self, spec: ToastSpec) {
        if let Some(t) = self.toasts.iter_mut().find(|t| t.spec.id == spec.id) {
            t.spec = spec; // update in place (e.g. countdown text)
            return;
        }
        let hwnd = create(w!("EarshotToast"), false);
        self.toasts.push(ToastWin {
            hwnd,
            spec,
            shown: Instant::now(),
            elapsed: 0.0,
            hover: false,
            hover_btn: None,
            tracking: false,
            closing: None,
        });
        self.layout_toasts();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetTimer(Some(hwnd), TIMER_TOAST, FRAME_MS, None);
        }
    }

    /// Stack toasts bottom-right of the pill's monitor, newest at the bottom.
    fn layout_toasts(&mut self) {
        let wa = work_area_at(self.pill.center);
        let mut bottom = wa.bottom - 8;
        for i in (0..self.toasts.len()).rev() {
            let hwnd = self.toasts[i].hwnd;
            let s = scale(hwnd);
            let h = ((menu::toast_height(&self.toasts[i].spec) + 2.0 * menu::MARGIN) * s) as i32;
            let w = ((menu::TOAST_W + 2.0 * menu::MARGIN) * s) as i32;
            self.render_toast(i, wa.right - 8 - w, bottom - h);
            bottom -= h - (menu::MARGIN * s) as i32;
        }
    }

    fn render_toast(&mut self, i: usize, x: i32, y: i32) {
        let t = &self.toasts[i];
        let s = scale(t.hwnd);
        let (w, h) = ((menu::TOAST_W + 2.0 * menu::MARGIN) * s, (menu::toast_height(&t.spec) + 2.0 * menu::MARGIN) * s);
        let Some(mut pm) = Pixmap::new(w.ceil() as u32, h.ceil() as u32) else { return };
        let progress = if t.spec.timeout_s > 0.0 { t.elapsed / t.spec.timeout_s } else { 0.0 };
        menu::draw_toast(&mut pm, &t.spec, s, t.hover_btn, t.hover, progress);
        let appear = (t.shown.elapsed().as_secs_f32() / 0.22).min(1.0);
        let gone = t.closing.map_or(0.0, |c| (c.elapsed().as_secs_f32() / 0.18).min(1.0));
        let ease = (1.0 - (1.0 - appear).powi(3)) * (1.0 - gone);
        let dy = ((1.0 - ease) * 14.0 * s) as i32;
        present(t.hwnd, &pm, x, y + dy, (ease * 255.0) as u8);
    }

    fn toast_msg(&mut self, hwnd: HWND, msg: u32, l: LPARAM) {
        let Some(i) = self.toasts.iter().position(|t| t.hwnd == hwnd) else { return };
        let s = scale(hwnd);
        let (lx, ly) = lparam_xy(l);
        let (x, y) = (lx as f32 / s, ly as f32 / s);
        let hit = menu::toast_layout(&self.toasts[i].spec);
        let btn = hit.buttons.iter().position(|b| x >= b.0 && x <= b.0 + b.2 && y >= b.1 && y <= b.1 + b.3);
        let on_close = menu::has_close(&self.toasts[i].spec)
            && ((x - hit.close.0).powi(2) + (y - hit.close.1).powi(2)).sqrt() <= hit.close.2 + 2.0;
        match msg {
            WM_TIMER => {
                let t = &mut self.toasts[i];
                if !t.hover && t.closing.is_none() {
                    t.elapsed += FRAME_MS as f32 / 1000.0;
                    if t.spec.timeout_s > 0.0 && t.elapsed >= t.spec.timeout_s {
                        t.closing = Some(Instant::now());
                    }
                }
                if t.closing.is_some_and(|c| c.elapsed().as_secs_f32() > 0.2) {
                    let t = self.toasts.remove(i);
                    unsafe {
                        let _ = DestroyWindow(t.hwnd);
                    }
                    self.emit(UiEvent::ToastClosed { id: t.spec.id });
                }
                self.layout_toasts();
            }
            WM_MOUSEMOVE => {
                let t = &mut self.toasts[i];
                if !t.tracking {
                    track_leave(hwnd);
                    t.tracking = true;
                }
                t.hover = true;
                t.hover_btn = btn;
            }
            WM_MOUSELEAVE => {
                let t = &mut self.toasts[i];
                (t.hover, t.hover_btn, t.tracking) = (false, None, false);
            }
            WM_LBUTTONUP => {
                let t = &mut self.toasts[i];
                if let Some(b) = btn {
                    let action = t.spec.buttons[b].0.clone();
                    let id = t.spec.id;
                    t.closing = Some(Instant::now());
                    self.emit(UiEvent::ToastButton { id, action });
                } else if on_close {
                    t.closing = Some(Instant::now());
                }
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn pill_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER => {
            let handled = with_ui(|ui| match w.0 {
                TIMER_ANIM => ui.tick(),
                TIMER_CLICK => {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_CLICK);
                    }
                    ui.emit(UiEvent::Click);
                }
                TIMER_PEEK => {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_PEEK);
                    }
                    if ui.pill.hover && ui.pill.drag.is_none() && ui.pill.state == PillState::Idle {
                        ui.pill.peek = true;
                        ui.kick();
                    }
                }
                _ => {}
            });
            if handled.is_some() {
                return LRESULT(0);
            }
        }
        WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONUP => {
            if with_ui(|ui| ui.pill_mouse(msg, l)).is_some() {
                return LRESULT(0);
            }
        }
        WM_DPICHANGED => {
            with_ui(|ui| ui.render_pill());
            return LRESULT(0);
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if matches!(msg, WM_TIMER | WM_MOUSEMOVE | WM_LBUTTONUP | WM_KEYDOWN | WM_ACTIVATE) {
        with_ui(|ui| ui.menu_msg(msg, w, l));
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

unsafe extern "system" fn toast_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER | WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONUP => {
            with_ui(|ui| ui.toast_msg(hwnd, msg, l));
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
