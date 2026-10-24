//! Nab by Puente Digital — copy text from anything on your screen.
//! Ctrl+Shift+click a block of text (or drag a box), and it's on your clipboard.
//! Uses Windows' built-in OCR: offline, no downloads.
//!
//! This crate is the whole free tool. `run` starts it; `ext` is the one place where an optional,
//! closed module (Nab Pro) can plug in. With no module installed, nothing in `ext` does anything.
#![deny(unreachable_patterns, unused_variables)] // a typo in a WM_ constant silently becomes a catch-all

mod blocks;
mod capture;
pub mod ext;
mod home;
mod logic;
mod ocr;
mod shortcuts;
mod sys;
mod trigger;

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub use shortcuts::how_to_grab;

pub const SITE: &str = "https://www.puentedigital.co.uk/";
pub const COFFEE: &str = "https://buymeacoffee.com/puentedigital";
pub const REPO: &str = "https://github.com/puente-digital-uk/nab";

const HOST_CLASS: PCWSTR = w!("NabHost");
pub const WM_TRAY: u32 = WM_APP + 1;
pub const WM_OCR_DONE: u32 = WM_APP + 2;
pub const WM_SHOW_HOME: u32 = WM_APP + 3;
pub const WM_SCAN_DONE: u32 = WM_APP + 4;
pub const WM_MOUSE_DOWN: u32 = WM_APP + 5;
pub const WM_MOUSE_UP: u32 = WM_APP + 6;
pub const WM_EXT_CALL: u32 = WM_APP + 7;

pub const TIMER_GRAB: usize = 1;

const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;

const CMD_GRAB: usize = 1;
const CMD_HOME: usize = 2;
const CMD_COFFEE: usize = 3;
const CMD_SITE: usize = 4;
const CMD_EXIT: usize = 5;

static HOST: AtomicIsize = AtomicIsize::new(0);

/// Tests that touch the desktop, hotkeys, clipboard or settings take this, so they never overlap.
#[cfg(test)]
pub fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn host() -> HWND {
    HWND(HOST.load(Ordering::SeqCst) as _)
}

/// Results handed from worker threads to the UI thread. The message carries a pointer, but only
/// pointers this mailbox issued are accepted back, so a stray or hostile `PostMessage` to the host
/// window with a made-up lParam is ignored instead of crashing the app.
pub mod mailbox {
    use std::any::TypeId;
    use std::sync::{Mutex, MutexGuard};
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    type Key = (u32, usize, TypeId); // message, pointer, what it points to
    static LIVE: Mutex<Vec<Key>> = Mutex::new(Vec::new());

    fn live() -> MutexGuard<'static, Vec<Key>> {
        LIVE.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn claim(key: Key) -> bool {
        let mut l = live();
        match l.iter().position(|k| *k == key) {
            Some(i) => {
                l.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// Post `value` to the host window as `msg`.
    pub fn post<T: Send + 'static>(msg: u32, value: T) {
        if crate::host().0.is_null() {
            return; // no window to receive it (a null hwnd would post to this worker thread's own queue)
        }
        let ptr = Box::into_raw(Box::new(value));
        let key = (msg, ptr as usize, TypeId::of::<T>());
        live().push(key);
        if unsafe { PostMessageW(crate::host(), msg, WPARAM(0), LPARAM(ptr as isize)) }.is_err() && claim(key) {
            drop(unsafe { Box::from_raw(ptr) });
        }
    }

    /// Take back a `T` that `post` sent as `msg`, or None if `lp` isn't one of ours, was already
    /// taken, or holds a different type.
    pub fn take<T: 'static>(msg: u32, lp: LPARAM) -> Option<T> {
        if lp.0 == 0 || !claim((msg, lp.0 as usize, TypeId::of::<T>())) {
            return None;
        }
        Some(*unsafe { Box::from_raw(lp.0 as *mut T) })
    }
}

pub fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(Into::into).unwrap_or_default() }
}

/// Is `pid` running the same executable as us?
unsafe fn is_our_exe(pid: u32) -> bool {
    let Ok(ours) = std::env::current_exe() else { return false };
    let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
    let _ = CloseHandle(process);
    ok && len as usize <= buf.len() && String::from_utf16_lossy(&buf[..len as usize]).eq_ignore_ascii_case(&ours.to_string_lossy())
}

/// Register window classes and create the hidden host window that owns the tray icon,
/// the hotkey and the clipboard.
unsafe fn create_host() -> Option<HWND> {
    let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); // fails harmlessly if already set
    let inst = instance();
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(host_proc), hInstance: inst, lpszClassName: HOST_CLASS, ..Default::default() });
    capture::register(inst);
    home::register(inst);
    let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), HOST_CLASS, w!("Nab"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, inst, None).ok()?;
    HOST.store(hwnd.0 as isize, Ordering::SeqCst);
    Some(hwnd)
}

/// Run Nab until the user exits it. `pro` is the optional closed module (`ext::NoPro` for none).
pub fn run(pro: &'static dyn ext::Pro) {
    ext::install(pro);
    unsafe {
        // One copy only: a second launch just opens the existing window. The window has to belong
        // to a real nab.exe, though: anyone can create a window with our class name, and a squatter
        // mustn't be able to keep Nab from starting or be handed the right to take the foreground.
        let _mutex = CreateMutexW(None, true, w!("Local\\PuenteDigital.Nab"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(existing) = FindWindowW(HOST_CLASS, None) {
                let mut pid = 0;
                GetWindowThreadProcessId(existing, Some(&mut pid));
                if is_our_exe(pid) {
                    let _ = AllowSetForegroundWindow(pid); // so it can come to the front
                    let _ = PostMessageW(existing, WM_SHOW_HOME, WPARAM(0), LPARAM(0));
                    return;
                }
            } else {
                return; // a real instance that is still starting up (or exiting); don't double up
            }
        }
        if create_host().is_none() {
            return;
        }
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::SeqCst);

        shortcuts::init();
        sys::tray(NIM_ADD, None);
        sys::refresh_autostart();
        ext::pro().start();

        let background = std::env::args_os().any(|a| a == "--background") || sys::launched_at_startup();
        if !background || (!shortcuts::key_ok() && !shortcuts::mouse_ok()) {
            home::show();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        shortcuts::shutdown();
        sys::tray(NIM_DELETE, None);
    }
}

extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_HOTKEY if wp.0 as i32 == shortcuts::HOTKEY_ID => {
                capture::start(false);
            }
            WM_MOUSE_DOWN => {
                let (x, y) = logic::unpack_point(lp.0);
                capture::mouse_down(x, y);
            }
            WM_MOUSE_UP => {
                let (x, y) = logic::unpack_point(lp.0);
                capture::mouse_up(x, y);
            }
            WM_HOTKEY if (ext::HOTKEY_ID_MIN..=ext::HOTKEY_ID_MAX).contains(&(wp.0 as i32)) => ext::pro().hotkey(wp.0 as i32),
            WM_SCAN_DONE => capture::scan_done(lp),
            WM_OCR_DONE => ocr::report(lp),
            WM_EXT_CALL => {
                if let Some(call) = mailbox::take::<ext::Call>(WM_EXT_CALL, lp) {
                    call.run();
                }
            }
            WM_TIMER if wp.0 == TIMER_GRAB => {
                let _ = KillTimer(hwnd, TIMER_GRAB);
                capture::start(false);
            }
            WM_SHOW_HOME => home::show(),
            // NOTIFYICON_VERSION_4: event in LOWORD(lParam). One handler per gesture, or menus double up.
            WM_TRAY => match (lp.0 & 0xffff) as u32 {
                NIN_SELECT | NIN_KEYSELECT => home::show(),
                WM_CONTEXTMENU => tray_menu(hwnd, logic::unpack_point(wp.0 as isize)),
                _ => {}
            },
            _ if msg == TASKBAR_CREATED.load(Ordering::SeqCst) && msg != 0 => sys::tray(NIM_ADD, None), // Explorer restarted
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

fn command(cmd: usize) {
    match cmd {
        CMD_GRAB => unsafe {
            capture::start(false);
        },
        CMD_HOME => home::show(),
        CMD_COFFEE => sys::open(COFFEE),
        CMD_SITE => sys::open(SITE),
        CMD_EXIT => unsafe { PostQuitMessage(0) },
        id if id >= ext::MENU_ID_MIN => ext::pro().menu_action(id),
        _ => {}
    }
}

/// The closed module's entries, as one submenu. At most two levels, and only ids the module is allowed to use.
unsafe fn append_ext_menu(menu: HMENU, title: &str, items: &[ext::MenuItem]) {
    unsafe fn fill(menu: HMENU, items: &[ext::MenuItem], depth: u32) {
        for item in items {
            if item.text.is_empty() {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                continue;
            }
            let wide = sys::wide(&item.text);
            if !item.children.is_empty() && depth < 2 {
                if let Ok(sub) = CreatePopupMenu() {
                    fill(sub, &item.children, depth + 1);
                    let flags = MF_POPUP | if item.enabled { MENU_ITEM_FLAGS(0) } else { MF_GRAYED };
                    let _ = AppendMenuW(menu, flags, sub.0 as usize, PCWSTR(wide.as_ptr()));
                }
            } else if item.id >= ext::MENU_ID_MIN {
                let mut flags = MF_STRING;
                if item.checked {
                    flags |= MF_CHECKED;
                }
                if !item.enabled {
                    flags |= MF_GRAYED;
                }
                let _ = AppendMenuW(menu, flags, item.id, PCWSTR(wide.as_ptr()));
            }
        }
    }
    if let Ok(sub) = CreatePopupMenu() {
        fill(sub, items, 1);
        let wide = sys::wide(title);
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(wide.as_ptr())); // destroyed with `menu`
    }
}

unsafe fn tray_menu(hwnd: HWND, (x, y): (i32, i32)) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let grab = match shortcuts::key_ok() {
        true => format!("Grab text\t{}", trigger::format(shortcuts::key())),
        false => "Grab text".to_string(),
    };
    let items: [(usize, &str); 7] = [
        (CMD_GRAB, &grab),
        (CMD_HOME, "Settings…"),
        (0, ""),
        (CMD_COFFEE, "Buy me a coffee ☕"),
        (CMD_SITE, "Made by Puente Digital"),
        (0, ""),
        (CMD_EXIT, "Exit"),
    ];
    let pro_items = ext::pro().menu();
    for (id, text) in items {
        if id == CMD_EXIT && !pro_items.is_empty() {
            append_ext_menu(menu, ext::pro().menu_title(), &pro_items);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        if id == 0 {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        } else {
            let wide = sys::wide(text);
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
        }
    }
    let _ = SetMenuDefaultItem(menu, CMD_GRAB as u32, 0);
    let _ = SetForegroundWindow(hwnd); // lets the menu close when you click elsewhere
    // TPM_RETURNCMD: the choice comes back here instead of as a WM_COMMAND, which any process could forge.
    let chosen = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD, x, y, 0, hwnd, None).0;
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    if chosen > 0 {
        command(chosen as usize);
    }
}

#[cfg(test)]
mod smoke {
    //! Drives the real windows on a real Windows desktop. Each test makes its own host window
    //! (windows belong to the thread that created them, and every test runs on its own thread).
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    fn pump() {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }

    /// Pump messages until `done` or the timeout.
    fn pump_until(secs: u64, mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            pump();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn at(x: i32, y: i32) -> LPARAM {
        LPARAM(logic::pack_point(x, y))
    }

    fn clipboard() -> String {
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::*;
        unsafe {
            if OpenClipboard(None).is_err() {
                return String::new();
            }
            let mut out = String::new();
            if let Ok(h) = GetClipboardData(13) {
                let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
                if !p.is_null() {
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                    let _ = GlobalUnlock(HGLOBAL(h.0));
                }
            }
            let _ = CloseClipboard();
            out
        }
    }

    fn norm(s: &str) -> String {
        s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
    }

    /// What the app last told the user, and whether the overlay lost focus: printed when a test fails.
    fn diagnose() -> String {
        format!(
            "last notification: {:?}; overlay lost focus {} time(s); OCR language: {}",
            sys::LAST_BALLOON.lock().map(|b| b.clone()).unwrap_or_default(),
            capture::DEACTIVATED.load(Ordering::SeqCst),
            ocr::language()
        )
    }

    /// A topmost white window showing `text` in a big font, at screen (x, y).
    unsafe fn show_text(text: &str, x: i32, y: i32, w: i32, h: i32) -> (HWND, HFONT) {
        let t = sys::wide(text);
        let hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("STATIC"), PCWSTR(t.as_ptr()), WS_POPUP | WS_VISIBLE, x, y, w, h, None, None, instance(), None).unwrap();
        let font = CreateFontW(-34, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
        SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
        let _ = UpdateWindow(hwnd);
        pump();
        std::thread::sleep(Duration::from_millis(400)); // let the compositor put it on screen
        pump();
        (hwnd, font)
    }

    #[test]
    fn overlay_and_home_survive_every_gesture() {
        let _serial = serial();
        unsafe {
            let host = create_host().unwrap();

            // Open, drag, paint, cancel — several times, to catch leaks/state bugs.
            for cancel in [WM_RBUTTONUP, WM_CLOSE, WM_KEYDOWN, WM_RBUTTONUP] {
                assert!(capture::start(false));
                assert!(capture::is_open());
                assert!(capture::start(false)); // second press while open is a no-op
                let ov = FindWindowW(w!("NabOverlay"), None).unwrap();
                SendMessageW(ov, WM_LBUTTONDOWN, WPARAM(1), at(10, 10));
                for i in 0..20 {
                    SendMessageW(ov, WM_MOUSEMOVE, WPARAM(1), at(10 + i * 15, 10 + i * 5));
                    let _ = UpdateWindow(ov);
                }
                let wp = if cancel == WM_KEYDOWN { WPARAM(VK_ESCAPE.0 as usize) } else { WPARAM(0) };
                SendMessageW(ov, cancel, wp, LPARAM(0));
                pump();
                assert!(!capture::is_open(), "cancel via {cancel:#x} left the overlay open");
                assert!(FindWindowW(w!("NabOverlay"), None).is_err());
            }

            // Hovering (with and without scan results) and stray button-ups must not crash or close.
            assert!(capture::start(false));
            let ov = FindWindowW(w!("NabOverlay"), None).unwrap();
            SendMessageW(ov, WM_LBUTTONUP, WPARAM(0), at(50, 50));
            assert!(capture::is_open());
            for i in 0..60 {
                SendMessageW(ov, WM_MOUSEMOVE, WPARAM(0), at(i * 17, i * 9));
                let _ = UpdateWindow(ov);
                pump();
                std::thread::sleep(Duration::from_millis(15));
            }
            assert!(capture::is_open());

            // A real drag: selection -> OCR thread -> result posted back to the host.
            let before = ocr::REPORTS.load(Ordering::SeqCst);
            SendMessageW(ov, WM_LBUTTONDOWN, WPARAM(1), at(0, 0));
            SendMessageW(ov, WM_MOUSEMOVE, WPARAM(1), at(300, 120));
            SendMessageW(ov, WM_LBUTTONUP, WPARAM(0), at(300, 120));
            assert!(!capture::is_open());
            let got = pump_until(15, || ocr::REPORTS.load(Ordering::SeqCst) != before);
            assert!(got, "OCR result never came back");

            // The home window opens, paints, reacts to hover, and closes.
            home::show();
            home::show(); // second call just focuses it
            let hw = FindWindowW(w!("NabHome"), None).unwrap();
            let _ = UpdateWindow(hw);
            for (x, y) in [(100, 130), (360, 134), (360, 186), (100, 290), (300, 290), (200, 368), (60, 419), (340, 419), (100, 478), (350, 478), (5, 5)] {
                SendMessageW(hw, WM_MOUSEMOVE, WPARAM(0), at(x, y));
                let _ = UpdateWindow(hw);
            }
            SendMessageW(hw, WM_CLOSE, WPARAM(0), LPARAM(0));
            pump();
            assert!(FindWindowW(w!("NabHome"), None).is_err());
            let _ = DestroyWindow(host);
        }
    }

    #[test]
    fn click_on_text_copies_that_block() {
        let _serial = serial();
        unsafe {
            let host = create_host().unwrap();
            // A plain window covering the whole virtual screen, with the text in its top-left
            // corner. Nothing else can be in the captured image, so a click far from the text is
            // blank by construction, whatever the desktop (or the console this test runs in)
            // happens to be showing, at any resolution or scaling.
            let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
            assert!(w >= 800 && h >= 600, "the screen is too small for this test: {w}x{h}");
            let (win, font) = show_text("Quarterly revenue grew twelve percent", x, y, w, h);
            let _ = sys::set_clipboard("sentinel", None);

            // Exactly what the mouse shortcut does: press and release at the same screen point,
            // on the text itself (one line of 34px type along the top).
            capture::mouse_down(x + 160, y + 20);
            assert!(capture::is_open());
            capture::mouse_up(x + 160, y + 20);
            let ok = pump_until(20, || norm(&clipboard()) == "quarterlyrevenuegrewtwelvepercent");
            assert!(ok, "clipboard was {:?}; {}", clipboard(), diagnose());
            assert!(pump_until(3, || !capture::is_open()), "overlay should close after the green flash");

            // Clicking blank space (mouse shortcut) closes without touching the clipboard. Give the
            // desktop a moment first, so the capture can't catch a frame of the overlay just closed.
            let (bx, by) = (x + w / 2, y + h - 150);
            let _ = sys::set_clipboard("sentinel", None);
            pump_until(1, || false);
            capture::mouse_down(bx, by);
            capture::mouse_up(bx, by);
            assert!(pump_until(20, || !capture::is_open()), "a click on nothing should close the overlay");
            pump_until(1, || false);
            assert_eq!(clipboard(), "sentinel", "a click on blank space at ({bx}, {by}) of {w}x{h} copied something; {}", diagnose());

            // Dragging with the mouse shortcut still reads a box: from just outside the text's
            // top-left corner to below and right of it, as people do.
            capture::mouse_down(x + 10, y + 8);
            let ov = FindWindowW(w!("NabOverlay"), None).unwrap();
            for i in 1..10 {
                SendMessageW(ov, WM_MOUSEMOVE, WPARAM(0), at(x + 10 + i * 100, y + 8 + i * 12));
            }
            capture::mouse_up(x + 1000, y + 140);
            assert!(!capture::is_open(), "a drag closes straight away");
            let ok = pump_until(20, || norm(&clipboard()).contains("quarterlyrevenue"));
            assert!(ok, "clipboard was {:?}; {}", clipboard(), diagnose());

            let _ = DestroyWindow(win);
            let _ = DeleteObject(font);
            let _ = DestroyWindow(host);
        }
    }

    /// Paints a small table: three columns at fixed positions, like a report in a PDF.
    extern "system" fn table_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            if msg == WM_PAINT {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let _ = PatBlt(dc, 0, 0, 2000, 2000, WHITENESS);
                let font = CreateFontW(-26, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET.0 as u32, 0, 0, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
                let old = SelectObject(dc, font);
                for (r, row) in TABLE.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        let t: Vec<u16> = cell.encode_utf16().collect();
                        let _ = TextOutW(dc, 20 + c as i32 * 260, 20 + r as i32 * 48, &t);
                    }
                }
                SelectObject(dc, old);
                let _ = DeleteObject(font);
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
    const TABLE: [[&str; 3]; 4] = [["Region", "Sales", "Growth"], ["North", "1200", "12%"], ["South", "980", "4%"], ["West", "770", "9%"]];

    #[test]
    fn dragging_over_a_table_copies_cells() {
        let _serial = serial();
        unsafe {
            let host = create_host().unwrap();
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(table_proc), hInstance: instance(), lpszClassName: w!("NabTestTable"), ..Default::default() });
            let win = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("NabTestTable"), w!(""), WS_POPUP | WS_VISIBLE, 100, 100, 860, 240, None, None, instance(), None).unwrap();
            let _ = UpdateWindow(win);
            pump_until(1, || false);
            sys::set_clipboard("sentinel", None).unwrap();

            let before = ocr::REPORTS.load(Ordering::SeqCst);
            capture::mouse_down(104, 104);
            capture::mouse_up(950, 335);
            assert!(pump_until(20, || ocr::REPORTS.load(Ordering::SeqCst) != before), "nothing was read");
            assert!(pump_until(5, || {
                let c = clipboard();
                !c.is_empty() && c != "sentinel"
            }), "nothing was copied");
            let text = clipboard();
            let rows: Vec<Vec<String>> = text.lines().map(|l| l.split('\t').map(norm).collect()).collect();
            let want: Vec<Vec<String>> = TABLE.iter().map(|r| r.iter().map(|c| norm(c)).collect()).collect();
            assert_eq!(rows, want, "clipboard text was {text:?}; {}", diagnose());

            // And there's an HTML table for Excel/Word.
            use windows::Win32::System::DataExchange::*;
            let _ = OpenClipboard(None);
            let html_format = RegisterClipboardFormatW(w!("HTML Format"));
            assert!(GetClipboardData(html_format).is_ok(), "no HTML version on the clipboard");
            let _ = CloseClipboard();

            let _ = DestroyWindow(win);
            let _ = DestroyWindow(host);
        }
    }

    #[test]
    fn keyboard_overlay_click_on_nothing_stays_open() {
        let _serial = serial();
        unsafe {
            let host = create_host().unwrap();
            let (win, font) = show_text("", 100, 100, 600, 300); // blank white area
            assert!(capture::start(false));
            let ov = FindWindowW(w!("NabOverlay"), None).unwrap();
            let (ox, oy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            let (cx, cy) = (400 - ox, 250 - oy);
            SendMessageW(ov, WM_LBUTTONDOWN, WPARAM(1), at(cx, cy));
            SendMessageW(ov, WM_LBUTTONUP, WPARAM(0), at(cx, cy));
            pump_until(15, || false); // let the scan finish
            assert!(capture::is_open(), "keyboard mode keeps the overlay open after a miss; {}", diagnose());
            SendMessageW(ov, WM_KEYDOWN, WPARAM(VK_ESCAPE.0 as usize), LPARAM(0));
            pump();
            assert!(!capture::is_open());
            let _ = DestroyWindow(win);
            let _ = DeleteObject(font);
            let _ = DestroyWindow(host);
        }
    }

    #[test]
    fn host_ignores_results_it_did_not_send() {
        let _serial = serial();
        unsafe {
            let host = create_host().unwrap();
            let before = ocr::REPORTS.load(Ordering::SeqCst);
            // Made-up pointers from another process (or a broadcast) must be ignored, not dereferenced.
            for lp in [1isize, 0x1234, -8, isize::MAX] {
                SendMessageW(host, WM_OCR_DONE, WPARAM(0), LPARAM(lp));
                SendMessageW(host, WM_SCAN_DONE, WPARAM(0), LPARAM(lp));
            }
            assert_eq!(ocr::REPORTS.load(Ordering::SeqCst), before);
            // A hotkey message with someone else's id does nothing.
            SendMessageW(host, WM_HOTKEY, WPARAM(99), LPARAM(0));
            assert!(!capture::is_open());

            // A real result arrives exactly once, and only as the message type it was posted as.
            mailbox::post(WM_APP + 50, 4242u32);
            let mut m = MSG::default();
            assert!(PeekMessageW(&mut m, host, WM_APP + 50, WM_APP + 50, PM_REMOVE).as_bool());
            assert_eq!(mailbox::take::<u32>(WM_APP + 51, m.lParam), None);
            assert_eq!(mailbox::take::<i32>(WM_APP + 50, m.lParam), None); // wrong type: refused, not reinterpreted
            assert_eq!(mailbox::take::<u32>(WM_APP + 50, m.lParam), Some(4242));
            assert_eq!(mailbox::take::<u32>(WM_APP + 50, m.lParam), None);
            let _ = DestroyWindow(host);
        }
    }

    #[test]
    fn only_a_real_nab_counts_as_the_running_instance() {
        unsafe {
            assert!(is_our_exe(windows::Win32::System::Threading::GetCurrentProcessId()));
            assert!(!is_our_exe(4)); // the System process
            assert!(!is_our_exe(0xFFFF_FFF0)); // no such process
        }
    }

    #[test]
    fn shortcuts_change_and_reject_conflicts() {
        use trigger::{Input, Trigger, ALT, CTRL, SHIFT};
        let _serial = serial();
        let _wipe = sys::TestSettings;
        unsafe {
            let host = create_host().unwrap();
            shortcuts::init();
            assert!(shortcuts::mouse().is_some());

            let a = Trigger { mods: CTRL | ALT | SHIFT, input: Input::Key(0x7A) }; // Ctrl+Alt+Shift+F11
            shortcuts::set_key(Some(a)).unwrap();
            assert!(shortcuts::key_ok());
            assert_eq!(shortcuts::key(), Some(a));
            assert_eq!(sys::setting("KeyShortcut").as_deref(), Some("Ctrl+Alt+Shift+F11"));

            // Someone else grabs a combination; we must refuse it and keep ours.
            let b = Trigger { mods: CTRL | ALT | SHIFT, input: Input::Key(0x79) }; // F10
            assert!(RegisterHotKey(None, 77, HOT_KEY_MODIFIERS((CTRL | ALT | SHIFT) as u32), 0x79).is_ok());
            assert!(shortcuts::set_key(Some(b)).is_err());
            assert_eq!(shortcuts::key(), Some(a));
            assert!(shortcuts::key_ok());
            let _ = UnregisterHotKey(None, 77);

            // Pausing frees the combination for the user to press; resuming takes it back.
            shortcuts::pause(true);
            assert!(RegisterHotKey(None, 78, HOT_KEY_MODIFIERS((CTRL | ALT | SHIFT) as u32), 0x7A).is_ok());
            let _ = UnregisterHotKey(None, 78);
            shortcuts::pause(false);
            assert!(RegisterHotKey(None, 79, HOT_KEY_MODIFIERS((CTRL | ALT | SHIFT) as u32), 0x7A).is_err());

            shortcuts::set_key(None).unwrap();
            assert!(!shortcuts::key_ok());
            assert_eq!(sys::setting("KeyShortcut").as_deref(), Some("Off"));

            shortcuts::set_mouse(Some(Trigger { mods: ALT, input: Input::Mouse(trigger::Button::Middle) }));
            assert_eq!(sys::setting("MouseShortcut").as_deref(), Some("Alt+Middle click"));
            assert!(shortcuts::hook_running());
            shortcuts::set_mouse(None);
            assert!(!shortcuts::hook_running());

            // Garbage in the saved settings falls back to the defaults.
            sys::save_setting("KeyShortcut", "Ctrl+Banana");
            sys::save_setting("MouseShortcut", "Ctrl+Alt+T"); // a key where a mouse button belongs
            shortcuts::init();
            assert_eq!(shortcuts::key(), Some(trigger::DEFAULT_KEY));
            assert_eq!(shortcuts::mouse(), Some(trigger::DEFAULT_MOUSE));
            shortcuts::shutdown();
            let _ = DestroyWindow(host);
        }
    }
}
