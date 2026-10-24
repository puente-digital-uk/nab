//! The one place where an optional, closed module (Nab Pro) can plug into the free tool.
//!
//! With no module installed (`NoPro`, which is what the GitHub download uses) every hook below
//! does nothing, so the free tool behaves exactly as it always has. A module can only *add*:
//! it can take over where a grab goes (`deliver`), add a card to the Nab window, add tray menu
//! entries, own a few hotkeys and ask for a region of the screen. It cannot change how the free
//! features work, and everything here runs on the UI thread unless it says otherwise.
//!
//! The rules a module has to keep are in CONTRIBUTING.md; the important one is that nothing the
//! free tool does may wait for the module.

use std::sync::OnceLock;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT};
use windows::Win32::UI::Shell::NIM_MODIFY;

pub use crate::blocks::Grab;
use crate::{capture, home, logic, sys};

/// Menu ids a module may use start here (the free tool's own start below it).
pub const MENU_ID_MIN: usize = 100;
/// Hotkey ids a module may register (the free tool's own are below this range).
pub const HOTKEY_ID_MIN: i32 = 100;
pub const HOTKEY_ID_MAX: i32 = 199;

/// A rectangle in virtual-screen pixels: what GetSystemMetrics(SM_XVIRTUALSCREEN) and friends use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

/// A grab the module dealt with itself: what to tell the user.
pub struct Handled {
    pub title: String,
    pub body: String,
}

/// The module's card in the Nab window.
pub struct Card {
    pub title: String,
    pub line: String,
    /// A button on the right of the card, if there is something to press.
    pub button: Option<String>,
}

/// One tray menu entry. An empty `text` is a separator; entries with `children` open a submenu.
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    pub id: usize,
    pub text: String,
    pub checked: bool,
    pub enabled: bool,
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn item(id: usize, text: impl Into<String>) -> Self {
        MenuItem { id, text: text.into(), enabled: true, ..Default::default() }
    }

    pub fn separator() -> Self {
        MenuItem::default()
    }

    pub fn submenu(text: impl Into<String>, children: Vec<MenuItem>) -> Self {
        MenuItem { text: text.into(), enabled: true, children, ..Default::default() }
    }
}

/// What a closed module provides. Every method has a do-nothing default.
///
/// All of these are called on the UI thread, so none may block: no waiting on the Store, a
/// network, a file dialog's result from another thread, or a lock another thread can hold for
/// long. Do slow work on your own thread and come back with `post`.
pub trait Pro: Sync {
    /// Once, after the tray icon exists and before the message loop starts.
    fn start(&self) {}

    /// A grab was read. Return `Some` to take it over: Nab then puts nothing on the clipboard and
    /// shows your message (or the error). Return `None` to leave it to the free tool.
    fn deliver(&self, _grab: &Grab) -> Option<Result<Handled, String>> {
        None
    }

    /// The card at the bottom of the Nab window. `None` (the default) means no card at all.
    fn card(&self) -> Option<Card> {
        None
    }

    /// The card's button was pressed. `hwnd` is the Nab window, the owner for anything modal.
    fn card_action(&self, _hwnd: HWND) {}

    /// The tray submenu's title, and its entries (ids from `MENU_ID_MIN`). Empty = no submenu.
    fn menu_title(&self) -> &str {
        ""
    }
    fn menu(&self) -> Vec<MenuItem> {
        vec![]
    }
    fn menu_action(&self, _id: usize) {}

    /// A hotkey registered with `register_hotkey` was pressed.
    fn hotkey(&self, _id: i32) {}

    /// The person is setting the free tool's own keyboard shortcut to this combination
    /// (`mods` as MOD_* flags). If you hold it, let go of it: the free tool's shortcut wins.
    fn yield_hotkey(&self, _mods: u32, _vk: u32) {}

    /// The user drew a box after `pick_region`.
    fn region_picked(&self, _region: ScreenRect) {}
}

/// No closed module: the free tool on its own.
pub struct NoPro;
impl Pro for NoPro {}

static PRO: OnceLock<&'static dyn Pro> = OnceLock::new();

pub(crate) fn install(pro: &'static dyn Pro) {
    let _ = PRO.set(pro); // the first one wins; `run` is called once
}

pub(crate) fn pro() -> &'static dyn Pro {
    PRO.get().copied().unwrap_or(&NoPro)
}

/// A closure sent to the UI thread by `post`. It travels through the mailbox, so a made-up
/// message from another process can't make Nab run anything.
pub struct Call(Box<dyn FnOnce() + Send>);

impl Call {
    pub(crate) fn run(self) {
        (self.0)()
    }
}

// ---- What a module can ask of Nab ---------------------------------------------------------------

/// Run `f` on the UI thread, later. Safe to call from any thread.
pub fn post(f: impl FnOnce() + Send + 'static) {
    crate::mailbox::post(crate::WM_EXT_CALL, Call(Box::new(f)));
}

/// The hidden window that owns the tray icon, the hotkeys and the clipboard.
pub fn host() -> HWND {
    crate::host()
}

/// A tray notification.
pub fn notify(title: &str, body: &str) {
    sys::tray(NIM_MODIFY, Some((title, body)));
}

/// Put text on the clipboard; with `table`, also the HTML version Excel and Word paste as cells.
pub fn set_clipboard(text: &str, table: Option<&[Vec<String>]>) -> Result<(), String> {
    let html = table.map(logic::html_table);
    sys::set_clipboard(text, html.as_deref())
}

/// A saved setting of the module's own (it can't read or overwrite Nab's).
pub fn setting(name: &str) -> Option<String> {
    sys::setting(&format!("Pro{name}"))
}

pub fn save_setting(name: &str, value: &str) {
    sys::save_setting(&format!("Pro{name}"), value);
}

/// Register a global hotkey (`id` in `HOTKEY_ID_MIN..=HOTKEY_ID_MAX`, `mods` as MOD_* flags).
/// False if the id is out of range or another app already has the combination.
pub fn register_hotkey(id: i32, mods: u32, vk: u32) -> bool {
    if !(HOTKEY_ID_MIN..=HOTKEY_ID_MAX).contains(&id) {
        return false;
    }
    unsafe {
        let _ = UnregisterHotKey(crate::host(), id); // re-registering the same id is fine
        RegisterHotKey(crate::host(), id, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk).is_ok()
    }
}

pub fn unregister_hotkey(id: i32) {
    if (HOTKEY_ID_MIN..=HOTKEY_ID_MAX).contains(&id) {
        unsafe {
            let _ = UnregisterHotKey(crate::host(), id);
        }
    }
}

/// Bring up the Nab window.
pub fn show_home() {
    home::show();
}

/// Repaint the Nab window if it is open (the card's text changed).
pub fn refresh_home() {
    home::refresh();
}

/// Freeze the screen and let the user drag a box; it comes back through `Pro::region_picked`.
/// Esc cancels, and then nothing comes back. False if the screen couldn't be captured.
pub fn pick_region() -> bool {
    capture::pick_region()
}

/// Read a rectangle of the screen as a dragged box would be (tables included) and deliver it
/// like any other grab. False if the area is unusable or another grab is on screen.
pub fn grab_rect(r: ScreenRect) -> bool {
    capture::grab_rect(r)
}

/// The whole virtual screen, all monitors together.
pub fn screen_bounds() -> ScreenRect {
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        ScreenRect { l: x, t: y, r: x + GetSystemMetrics(SM_CXVIRTUALSCREEN), b: y + GetSystemMetrics(SM_CYVIRTUALSCREEN) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_module_every_hook_does_nothing() {
        let none = NoPro;
        assert!(none.deliver(&Grab::default()).is_none(), "the free tool must keep the clipboard");
        assert!(none.card().is_none());
        assert!(none.menu().is_empty());
        assert_eq!(none.menu_title(), "");
        // The installed module defaults to the free tool when `run` hasn't installed one.
        assert!(pro().card().is_none());
        assert!(pro().deliver(&Grab::default()).is_none());
    }

    #[test]
    fn hotkey_ids_outside_the_modules_range_are_refused() {
        assert!(!register_hotkey(1, 0, 0x41)); // the free tool's own id
        assert!(!register_hotkey(HOTKEY_ID_MAX + 1, 0, 0x41));
        assert!(!register_hotkey(-5, 0, 0x41));
        unregister_hotkey(1); // must not touch the free tool's hotkey
    }

    #[test]
    fn menu_helpers() {
        assert!(MenuItem::separator().text.is_empty());
        let m = MenuItem::submenu("Regions", vec![MenuItem::item(MENU_ID_MIN, "One")]);
        assert!(m.enabled && m.children.len() == 1 && m.id == 0);
        assert!(MenuItem::item(MENU_ID_MIN, "x").enabled);
    }
}
