//! The two shortcuts the user can configure:
//! - keyboard: a normal global hotkey (RegisterHotKey), opens the selection screen;
//! - mouse: modifier keys + a mouse button, via a low-level mouse hook. Click = copy the
//!   text block under the pointer, drag = draw a box. The hook lives on its own thread that
//!   does nothing else, so a busy UI can never make Windows time it out and drop it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::Mutex;

use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::trigger::{self, Button, Input, Trigger};
use crate::{logic, sys};

const HOTKEY_ID: i32 = 1;
const KEY_SETTING: &str = "KeyShortcut";
const MOUSE_SETTING: &str = "MouseShortcut";

static KEY: AtomicU32 = AtomicU32::new(0);
static KEY_OK: AtomicBool = AtomicBool::new(false);
static MOUSE: AtomicU32 = AtomicU32::new(0);
static MOUSE_OK: AtomicBool = AtomicBool::new(false); // the hook is installed and its thread is alive
static PAUSED: AtomicBool = AtomicBool::new(false); // while the user is picking a new shortcut
static ACTIVE: AtomicBool = AtomicBool::new(false); // we swallowed a button-down; swallow its button-up too
static HOOK: Mutex<Option<(u32, std::thread::JoinHandle<()>)>> = Mutex::new(None);

pub fn key() -> Option<Trigger> {
    trigger::decode(KEY.load(SeqCst))
}

pub fn mouse() -> Option<Trigger> {
    trigger::decode(MOUSE.load(SeqCst))
}

/// The keyboard shortcut is set *and* Windows let us have it.
pub fn key_ok() -> bool {
    KEY_OK.load(SeqCst)
}

/// The mouse shortcut is set *and* the hook that makes it work is installed.
pub fn mouse_ok() -> bool {
    MOUSE_OK.load(SeqCst)
}

/// One line for the tray tip and notifications, e.g. "Ctrl+Shift+Left click text, or press Alt+Space".
pub fn how_to_grab() -> String {
    let m = mouse().filter(|_| mouse_ok()).map(|t| format!("{} text", trigger::format(Some(t))));
    let k = key_ok().then(|| format!("press {}", trigger::format(key())));
    match (m, k) {
        (Some(m), Some(k)) => format!("{m}, or {k}"),
        (Some(m), None) => m,
        (None, Some(k)) => k,
        (None, None) => "click the tray icon".into(),
    }
}

/// Load the saved shortcuts (or the defaults) and switch them on.
pub fn init() {
    let load = |name: &str, default: Trigger, mouse: bool| match sys::setting(name).as_deref().and_then(trigger::parse) {
        Some(None) => None, // "Off"
        Some(Some(t)) if trigger::validate(t).is_ok() && matches!(t.input, Input::Mouse(_)) == mouse => Some(t),
        _ => Some(default),
    };
    KEY.store(trigger::encode(load(KEY_SETTING, trigger::DEFAULT_KEY, false)), SeqCst);
    refresh_key();
    let m = load(MOUSE_SETTING, trigger::DEFAULT_MOUSE, true);
    MOUSE.store(trigger::encode(m), SeqCst);
    hook(m.is_some());
}

pub fn shutdown() {
    hook(false);
    unsafe {
        let _ = UnregisterHotKey(crate::host(), HOTKEY_ID);
    }
}

/// Register the current keyboard shortcut, ignoring the pause. True if Windows accepted it.
fn try_register() -> bool {
    unsafe {
        let _ = UnregisterHotKey(crate::host(), HOTKEY_ID);
        match key() {
            Some(Trigger { mods, input: Input::Key(vk) }) => {
                RegisterHotKey(crate::host(), HOTKEY_ID, HOT_KEY_MODIFIERS(mods as u32) | MOD_NOREPEAT, vk as u32).is_ok()
            }
            _ => false,
        }
    }
}

/// Make the registration match the current shortcut and pause state.
fn refresh_key() {
    KEY_OK.store(try_register(), SeqCst);
    if PAUSED.load(SeqCst) {
        unsafe {
            let _ = UnregisterHotKey(crate::host(), HOTKEY_ID);
        }
    }
}

/// Stop both shortcuts from firing while the user is choosing a new one.
pub fn pause(on: bool) {
    PAUSED.store(on, SeqCst);
    refresh_key();
}

/// Change the keyboard shortcut. If another app already owns it, keep the old one.
pub fn set_key(t: Option<Trigger>) -> Result<(), String> {
    let old = KEY.swap(trigger::encode(t), SeqCst);
    if t.is_some() && !try_register() {
        KEY.store(old, SeqCst);
        refresh_key();
        return Err(format!("Another app already uses {}.", trigger::format(t)));
    }
    refresh_key();
    sys::save_setting(KEY_SETTING, &trigger::format(t));
    Ok(())
}

pub fn set_mouse(t: Option<Trigger>) {
    MOUSE.store(trigger::encode(t), SeqCst);
    sys::save_setting(MOUSE_SETTING, &trigger::format(t));
    hook(t.is_some());
}

/// Which modifier keys are held right now.
pub fn held_mods() -> u8 {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
    let mut m = 0;
    for (vk, bit) in [(VK_CONTROL, trigger::CTRL), (VK_MENU, trigger::ALT), (VK_SHIFT, trigger::SHIFT), (VK_LWIN, trigger::WIN), (VK_RWIN, trigger::WIN)] {
        if down(vk) {
            m |= bit;
        }
    }
    m
}

/// Mouse message -> (button, is_down).
pub fn button_event(msg: u32, mouse_data: u32) -> Option<(Button, bool)> {
    let x = || match (mouse_data >> 16) & 0xffff {
        1 => Some(Button::Back),
        2 => Some(Button::Forward),
        _ => None,
    };
    Some(match msg {
        WM_LBUTTONDOWN => (Button::Left, true),
        WM_LBUTTONUP => (Button::Left, false),
        WM_RBUTTONDOWN => (Button::Right, true),
        WM_RBUTTONUP => (Button::Right, false),
        WM_MBUTTONDOWN => (Button::Middle, true),
        WM_MBUTTONUP => (Button::Middle, false),
        WM_XBUTTONDOWN => (x()?, true),
        WM_XBUTTONUP => (x()?, false),
        _ => return None,
    })
}

/// Start or stop the hook thread.
fn hook(on: bool) {
    let Ok(mut slot) = HOOK.lock() else { return };
    if slot.as_ref().is_some_and(|(_, h)| h.is_finished()) {
        // The hook couldn't be installed (or its thread died): clear it so we try again.
        if let Some((_, h)) = slot.take() {
            let _ = h.join();
        }
    }
    match (on, slot.is_some()) {
        (true, false) => {
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("nab-mouse-hook".into()).spawn(move || unsafe {
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE); // create the queue first
                let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), crate::instance(), 0);
                let _ = tx.send((GetCurrentThreadId(), hook.is_ok()));
                if let Ok(hook) = hook {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                    let _ = UnhookWindowsHookEx(hook);
                }
                MOUSE_OK.store(false, SeqCst);
            });
            match spawned {
                Ok(handle) => {
                    let (id, ok) = rx.recv().unwrap_or((0, false));
                    MOUSE_OK.store(ok, SeqCst);
                    *slot = Some((id, handle));
                }
                Err(_) => MOUSE_OK.store(false, SeqCst),
            }
        }
        (false, true) => {
            if let Some((id, handle)) = slot.take() {
                unsafe {
                    let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
                let _ = handle.join();
            }
            MOUSE_OK.store(false, SeqCst);
            ACTIVE.store(false, SeqCst);
        }
        _ => {}
    }
}

#[cfg(test)]
pub fn hook_running() -> bool {
    HOOK.lock().map(|s| s.as_ref().is_some_and(|(_, h)| !h.is_finished())).unwrap_or(false)
}

/// Runs for every mouse event on the desktop: must be quick. Only swallows our exact gesture.
unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
        if let (Some(Trigger { mods, input: Input::Mouse(want) }), Some((button, down))) = (mouse(), button_event(wp.0 as u32, info.mouseData)) {
            if button == want {
                let post = |msg| {
                    let _ = PostMessageW(crate::host(), msg, WPARAM(0), LPARAM(logic::pack_point(info.pt.x, info.pt.y)));
                };
                if down && !PAUSED.load(SeqCst) && held_mods() == mods {
                    ACTIVE.store(true, SeqCst);
                    post(crate::WM_MOUSE_DOWN);
                    return LRESULT(1);
                }
                if !down && ACTIVE.swap(false, SeqCst) {
                    post(crate::WM_MOUSE_UP);
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_events_decode() {
        assert_eq!(button_event(WM_LBUTTONDOWN, 0), Some((Button::Left, true)));
        assert_eq!(button_event(WM_RBUTTONUP, 0), Some((Button::Right, false)));
        assert_eq!(button_event(WM_MBUTTONDOWN, 0), Some((Button::Middle, true)));
        assert_eq!(button_event(WM_XBUTTONDOWN, 1 << 16), Some((Button::Back, true)));
        assert_eq!(button_event(WM_XBUTTONUP, 2 << 16), Some((Button::Forward, false)));
        assert_eq!(button_event(WM_XBUTTONDOWN, 3 << 16), None);
        assert_eq!(button_event(WM_MOUSEMOVE, 0), None);
        assert_eq!(button_event(WM_MOUSEWHEEL, 120 << 16), None);
    }

    #[test]
    fn hook_thread_starts_and_stops_cleanly() {
        let _serial = crate::serial();
        for _ in 0..3 {
            hook(true);
            hook(true); // idempotent
            assert!(hook_running());
            hook(false);
            hook(false);
            assert!(!hook_running());
        }
    }

    #[test]
    fn how_to_grab_mentions_what_is_on() {
        let _serial = crate::serial();
        MOUSE.store(trigger::encode(Some(trigger::DEFAULT_MOUSE)), SeqCst);
        KEY.store(trigger::encode(Some(trigger::DEFAULT_KEY)), SeqCst);
        KEY_OK.store(true, SeqCst);
        MOUSE_OK.store(true, SeqCst);
        assert_eq!(how_to_grab(), "Ctrl+Shift+Left click text, or press Alt+Space");
        KEY_OK.store(false, SeqCst);
        assert_eq!(how_to_grab(), "Ctrl+Shift+Left click text");
        MOUSE_OK.store(false, SeqCst); // configured but the hook isn't running: don't advertise it
        assert_eq!(how_to_grab(), "click the tray icon");
        MOUSE_OK.store(true, SeqCst);
        MOUSE.store(0, SeqCst);
        assert_eq!(how_to_grab(), "click the tray icon");
        MOUSE_OK.store(false, SeqCst);
    }
}
