//! The Nab window: shortcuts (changeable), "grab now", start-with-Windows, and the Puente Digital bits.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::NIM_MODIFY;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::trigger::{self, Button, Input, Trigger};
use crate::{shortcuts, sys};

const CLASS: PCWSTR = w!("NabHome");
const SIZE: (i32, i32) = (440, 508);
const WM_MOUSELEAVE: u32 = 0x02A3; // not exported by windows-rs under these features
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}
const BG: COLORREF = rgb(0x0F, 0x14, 0x1B);
const CARD: COLORREF = rgb(0x19, 0x21, 0x2B);
const LINE: COLORREF = rgb(0x2A, 0x34, 0x41);
const TEXT: COLORREF = rgb(0xF3, 0xF6, 0xFA);
const MUTED: COLORREF = rgb(0x9A, 0xA7, 0xB6);
const WARN: COLORREF = rgb(0xFB, 0xBF, 0x24);
const BLUE: (COLORREF, COLORREF) = (rgb(0x25, 0x63, 0xEB), rgb(0x3B, 0x76, 0xF6));
const COFFEE: (COLORREF, COLORREF) = (rgb(0xFF, 0xDD, 0x00), rgb(0xFF, 0xE5, 0x4D));
const LINK: (COLORREF, COLORREF) = (rgb(0x5E, 0xEA, 0xD4), rgb(0x99, 0xF6, 0xE4));

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    ChangeMouse,
    ChangeKey,
    Info, // the ⓘ next to "Time saved": hovering it explains the counter
    Grab,
    Coffee,
    Autostart,
    Site,
    Source,
}
const HITS: [Hit; 8] = [Hit::ChangeMouse, Hit::ChangeKey, Hit::Info, Hit::Grab, Hit::Coffee, Hit::Autostart, Hit::Site, Hit::Source];

/// Which shortcut the user is currently choosing.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Rec {
    Mouse,
    Key,
}

struct Home {
    hwnd: HWND,
    dpi: i32,
    hover: Option<Hit>,
    rec: Option<Rec>,
    note: Option<String>, // a problem with what they just pressed
    swallow_up: bool,     // the click that set a mouse shortcut mustn't also press a button
}

thread_local!(static HOME: RefCell<Option<Home>> = const { RefCell::new(None) });
static TOLD_ABOUT_TRAY: AtomicBool = AtomicBool::new(false);

/// Layout in 96-dpi units (left, top, right, bottom).
fn area(h: Hit) -> (i32, i32, i32, i32) {
    match h {
        Hit::ChangeMouse => (332, 118, 396, 150),
        Hit::ChangeKey => (332, 170, 396, 202),
        Hit::Info => (128, 264, 146, 282),
        Hit::Grab => (32, 346, 408, 390),
        Hit::Autostart => (32, 408, 240, 430),
        Hit::Coffee => (268, 404, 408, 434), // small, out of the way of the main button
        Hit::Site => (32, 464, 290, 492),
        Hit::Source => (300, 464, 408, 492),
    }
}

fn scaled(dpi: i32, (l, t, r, b): (i32, i32, i32, i32)) -> RECT {
    let s = |v: i32| v * dpi / 96;
    RECT { left: s(l), top: s(t), right: s(r), bottom: s(b) }
}

pub fn register(inst: HINSTANCE) {
    unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst,
            hIcon: sys::icon(32),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        });
    }
}

pub fn show() {
    unsafe {
        if let Some(hwnd) = HOME.with(|h| h.borrow().as_ref().map(|h| h.hwnd)) {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            return;
        }
        let dpi = GetDpiForSystem() as i32;
        let mut r = scaled(dpi, (0, 0, SIZE.0, SIZE.1));
        let _ = AdjustWindowRectExForDpi(&mut r, STYLE, false, WINDOW_EX_STYLE(0), dpi as u32);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (x, y) = ((GetSystemMetrics(SM_CXSCREEN) - w) / 2, (GetSystemMetrics(SM_CYSCREEN) - h) / 2);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, w!("Nab"), STYLE, x, y, w, h, None, None, crate::instance(), None) else {
            return;
        };
        // The system DPI can be stale (scaling changed without signing out); use the real one.
        let real = GetDpiForWindow(hwnd) as i32;
        let dpi = if real > 0 && real != dpi {
            let mut r = scaled(real, (0, 0, SIZE.0, SIZE.1));
            let _ = AdjustWindowRectExForDpi(&mut r, STYLE, false, WINDOW_EX_STYLE(0), real as u32);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let (x, y) = ((GetSystemMetrics(SM_CXSCREEN) - w) / 2, (GetSystemMetrics(SM_CYSCREEN) - h) / 2);
            let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            real
        } else {
            dpi
        };
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        HOME.with(|h| *h.borrow_mut() = Some(Home { hwnd, dpi, hover: None, rec: None, note: None, swallow_up: false }));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Repaint the window if it's open (e.g. the time-saved counter just went up).
pub fn refresh() {
    if let Some(hwnd) = HOME.with(|h| h.borrow().as_ref().map(|h| h.hwnd)) {
        unsafe { redraw(hwnd) }; // InvalidateRect only queues a paint: safe with no borrow held
    }
}

fn hit_test(dpi: i32, lp: LPARAM) -> Option<Hit> {
    let (x, y) = crate::logic::unpack_point(lp.0);
    HITS.into_iter().find(|&h| {
        let r = scaled(dpi, area(h));
        x >= r.left && x < r.right && y >= r.top && y < r.bottom
    })
}

fn with<R>(f: impl FnOnce(&mut Home) -> R) -> Option<R> {
    HOME.with(|h| h.borrow_mut().as_mut().map(f))
}

unsafe fn redraw(hwnd: HWND) {
    let _ = InvalidateRect(hwnd, None, false);
}

/// Start (or stop) choosing a new shortcut. While choosing, both shortcuts are paused,
/// so pressing the current one doesn't fire it.
fn record(hwnd: HWND, rec: Option<Rec>) {
    with(|h| {
        h.rec = rec;
        h.note = None;
    });
    shortcuts::pause(rec.is_some());
    unsafe { redraw(hwnd) };
}

/// The user pressed something while choosing. Apply it, or explain why not.
fn offer(hwnd: HWND, t: Option<Trigger>) {
    let Some(rec) = with(|h| h.rec).flatten() else { return };
    let result = match t {
        Some(t) => trigger::validate(t).map_err(str::to_string).and_then(|_| match rec {
            Rec::Key => shortcuts::set_key(Some(t)),
            Rec::Mouse => {
                shortcuts::set_mouse(Some(t));
                Ok(())
            }
        }),
        None => match rec {
            Rec::Key => shortcuts::set_key(None),
            Rec::Mouse => {
                shortcuts::set_mouse(None);
                Ok(())
            }
        },
    };
    match result {
        Ok(()) => {
            record(hwnd, None);
            sys::tray(NIM_MODIFY, None); // refresh the tray tooltip
        }
        Err(e) => {
            with(|h| h.note = Some(e));
            unsafe { redraw(hwnd) };
        }
    }
}

fn act(hwnd: HWND, h: Hit) {
    match h {
        Hit::ChangeMouse | Hit::ChangeKey => {
            let want = if h == Hit::ChangeMouse { Rec::Mouse } else { Rec::Key };
            let now = with(|h| h.rec).flatten();
            record(hwnd, if now == Some(want) { None } else { Some(want) }); // the button reads "Cancel" while choosing
        }
        Hit::Grab => unsafe {
            // Hide first so the window isn't in the screenshot, then grab a moment later.
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = SetTimer(crate::host(), crate::TIMER_GRAB, 250, None);
        },
        Hit::Info => {} // hover does the work
        Hit::Coffee => sys::open(crate::COFFEE),
        Hit::Site => sys::open(crate::SITE),
        Hit::Source => sys::open(crate::REPO),
        Hit::Autostart => {
            sys::set_autostart(!sys::autostart_enabled());
            if sys::autostart_locked() {
                sys::tray(NIM_MODIFY, Some(("Nab", "Windows has switched off Nab's startup. Turn it on in Settings › Apps › Startup.")));
            }
            unsafe { redraw(hwnd) };
        }
    }
}

fn mouse_button(msg: u32, wp: WPARAM) -> Option<Button> {
    Some(match msg {
        WM_LBUTTONDOWN => Button::Left,
        WM_RBUTTONDOWN => Button::Right,
        WM_MBUTTONDOWN => Button::Middle,
        WM_XBUTTONDOWN => match (wp.0 >> 16) & 0xffff {
            1 => Button::Back,
            2 => Button::Forward,
            _ => return None,
        },
        _ => return None,
    })
}

extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some((dpi, hover, rec)) = with(|h| (h.dpi, h.hover, h.rec)) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        let set_hover = |v: Option<Hit>| {
            if v != hover {
                with(|h| h.hover = v);
                redraw(hwnd);
            }
        };
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some((note, hover)) = with(|h| (h.note.clone(), h.hover)) {
                    paint(hwnd, hdc, dpi, hover, rec, note.as_deref());
                }
                let _ = EndPaint(hwnd, &ps);
            }
            WM_MOUSEMOVE => {
                if hover.is_none() {
                    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                }
                set_hover(hit_test(dpi, lp));
            }
            WM_MOUSELEAVE => set_hover(None),
            WM_SETCURSOR if hover.is_some_and(|h| h != Hit::Info) && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                SetCursor(LoadCursorW(None, IDC_HAND).unwrap_or_default());
                return LRESULT(1);
            }

            // Choosing a mouse shortcut: hold the keys and click any button in the window.
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN if rec == Some(Rec::Mouse) => {
                let cancel = matches!(hit_test(dpi, lp), Some(Hit::ChangeMouse));
                let mods = shortcuts::held_mods();
                if let (false, Some(b)) = (cancel && mods == 0, mouse_button(msg, wp)) {
                    if b == Button::Left {
                        with(|h| h.swallow_up = true); // only a left button-up would press a button
                    }
                    offer(hwnd, Some(Trigger { mods, input: Input::Mouse(b) }));
                }
                return LRESULT(if msg == WM_XBUTTONDOWN { 1 } else { 0 });
            }
            WM_LBUTTONUP => {
                if with(|h| std::mem::take(&mut h.swallow_up)) == Some(true) {
                    return LRESULT(0);
                }
                match hit_test(dpi, lp) {
                    Some(h) => act(hwnd, h),
                    None if rec.is_some() => record(hwnd, None), // clicking elsewhere cancels
                    None => {}
                }
            }

            // Choosing a keyboard shortcut (or Esc / Delete while choosing either).
            // Held keys auto-repeat: only the first press counts (else a held Esc cancels, then closes).
            WM_KEYDOWN | WM_SYSKEYDOWN if lp.0 & (1 << 30) != 0 => return LRESULT(0),
            WM_KEYDOWN | WM_SYSKEYDOWN if rec.is_some() => {
                let vk = wp.0 as u16;
                let mods = shortcuts::held_mods();
                match (vk, mods) {
                    (trigger::VK_ESCAPE, 0) => record(hwnd, None),
                    (trigger::VK_DELETE | trigger::VK_BACK, 0) => offer(hwnd, None), // turn it off
                    _ if trigger::is_modifier(vk) => {}
                    _ if rec == Some(Rec::Key) => offer(hwnd, Some(Trigger { mods, input: Input::Key(vk) })),
                    _ => {
                        with(|h| h.note = Some("Hold the keys, then click a mouse button.".into()));
                        redraw(hwnd);
                    }
                }
                return LRESULT(0); // also stops Alt/F10 from opening the window menu
            }
            WM_SYSCHAR if rec.is_some() => return LRESULT(0),
            WM_SYSKEYUP if matches!(wp.0 as u16, 0x12 | 0x79) => return LRESULT(0), // Alt / F10: no window-menu focus
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE && rec.is_some() => record(hwnd, None),

            WM_KEYDOWN if wp.0 as u16 == VK_RETURN.0 => act(hwnd, Hit::Grab),
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            WM_DPICHANGED => {
                // Windows passes a suggested RECT in lParam, but any process can send this message
                // with a bogus pointer, so size the window ourselves from the new DPI instead.
                let new_dpi = ((wp.0 >> 16) & 0xffff) as i32;
                if (48..=960).contains(&new_dpi) {
                    with(|h| h.dpi = new_dpi);
                    let mut r = scaled(new_dpi, (0, 0, SIZE.0, SIZE.1));
                    let _ = AdjustWindowRectExForDpi(&mut r, STYLE, false, WINDOW_EX_STYLE(0), new_dpi as u32);
                    let _ = SetWindowPos(hwnd, None, 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
                    redraw(hwnd);
                }
            }
            WM_CLOSE => {
                if rec.is_some() {
                    record(hwnd, None);
                }
                let _ = DestroyWindow(hwnd);
                if !TOLD_ABOUT_TRAY.swap(true, Ordering::SeqCst) {
                    let body = format!("Still running in the tray — {}.", shortcuts::how_to_grab());
                    sys::tray(NIM_MODIFY, Some(("Nab", &body)));
                }
            }
            WM_DESTROY => {
                if rec.is_some() {
                    shortcuts::pause(false);
                }
                HOME.with(|h| *h.borrow_mut() = None);
            }
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

unsafe fn fill(dc: HDC, r: RECT, color: COLORREF, radius: i32, border: Option<COLORREF>) {
    let brush = CreateSolidBrush(color);
    let pen = CreatePen(PS_SOLID, 1, border.unwrap_or(color));
    let (ob, op) = (SelectObject(dc, brush), SelectObject(dc, pen));
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    let _ = DeleteObject(brush);
    let _ = DeleteObject(pen);
}

/// Run `f` with a Segoe UI font of `px` pixels and `weight` selected into `dc`.
unsafe fn with_font<R>(dc: HDC, px: i32, weight: i32, f: impl FnOnce() -> R) -> R {
    let font = CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
    let old = SelectObject(dc, font);
    let r = f();
    SelectObject(dc, old);
    let _ = DeleteObject(font);
    r
}

/// Width of one line of text.
unsafe fn measure(dc: HDC, s: &str, px: i32, weight: i32) -> i32 {
    with_font(dc, px, weight, || {
        let mut wide: Vec<u16> = s.encode_utf16().collect();
        let mut r = RECT::default();
        DrawTextW(dc, &mut wide, &mut r, DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT);
        r.right - r.left
    })
}

/// Draw one line of text in `r`. Returns its width.
unsafe fn text(dc: HDC, s: &str, px: i32, weight: i32, color: COLORREF, mut r: RECT, flags: DRAW_TEXT_FORMAT) -> i32 {
    let width = measure(dc, s, px, weight);
    with_font(dc, px, weight, || {
        SetTextColor(dc, color);
        let mut wide: Vec<u16> = s.encode_utf16().collect();
        DrawTextW(dc, &mut wide, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER | DT_END_ELLIPSIS);
    });
    width
}

/// Draw a shortcut as key-cap chips, right-aligned to `right`, vertically in [top, bottom).
unsafe fn chips(dc: HDC, dpi: i32, name: &str, right: i32, top: i32, bottom: i32) {
    let s = |v: i32| v * dpi / 96;
    let mut x = right;
    for key in name.split('+').rev() {
        let kw = measure(dc, key, s(13), 600) + s(16);
        x -= kw;
        let r = RECT { left: x, top, right: x + kw, bottom };
        fill(dc, r, BG, s(8), Some(LINE));
        text(dc, key, s(13), 600, TEXT, r, DT_CENTER);
        x -= s(6);
    }
}

/// One row of the shortcut card.
unsafe fn shortcut_row(dc: HDC, dpi: i32, top: i32, label: &str, which: Rec, rec: Option<Rec>, hover: Option<Hit>) {
    let s = |v: i32| v * dpi / 96;
    let r = |l, t, rr, b| scaled(dpi, (l, t, rr, b));
    text(dc, label, s(14), 400, MUTED, r(50, top, 200, top + 52), DT_LEFT);
    let (t, ok, problem) = match which {
        Rec::Mouse => (shortcuts::mouse(), shortcuts::mouse_ok() || shortcuts::mouse().is_none(), "Couldn't start — restart Nab"),
        Rec::Key => (shortcuts::key(), shortcuts::key_ok() || shortcuts::key().is_none(), "Taken by another app"),
    };
    if rec == Some(which) {
        let prompt = if which == Rec::Mouse { "Hold keys, click a button…" } else { "Press the new keys…" };
        text(dc, prompt, s(14), 600, LINK.0, r(130, top, 322, top + 52), DT_RIGHT);
    } else if t.is_none() {
        text(dc, "Off", s(14), 600, MUTED, r(200, top, 322, top + 52), DT_RIGHT);
    } else if !ok {
        text(dc, problem, s(13), 600, WARN, r(130, top, 322, top + 52), DT_RIGHT);
    } else {
        chips(dc, dpi, &trigger::format(t), s(322), s(top + 12), s(top + 40));
    }
    let hit = if which == Rec::Mouse { Hit::ChangeMouse } else { Hit::ChangeKey };
    let label = if rec == Some(which) { "Cancel" } else { "Change" };
    let c = if hover == Some(hit) { LINK.1 } else { LINK.0 };
    text(dc, label, s(13), 600, c, scaled(dpi, area(hit)), DT_RIGHT);
}

unsafe fn paint(hwnd: HWND, hdc: HDC, dpi: i32, hover: Option<Hit>, rec: Option<Rec>, note: Option<&str>) {
    let s = |v: i32| v * dpi / 96;
    let r = |l, t, rr, b| scaled(dpi, (l, t, rr, b));
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let (cw, ch) = (client.right, client.bottom);

    // Double-buffer the whole (small) window.
    let dc = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, cw, ch);
    let old_bmp = SelectObject(dc, bmp);
    SetBkMode(dc, TRANSPARENT);
    let bg = CreateSolidBrush(BG);
    FillRect(dc, &client, bg);
    let _ = DeleteObject(bg);

    // Header
    let _ = DrawIconEx(dc, s(32), s(28), sys::icon(s(52)), s(52), s(52), 0, None, DI_NORMAL);
    text(dc, "Nab", s(26), 700, TEXT, r(100, 24, 408, 56), DT_LEFT);
    text(dc, "Copy text from anything on your screen.", s(14), 400, MUTED, r(100, 56, 408, 80), DT_LEFT);

    // Shortcut card: two rows
    fill(dc, r(32, 108, 408, 212), CARD, s(14), Some(if rec.is_some() { BLUE.0 } else { LINE }));
    let sep = CreateSolidBrush(LINE);
    FillRect(dc, &r(48, 160, 392, 161), sep);
    shortcut_row(dc, dpi, 108, "Click or drag", Rec::Mouse, rec, hover);
    shortcut_row(dc, dpi, 160, "Keyboard", Rec::Key, rec, hover);

    // One line under the card: a problem, a hint while choosing, or how it works.
    let (line, color) = match (note, rec) {
        (Some(n), _) => (n.to_string(), WARN),
        (None, Some(_)) => ("Esc to cancel  ·  Delete to turn it off".to_string(), MUTED),
        (None, None) => ("Click on any text to copy that block — or drag a box.".to_string(), MUTED),
    };
    text(dc, &line, s(13), 400, color, r(32, 220, 408, 246), DT_LEFT);

    // Time saved: everything compared with retyping what was copied.
    let saved = sys::saved();
    fill(dc, r(32, 258, 408, 330), CARD, s(14), Some(LINE));
    text(dc, "TIME SAVED", s(11), 700, MUTED, r(50, 266, 220, 282), DT_LEFT);
    let info = scaled(dpi, area(Hit::Info));
    let info_colour = if hover == Some(Hit::Info) { LINK.1 } else { MUTED };
    fill(dc, info, CARD, s(18), Some(info_colour));
    text(dc, "i", s(11), 700, info_colour, info, DT_CENTER);
    text(dc, &crate::logic::format_saved(saved.seconds), s(22), 700, TEXT, r(50, 282, 230, 310), DT_LEFT);
    text(dc, "vs screenshot → OCR/AI tool → copy back", s(11), 400, MUTED, r(50, 310, 300, 326), DT_LEFT);
    let (nabs, keys) = match saved.nabs {
        0 => ("Nab something".to_string(), "to start the clock".to_string()),
        n => (
            format!("{} nab{}", crate::logic::thousands(n), if n == 1 { "" } else { "s" }),
            format!("{} steps skipped", crate::logic::thousands(saved.steps)),
        ),
    };
    text(dc, &nabs, s(14), 600, TEXT, r(220, 264, 392, 286), DT_RIGHT);
    text(dc, &keys, s(13), 400, MUTED, r(200, 286, 392, 306), DT_RIGHT);
    if hover == Some(Hit::Info) {
        // The explanation, as a panel above the card (so the ⓘ itself stays uncovered).
        let panel = r(40, 104, 400, 258);
        fill(dc, panel, BG, s(12), Some(MUTED));
        text(dc, "How the counter works", s(13), 600, TEXT, r(54, 112, 386, 132), DT_LEFT);
        let body = "Text you can't select usually means: screenshot it, paste it into an OCR or AI tool, wait, \
copy the result, come back. Nab is one click.\n\nIt counts that as about 20 seconds and 5 steps per copy, \
and a minute and 7 steps for a table (asking for a table and fixing the columns in Excel). \
It's an estimate: nothing is measured, and nothing is sent anywhere.";
        with_font(dc, s(12), 400, || {
            SetTextColor(dc, MUTED);
            let mut wide: Vec<u16> = body.encode_utf16().collect();
            let mut rect = r(54, 134, 386, 254);
            DrawTextW(dc, &mut wide, &mut rect, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX);
        });
    }

    // Buttons
    let pick = |h: Hit, c: (COLORREF, COLORREF)| if hover == Some(h) { c.1 } else { c.0 };
    let grab = scaled(dpi, area(Hit::Grab));
    fill(dc, grab, pick(Hit::Grab, BLUE), s(12), None);
    text(dc, "Grab text now", s(15), 600, TEXT, grab, DT_CENTER);
    let coffee = scaled(dpi, area(Hit::Coffee));
    fill(dc, coffee, pick(Hit::Coffee, COFFEE), s(15), None);
    text(dc, "☕ Buy me a coffee", s(12), 600, rgb(0x1A, 0x1A, 0x1A), coffee, DT_CENTER);

    // Start with Windows
    let on = sys::autostart_enabled();
    let auto = scaled(dpi, area(Hit::Autostart));
    let boxr = RECT { left: auto.left, top: auto.top + s(2), right: auto.left + s(18), bottom: auto.top + s(20) };
    fill(dc, boxr, if on { BLUE.0 } else { CARD }, s(6), Some(if hover == Some(Hit::Autostart) { MUTED } else { LINE }));
    if on {
        text(dc, "✓", s(13), 700, TEXT, boxr, DT_CENTER);
    }
    text(dc, "Start with Windows", s(14), 400, TEXT, RECT { left: boxr.right + s(10), ..auto }, DT_LEFT);

    // Footer
    FillRect(dc, &r(32, 450, 408, 451), sep);
    let _ = DeleteObject(sep);
    let site = scaled(dpi, area(Hit::Site));
    let wfree = text(dc, "Free, by ", s(13), 400, MUTED, site, DT_LEFT);
    text(dc, "Puente Digital ↗", s(13), 600, pick(Hit::Site, LINK), RECT { left: site.left + wfree, ..site }, DT_LEFT);
    text(dc, "Open source ↗", s(13), 400, pick(Hit::Source, (MUTED, TEXT)), scaled(dpi, area(Hit::Source)), DT_RIGHT);

    let _ = BitBlt(hdc, 0, 0, cw, ch, dc, 0, 0, SRCCOPY);
    SelectObject(dc, old_bmp);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(dc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_hit_areas_do_not_overlap() {
        for (i, a) in HITS.iter().enumerate() {
            for b in &HITS[i + 1..] {
                let (x, y) = (scaled(96, area(*a)), scaled(96, area(*b)));
                let overlap = x.left < y.right && y.left < x.right && x.top < y.bottom && y.top < x.bottom;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
            let r = area(*a);
            assert!(r.0 >= 0 && r.2 <= SIZE.0 && r.1 >= 0 && r.3 <= SIZE.1, "{a:?} is outside the window");
        }
    }

    #[test]
    fn hit_test_finds_each_area() {
        for h in HITS {
            let r = scaled(144, area(h)); // 150% scaling
            let lp = LPARAM(crate::logic::pack_point((r.left + r.right) / 2, (r.top + r.bottom) / 2));
            assert_eq!(hit_test(144, lp), Some(h));
        }
        assert_eq!(hit_test(96, LPARAM(crate::logic::pack_point(2, 2))), None);
    }
}
