//! The selection screen. Freezes every monitor, dims it, and lets the user either
//! - click: the block of text under the pointer is found automatically (hover shows it), or
//! - drag: draw a box.
//!
//! Finding blocks: as soon as the screen is frozen, a background thread runs OCR over each
//! monitor (the one under the pointer first) and groups the words into blocks.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::NIM_MODIFY;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::blocks::{self, Block, Grab, Rect};
use crate::{logic, ocr, sys};

const CLASS: PCWSTR = w!("NabOverlay");
const ACCENT: COLORREF = COLORREF(0x00EB6325); // #2563EB (GDI colours are BGR)
const PICKED: COLORREF = COLORREF(0x005EC522); // #22C55E
const ESC_HOTKEY: i32 = 2; // works even if Windows refuses to give the overlay focus
const TIMER_CLOSE: usize = 1;
/// Smallest box worth saving as a region or re-reading (smaller than this is a slip of the hand).
const MIN_REGION_PX: i32 = 8;
const DRAG_PX: i32 = 5; // moved less than this between press and release = a click

/// A 32-bit top-down DIB we can draw on with GDI *and* read/write as plain memory.
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u8,
}

impl Surface {
    unsafe fn new(w: i32, h: i32) -> Option<Self> {
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(None, &bmi(w, h), DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(bmp);
            return None;
        }
        let dc = CreateCompatibleDC(None);
        SelectObject(dc, bmp);
        Some(Surface { dc, bmp, bits: bits as *mut u8 })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(self.bmp);
        }
    }
}

struct Monitor {
    rect: Rect, // in overlay coordinates
    scanned: bool,
}

struct Snap {
    hwnd: HWND,
    origin: (i32, i32), // top-left of the virtual screen, in screen coordinates
    w: i32,
    h: i32,
    bright: Surface, // the frozen screen
    dim: Surface,    // darkened copy + hint
    back: Surface,   // back buffer (no flicker)
    gen: u32,
    start: Option<(i32, i32)>, // button down here, not released yet
    cur: (i32, i32),
    monitors: Vec<Monitor>,
    blocks: Vec<Block>,
    hover: Option<usize>,
    pending: Option<(i32, i32)>, // clicked before that monitor was scanned
    picked: Option<Rect>,        // flashing green, about to close
    close_on_miss: bool,         // opened by the mouse shortcut: clicking on nothing just closes
    error: Option<String>,       // why the scan found nothing, if OCR itself failed
}

/// Results from the background scan of one monitor.
struct Scan {
    gen: u32,
    monitor: usize,
    blocks: Vec<Block>,
    error: Option<String>, // OCR itself failed (e.g. no language installed)
}

/// One overlay's monitors, waiting to be scanned.
struct Job {
    gen: u32,
    monitors: Vec<(usize, Rect, Vec<u8>)>,
}

thread_local!(static SNAP: RefCell<Option<Snap>> = const { RefCell::new(None) });
static GEN: AtomicU32 = AtomicU32::new(0);
// One scan worker, one pending job: opening and closing the overlay quickly can't pile up
// scans (each monitor costs ~100 MB while it's being read). A newer job replaces the pending one.
static PENDING: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static WORKER: AtomicBool = AtomicBool::new(false);
/// The next dragged box is handed to the closed module (`ext::pick_region`) instead of being read.
static PICK: AtomicBool = AtomicBool::new(false);
/// How often an overlay lost focus (and so closed itself): lets tests explain a failure.
#[cfg(test)]
pub static DEACTIVATED: AtomicU32 = AtomicU32::new(0);

pub fn register(inst: HINSTANCE) {
    unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst,
            lpszClassName: CLASS,
            ..Default::default() // no class cursor: WM_SETCURSOR picks one
        });
    }
}

pub fn is_open() -> bool {
    SNAP.with(|s| s.borrow().is_some())
}

/// Open the selection screen to pick a region for the closed module (see `ext`).
pub fn pick_region() -> bool {
    PICK.store(true, Ordering::SeqCst);
    let ok = unsafe { start(false) };
    if !ok {
        PICK.store(false, Ordering::SeqCst);
    }
    ok
}

/// Read a rectangle of the live screen, as a dragged box would be, and deliver it like any grab.
pub fn grab_rect(r: crate::ext::ScreenRect) -> bool {
    let (w, h) = (r.r - r.l, r.b - r.t);
    let bounds = crate::ext::screen_bounds();
    let inside = r.l >= bounds.l && r.t >= bounds.t && r.r <= bounds.r && r.b <= bounds.b;
    if is_open() || !inside || w < MIN_REGION_PX || h < MIN_REGION_PX {
        return false;
    }
    unsafe {
        let Some(shot) = Surface::new(w, h) else { return false };
        let screen = GetDC(None);
        let copied = BitBlt(shot.dc, 0, 0, w, h, screen, r.l, r.t, SRCCOPY | CAPTUREBLT).is_ok();
        ReleaseDC(None, screen);
        let _ = GdiFlush();
        if !copied {
            return false;
        }
        ocr::spawn(copy_out(&shot, w, &Rect::new(0, 0, w, h)), w as u32, h as u32, None);
    }
    true
}

fn bmi(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn to_rect(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

fn to_win(r: &Rect) -> RECT {
    RECT { left: r.l, top: r.t, right: r.r, bottom: r.b }
}

unsafe extern "system" fn add_monitor(_: HMONITOR, _: HDC, r: *mut RECT, list: LPARAM) -> BOOL {
    (*(list.0 as *mut Vec<Rect>)).push(to_rect(&*r));
    true.into()
}

/// Copy a rectangle of a surface out as tightly packed BGRA.
unsafe fn copy_out(s: &Surface, stride_px: i32, r: &Rect) -> Vec<u8> {
    let stride = stride_px as usize * 4;
    let mut out = Vec::with_capacity(r.w() as usize * r.h() as usize * 4);
    for row in r.t..r.b {
        let p = s.bits.add(row as usize * stride + r.l as usize * 4);
        out.extend_from_slice(std::slice::from_raw_parts(p, r.w() as usize * 4));
    }
    out
}

/// Freeze the screen and show the overlay. Returns false if it couldn't.
pub unsafe fn start(close_on_miss: bool) -> bool {
    if is_open() {
        return true;
    }
    let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
    let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
    if w <= 0 || h <= 0 {
        return false;
    }
    let (Some(bright), Some(dim), Some(back)) = (Surface::new(w, h), Surface::new(w, h), Surface::new(w, h)) else {
        sys::tray(NIM_MODIFY, Some(("Nab", "Couldn't capture the screen (out of memory).")));
        return false;
    };

    let screen = GetDC(None);
    let _ = BitBlt(bright.dc, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
    ReleaseDC(None, screen);
    let _ = GdiFlush();

    // Darkened background, built once, straight in memory.
    let len = w as usize * h as usize * 4;
    std::ptr::copy_nonoverlapping(bright.bits, dim.bits, len);
    logic::dim(std::slice::from_raw_parts_mut(dim.bits, len), 45);
    draw_hint(dim.dc, -x, -y, PICK.load(Ordering::SeqCst)); // primary monitor's top-left in overlay coordinates

    // Monitors, the one under the pointer first, so a click there is answered soonest.
    let mut rects: Vec<Rect> = vec![];
    let _ = EnumDisplayMonitors(None, None, Some(add_monitor), LPARAM(&mut rects as *mut _ as isize));
    let mut rects: Vec<Rect> = rects.iter().map(|r| r.offset(-x, -y).clamp(w, h)).filter(|r| r.w() > 0 && r.h() > 0).collect();
    if rects.is_empty() {
        rects.push(Rect::new(0, 0, w, h));
    }
    let mut cursor = POINT::default();
    let _ = GetCursorPos(&mut cursor);
    rects.sort_by_key(|r| !r.contains(cursor.x - x, cursor.y - y));

    let gen = GEN.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    let monitors_px = rects.iter().enumerate().map(|(i, r)| (i, *r, copy_out(&bright, w, r))).collect();
    submit(Job { gen, monitors: monitors_px });

    let Ok(hwnd) = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, CLASS, w!("Nab"), WS_POPUP, x, y, w, h, None, None, crate::instance(), None) else {
        GEN.fetch_add(1, Ordering::SeqCst); // the job submitted above is now stale
        return false;
    };
    let _ = RegisterHotKey(hwnd, ESC_HOTKEY, MOD_NOREPEAT, VK_ESCAPE.0 as u32);
    let monitors = rects.into_iter().map(|rect| Monitor { rect, scanned: false }).collect();
    SNAP.with(|s| {
        *s.borrow_mut() = Some(Snap {
            hwnd,
            origin: (x, y),
            w,
            h,
            bright,
            dim,
            back,
            gen,
            start: None,
            cur: (0, 0),
            monitors,
            blocks: vec![],
            hover: None,
            pending: None,
            picked: None,
            close_on_miss,
            error: None,
        })
    });
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
    true
}

/// Hand a job to the scan worker (starting it the first time), replacing any job still waiting.
fn submit(job: Job) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    *pending = Some(job);
    drop(pending);
    if !WORKER.swap(true, Ordering::SeqCst) && std::thread::Builder::new().name("nab-scan".into()).spawn(scan_worker).is_err() {
        WORKER.store(false, Ordering::SeqCst);
        GEN.fetch_add(1, Ordering::SeqCst); // the overlay stays usable for drags; there just won't be hover blocks
    }
    WAKE.notify_one();
}

/// Scans each monitor of the newest job and posts the blocks to the UI thread, forever.
fn scan_worker() {
    loop {
        let job = {
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = pending.take() {
                    break job;
                }
                pending = WAKE.wait(pending).unwrap_or_else(|e| e.into_inner());
            }
        };
        for (i, r, px) in job.monitors {
            if GEN.load(Ordering::SeqCst) != job.gen {
                break; // that overlay has closed (or a newer one opened): don't burn CPU on it
            }
            let (words, error) = match ocr::words(&px, r.w() as u32, r.h() as u32) {
                Ok(words) => (words, None),
                Err(e) => (vec![], Some(e.message())),
            };
            let words: Vec<_> = words.into_iter().map(|mut wd| {
                wd.rect = wd.rect.offset(r.l, r.t);
                wd
            }).collect();
            crate::mailbox::post(crate::WM_SCAN_DONE, Scan { gen: job.gen, monitor: i, blocks: blocks::blocks(&words), error });
        }
    }
}

/// "Click on text or drag a box · Esc to cancel" pill, near the top of the primary monitor.
unsafe fn draw_hint(dc: HDC, px: i32, py: i32, pick: bool) {
    let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    let sc = |v: i32| v * dx as i32 / 96;
    let pw = GetSystemMetrics(SM_CXSCREEN);
    let (bw, bh) = (sc(470), sc(58));
    let r = RECT { left: px + (pw - bw) / 2, top: py + sc(24), right: px + (pw + bw) / 2, bottom: py + sc(24) + bh };

    let brush = CreateSolidBrush(COLORREF(0x00221A14)); // #141A22
    let pen = CreatePen(PS_SOLID, 1, COLORREF(0x00433A33));
    let (ob, op) = (SelectObject(dc, brush), SelectObject(dc, pen));
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, sc(18), sc(18));
    SelectObject(dc, ob);
    SelectObject(dc, op);
    let _ = DeleteObject(brush);
    let _ = DeleteObject(pen);

    SetBkMode(dc, TRANSPARENT);
    let line = if pick { "Drag a box around the region to save  ·  Esc to cancel" } else { "Click on text, or drag a box  ·  Esc to cancel" };
    text(dc, line, sc(16), 600, COLORREF(0x00FFFFFF), RECT { top: r.top + sc(8), bottom: r.top + sc(34), ..r });
    text(dc, "Nab · free by Puente Digital", sc(12), 400, COLORREF(0x00B8A99A), RECT { top: r.top + sc(32), bottom: r.bottom - sc(4), ..r });
}

unsafe fn text(dc: HDC, s: &str, px: i32, weight: i32, color: COLORREF, mut r: RECT) {
    let font = CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
    let old = SelectObject(dc, font);
    SetTextColor(dc, color);
    let mut wide: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(dc, &mut wide, &mut r, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    SelectObject(dc, old);
    let _ = DeleteObject(font);
}

fn mouse(lp: LPARAM) -> (i32, i32) {
    logic::unpack_point(lp.0)
}

fn sel_rect(a: (i32, i32), b: (i32, i32)) -> Rect {
    Rect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

unsafe fn invalidate(hwnd: HWND, r: Option<Rect>) {
    if let Some(r) = r {
        let _ = InvalidateRect(hwnd, Some(&to_win(&r.inflate(4))), false);
    }
}

/// Free everything, then close the window. Safe to call more than once.
unsafe fn close() {
    PICK.store(false, Ordering::SeqCst); // Esc, right-click or a lost focus cancels picking a region
    if let Some(s) = SNAP.with(|s| s.borrow_mut().take()) {
        GEN.fetch_add(1, Ordering::SeqCst); // tells its scan thread to stop
        let _ = UnregisterHotKey(s.hwnd, ESC_HOTKEY);
        let _ = KillTimer(s.hwnd, TIMER_CLOSE);
        let _ = ReleaseCapture();
        let _ = DestroyWindow(s.hwnd);
        // Surfaces are freed when `s` drops here.
    }
}

fn hover_rect(s: &Snap) -> Option<Rect> {
    s.hover.map(|i| s.blocks[i].padded().clamp(s.w, s.h))
}

/// Move the hover highlight to whatever block is under (x, y).
unsafe fn update_hover(s: &mut Snap, x: i32, y: i32) {
    if s.start.is_some() || s.picked.is_some() {
        return;
    }
    let new = blocks::pick(&s.blocks, x, y);
    if new != s.hover {
        invalidate(s.hwnd, hover_rect(s));
        s.hover = new;
        invalidate(s.hwnd, hover_rect(s));
    }
}

/// Mouse button pressed (on the overlay, or via the mouse shortcut). Point in overlay coordinates.
pub unsafe fn press(x: i32, y: i32) {
    SNAP.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            if s.picked.is_none() {
                invalidate(s.hwnd, hover_rect(s));
                s.hover = None;
                s.pending = None;
                s.start = Some((x, y));
                s.cur = (x, y);
            }
        }
    });
}

enum Then {
    Nothing,
    Close,
    Read(Vec<u8>, u32, u32, Option<Grab>),
    Region(crate::ext::ScreenRect), // a box for the closed module, not to be read
    Miss(String),   // close and say why nothing was copied
    Notify(String), // stay open, tell the user something (OCR failed)
}

const NO_TEXT: &str = "No text found under the pointer. Try dragging a box around it.";

/// Mouse button released: a drag reads the box, a click picks the block under the pointer.
pub unsafe fn release(x: i32, y: i32) {
    let then = SNAP.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return Then::Nothing };
        let Some(a) = s.start.take() else { return Then::Nothing }; // up without down: ignore
        if (x - a.0).abs() > DRAG_PX || (y - a.1).abs() > DRAG_PX {
            let Some((sx, sy, sw, sh)) = logic::selection(a, (x, y), s.w, s.h) else {
                // Too thin to read. From the mouse shortcut, just stop; from the keyboard, let them retry.
                invalidate(s.hwnd, Some(sel_rect(a, s.cur)));
                return if s.close_on_miss { Then::Close } else { Then::Nothing };
            };
            let r = Rect::new(sx as i32, sy as i32, (sx + sw) as i32, (sy + sh) as i32);
            if PICK.swap(false, Ordering::SeqCst) {
                let (ox, oy) = s.origin;
                return Then::Region(crate::ext::ScreenRect { l: ox + r.l, t: oy + r.t, r: ox + r.r, b: oy + r.b });
            }
            return Then::Read(copy_out(&s.bright, s.w, &r), sw, sh, None);
        }
        invalidate(s.hwnd, Some(sel_rect(a, s.cur)));
        if PICK.load(Ordering::SeqCst) {
            return Then::Nothing; // choosing a region: only a dragged box counts
        }
        s.pending = Some((x, y));
        resolve(s)
    });
    finish(then);
}

/// If a click is waiting and its monitor has been scanned, act on it.
unsafe fn resolve(s: &mut Snap) -> Then {
    let Some((x, y)) = s.pending else { return Then::Nothing };
    let ready = match s.monitors.iter().find(|m| m.rect.contains(x, y)) {
        Some(m) => m.scanned,
        None => s.monitors.iter().all(|m| m.scanned), // in a gap between monitors
    };
    if !ready {
        SetCursor(LoadCursorW(None, IDC_APPSTARTING).unwrap_or_default());
        return Then::Nothing;
    }
    s.pending = None;
    match blocks::pick(&s.blocks, x, y) {
        Some(i) => {
            let r = s.blocks[i].padded().clamp(s.w, s.h);
            if r.w() < 1 || r.h() < 1 {
                return Then::Miss(NO_TEXT.into());
            }
            s.hover = None;
            s.picked = Some(r);
            invalidate(s.hwnd, Some(r));
            let _ = SetTimer(s.hwnd, TIMER_CLOSE, 280, None);
            let fallback = Grab { text: s.blocks[i].text.clone(), table: None };
            Then::Read(copy_out(&s.bright, s.w, &r), r.w() as u32, r.h() as u32, Some(fallback))
        }
        None if s.close_on_miss => Then::Miss(s.error.clone().unwrap_or_else(|| NO_TEXT.into())),
        None => {
            update_hover(s, x, y); // keyboard mode: nothing there, stay open

            Then::Nothing
        }
    }
}

/// Do what release/resolve decided, outside of any borrow (closing re-enters the window proc).
unsafe fn finish(then: Then) {
    match then {
        Then::Nothing => {}
        Then::Close => close(),
        Then::Read(px, w, h, fallback) => {
            let flashing = SNAP.with(|s| s.borrow().as_ref().is_some_and(|s| s.picked.is_some()));
            if !flashing {
                close();
            }
            ocr::spawn(px, w, h, fallback);
        }
        Then::Region(region) => {
            close();
            crate::ext::pro().region_picked(region);
        }
        Then::Miss(why) => {
            close();
            sys::tray(NIM_MODIFY, Some(("Nab", &why)));
        }
        Then::Notify(what) => sys::tray(NIM_MODIFY, Some(("Nab", &what))),
    }
}

/// A monitor finished scanning (posted from the scan thread to the host window).
pub unsafe fn scan_done(lp: LPARAM) {
    let Some(scan) = crate::mailbox::take::<Scan>(crate::WM_SCAN_DONE, lp) else { return };
    let then = SNAP.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut().filter(|s| s.gen == scan.gen) else { return Then::Nothing };
        if let Some(m) = s.monitors.get_mut(scan.monitor) {
            m.scanned = true;
        }
        s.blocks.extend(scan.blocks);
        let fresh_error = scan.error.filter(|_| s.error.is_none());
        if fresh_error.is_some() {
            s.error = fresh_error.clone();
        }
        let mut c = POINT::default();
        let _ = GetCursorPos(&mut c);
        update_hover(s, c.x - s.origin.0, c.y - s.origin.1);
        match (resolve(s), fresh_error) {
            (Then::Nothing, Some(e)) => Then::Notify(e), // keyboard mode: say it once, stay open
            (then, _) => then,
        }
    });
    finish(then);
}

/// The mouse shortcut, in screen coordinates: opens the overlay if needed and starts a press.
pub unsafe fn mouse_down(sx: i32, sy: i32) {
    if !is_open() && !start(true) {
        return;
    }
    let o = SNAP.with(|s| s.borrow().as_ref().map(|s| s.origin));
    if let Some((ox, oy)) = o {
        press(sx - ox, sy - oy);
    }
}

pub unsafe fn mouse_up(sx: i32, sy: i32) {
    let o = SNAP.with(|s| s.borrow().as_ref().map(|s| s.origin));
    if let Some((ox, oy)) = o {
        release(sx - ox, sy - oy);
    }
}

extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                SNAP.with(|s| {
                    if let Some(s) = s.borrow().as_ref() {
                        paint(s, hdc, ps.rcPaint);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
            }
            WM_SETCURSOR => {
                let busy = SNAP.with(|s| s.borrow().as_ref().map(|s| (s.pending.is_some(), s.hover.is_some() && s.start.is_none())));
                let cursor = match busy {
                    Some((true, _)) => IDC_APPSTARTING,
                    Some((_, true)) => IDC_HAND,
                    _ => IDC_CROSS,
                };
                SetCursor(LoadCursorW(None, cursor).unwrap_or_default());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = mouse(lp);
                press(x, y);
                SetCapture(hwnd);
            }
            WM_MOUSEMOVE => {
                let (x, y) = mouse(lp);
                SNAP.with(|s| {
                    if let Some(s) = s.borrow_mut().as_mut() {
                        if let Some(a) = s.start {
                            // Repaint only what changed: old box ∪ new box.
                            let dirty = sel_rect(a, s.cur).union(&sel_rect(a, (x, y)));
                            s.cur = (x, y);
                            invalidate(hwnd, Some(dirty));
                        } else if s.pending.is_none() {
                            update_hover(s, x, y);
                        }
                    }
                });
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = mouse(lp);
                release(x, y);
            }
            WM_TIMER if wp.0 == TIMER_CLOSE => close(),
            WM_HOTKEY if wp.0 as i32 == ESC_HOTKEY => close(),
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => close(),
            WM_RBUTTONUP => close(), // on *up*, so the click doesn't land on the app underneath
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                #[cfg(test)]
                DEACTIVATED.fetch_add(1, Ordering::SeqCst);
                // Alt-tab etc.: close, but not from inside the activation change itself.
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            WM_CLOSE | WM_DESTROY => close(),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// Show `r` at full brightness with a border of `color`.
unsafe fn highlight(s: &Snap, r: &Rect, color: COLORREF, width: i32) {
    let _ = BitBlt(s.back.dc, r.l, r.t, r.w(), r.h(), s.bright.dc, r.l, r.t, SRCCOPY);
    let pen = CreatePen(PS_SOLID, width, color);
    let (op, ob) = (SelectObject(s.back.dc, pen), SelectObject(s.back.dc, GetStockObject(NULL_BRUSH)));
    let _ = Rectangle(s.back.dc, r.l - 1, r.t - 1, r.r + 1, r.b + 1);
    SelectObject(s.back.dc, op);
    SelectObject(s.back.dc, ob);
    let _ = DeleteObject(pen);
}

unsafe fn paint(s: &Snap, hdc: HDC, dirty: RECT) {
    let (l, t, w, h) = (dirty.left, dirty.top, dirty.right - dirty.left, dirty.bottom - dirty.top);
    let _ = BitBlt(s.back.dc, l, t, w, h, s.dim.dc, l, t, SRCCOPY);
    if let Some(r) = s.picked {
        highlight(s, &r, PICKED, 3);
    } else if let Some(a) = s.start {
        highlight(s, &sel_rect(a, s.cur), ACCENT, 2);
    } else if let Some(r) = hover_rect(s) {
        highlight(s, &r, ACCENT, 2);
    }
    let _ = BitBlt(hdc, l, t, w, h, s.back.dc, l, t, SRCCOPY);
}
