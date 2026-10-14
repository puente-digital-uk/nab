//! Small Windows helpers: tray icon, clipboard, start-with-Windows, opening links.

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::{AppInstance, StartupTask, StartupTaskState};
use windows::Win32::Foundation::*;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
#[cfg(not(test))]
const RUN_VALUE: PCWSTR = w!("Nab");
#[cfg(test)]
const RUN_VALUE: PCWSTR = w!("Nab-tests"); // never touch the real start-with-Windows entry
#[cfg(not(test))]
const APP_KEY: PCWSTR = w!("Software\\PuenteDigital\\Nab");
#[cfg(test)]
const APP_KEY: PCWSTR = w!("Software\\PuenteDigital\\Nab-tests"); // never touch the real settings
const CF_UNICODETEXT: u32 = 13;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

thread_local!(static ICONS: std::cell::RefCell<Vec<(i32, HICON)>> = const { std::cell::RefCell::new(Vec::new()) });

/// The app icon at a given pixel size, loaded once per size and kept for the app's lifetime.
pub fn icon(size: i32) -> HICON {
    if let Some(h) = ICONS.with(|c| c.borrow().iter().find(|(s, _)| *s == size).map(|(_, h)| *h)) {
        return h;
    }
    let h = unsafe {
        LoadImageW(crate::instance(), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
    };
    ICONS.with(|c| c.borrow_mut().push((size, h)));
    h
}

/// The last notification shown (lets tests explain a failure).
#[cfg(test)]
pub static LAST_BALLOON: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Add / remove the tray icon, or show a notification (`balloon` = title, body).
pub fn tray(op: NOTIFY_ICON_MESSAGE, balloon: Option<(&str, &str)>) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: crate::host(),
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: crate::WM_TRAY,
            hIcon: icon(GetSystemMetrics(SM_CXSMICON)),
            ..Default::default()
        };
        crate::logic::fill_utf16(&mut nid.szTip, &format!("Nab — {}", crate::how_to_grab()));
        if let Some((title, body)) = balloon {
            nid.uFlags |= NIF_INFO;
            nid.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON | NIIF_NOSOUND;
            nid.hBalloonIcon = icon(GetSystemMetrics(SM_CXICON));
            crate::logic::fill_utf16(&mut nid.szInfoTitle, title);
            crate::logic::fill_utf16(&mut nid.szInfo, body);
        }
        #[cfg(test)]
        if let (Some((title, body)), Ok(mut last)) = (balloon, LAST_BALLOON.lock()) {
            *last = format!("{title}: {body}");
        }
        let _ = Shell_NotifyIconW(op, &nid);
        if op == NIM_ADD {
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }
}

/// Copy `bytes` into movable global memory and hand it to the clipboard as `format`.
unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.message())?;
    let ptr = GlobalLock(mem) as *mut u8;
    if ptr.is_null() {
        let _ = GlobalFree(mem);
        return Err("Out of memory".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    let _ = GlobalUnlock(mem);
    if let Err(e) = SetClipboardData(format, HANDLE(mem.0)) {
        let _ = GlobalFree(mem); // ownership only passes to Windows on success
        return Err(e.message());
    }
    Ok(())
}

/// Put text on the clipboard, plus an HTML version (a table, so Excel and Word paste real cells).
/// Retries briefly because other apps can hold the clipboard open.
pub fn set_clipboard(text: &str, html: Option<&str>) -> Result<(), String> {
    let data = crate::logic::clipboard_utf16(text);
    let bytes: Vec<u8> = data.iter().flat_map(|u| u.to_le_bytes()).collect();
    unsafe {
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(crate::host()).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !opened {
            return Err("The clipboard is busy — try again.".into());
        }
        let result = (|| {
            EmptyClipboard().map_err(|e| e.message())?;
            put(CF_UNICODETEXT, &bytes)?;
            if let Some(html) = html {
                let format = RegisterClipboardFormatW(w!("HTML Format"));
                if format != 0 {
                    let mut doc = crate::logic::cf_html(html).into_bytes();
                    doc.push(0);
                    let _ = put(format, &doc); // the plain text is already there if this fails
                }
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

pub fn open(url: &str) {
    let url = wide(url);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

/// A saved setting (string) from HKCU\\Software\\PuenteDigital\\Nab.
pub fn setting(name: &str) -> Option<String> {
    let name = wide(name);
    let mut buf = [0u16; 256];
    let mut bytes = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, APP_KEY, PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut bytes))
            .ok()
            .ok()?;
    }
    let n = (bytes as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string())
}

pub fn save_setting(name: &str, value: &str) {
    let (name, data) = (wide(name), wide(value));
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, APP_KEY, PCWSTR(name.as_ptr()), REG_SZ.0, Some(data.as_ptr() as _), (data.len() * 2) as u32);
    }
}

fn number(name: &str) -> u64 {
    setting(name).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// What Nab has saved so far (see `logic::saving` for what it's compared with).
#[derive(Debug, PartialEq, Default, Clone, Copy)]
pub struct Saved {
    pub seconds: u64,
    pub nabs: u64,
    pub steps: u64,
}

/// Add one successful nab (a table or plain text) to the counter.
pub fn record_nab(table: bool) {
    let add = |name: &str, n: u64| save_setting(name, &number(name).saturating_add(n).to_string());
    let s = crate::logic::saving(table);
    add("SavedSeconds", s.seconds);
    add("Nabs", 1);
    add("Steps", s.steps);
}

pub fn saved() -> Saved {
    Saved { seconds: number("SavedSeconds"), nabs: number("Nabs"), steps: number("Steps") }
}

fn run_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --background", exe.display()))
}

// ---- Packaged (MSIX / Microsoft Store) installs ------------------------------------------------
// Inside a package Windows redirects HKCU writes to a private hive, so the Run key would never
// start us. Packaged builds use the startup task declared in msix/AppxManifest.xml instead.

/// Task id from msix/AppxManifest.xml.
const STARTUP_TASK: &str = "NabStartup";

/// Running from an MSIX package (e.g. installed from the Microsoft Store)?
pub fn packaged() -> bool {
    static PACKAGED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PACKAGED.get_or_init(|| unsafe {
        let mut len = 0u32;
        GetCurrentPackageFullName(&mut len, PWSTR::null()) != APPMODEL_ERROR_NO_PACKAGE
    })
}

/// Run a short WinRT call on its own MTA thread (the UI thread has no apartment) and wait for it.
fn winrt<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    let handle = std::thread::Builder::new()
        .name("nab-winrt".into())
        .spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            f()
        })
        .ok()?;
    handle.join().ok()
}

fn startup_task_state() -> Option<StartupTaskState> {
    winrt(|| StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).ok()?.get().ok()?.State().ok()).flatten()
}

/// Windows launched us because the user starts Nab at sign-in (packaged builds get no `--background`).
pub fn launched_at_startup() -> bool {
    packaged() && winrt(|| AppInstance::GetActivatedEventArgs().and_then(|a| a.Kind()).ok() == Some(ActivationKind::StartupTask)).unwrap_or(false)
}

pub fn autostart_enabled() -> bool {
    if packaged() {
        return matches!(startup_task_state(), Some(StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy));
    }
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

/// Start-with-Windows has been switched off in Windows Settings (or by policy): the app can't turn it on.
pub fn autostart_locked() -> bool {
    packaged() && matches!(startup_task_state(), Some(StartupTaskState::DisabledByUser | StartupTaskState::DisabledByPolicy))
}

pub fn set_autostart(on: bool) {
    if packaged() {
        winrt(move || {
            let Ok(task) = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).and_then(|a| a.get()) else { return };
            if on {
                let _ = task.RequestEnableAsync().and_then(|a| a.get());
            } else {
                let _ = task.Disable();
            }
        });
        return;
    }
    unsafe {
        if on {
            let Some(cmd) = run_command() else { return };
            let data = wide(&cmd);
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(data.as_ptr() as _), (data.len() * 2) as u32);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
}

/// If start-with-Windows is on but the .exe has moved, point it at the new location.
/// (Packaged installs are started by Windows itself, so there's nothing to refresh.)
pub fn refresh_autostart() {
    if !packaged() && autostart_enabled() {
        set_autostart(true);
    }
}

/// Wipe the test settings key (never the real one: `APP_KEY` is the `-tests` key under `cfg(test)`).
#[cfg(test)]
pub fn wipe_test_settings() {
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, APP_KEY);
    }
}

/// Deletes the test settings key when dropped, so a failing test leaves nothing behind.
#[cfg(test)]
pub struct TestSettings;
#[cfg(test)]
impl Drop for TestSettings {
    fn drop(&mut self) {
        wipe_test_settings();
    }
}

#[cfg(test)]
mod tests {
    use windows::core::w;
    use windows::Win32::System::Registry::*;

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(super::wide("ok"), vec![b'o' as u16, b'k' as u16, 0]);
    }

    #[test]
    fn clipboard_roundtrip() {
        let _serial = crate::serial();
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::*;
        use windows::Win32::UI::WindowsAndMessaging::*;
        // The clipboard needs an owner window (the app uses its hidden host window).
        let owner = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, None, None, None).unwrap()
        };
        crate::HOST.store(owner.0 as isize, std::sync::atomic::Ordering::SeqCst);
        let text = "Nab ✓ test\nline two";
        super::set_clipboard(text, Some("<table><tr><td>£1</td></tr></table>")).unwrap();
        unsafe {
            OpenClipboard(None).unwrap();
            let h = GetClipboardData(super::CF_UNICODETEXT).unwrap();
            let p = GlobalLock(windows::Win32::Foundation::HGLOBAL(h.0)) as *const u16;
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let got = String::from_utf16(std::slice::from_raw_parts(p, n)).unwrap();
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(h.0));
            // The HTML version is there too, with a valid envelope around our fragment.
            let format = RegisterClipboardFormatW(w!("HTML Format"));
            let hh = GetClipboardData(format).unwrap();
            let size = GlobalSize(windows::Win32::Foundation::HGLOBAL(hh.0));
            let p = GlobalLock(windows::Win32::Foundation::HGLOBAL(hh.0)) as *const u8;
            let html = String::from_utf8_lossy(std::slice::from_raw_parts(p, size)).to_string();
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(hh.0));
            let _ = CloseClipboard();
            assert_eq!(got, "Nab ✓ test\r\nline two");
            assert!(html.starts_with("Version:0.9"), "{html}");
            assert!(html.contains("<!--StartFragment--><table><tr><td>£1</td></tr></table><!--EndFragment-->"), "{html}");
        }
    }

    #[test]
    fn not_packaged_when_run_as_a_test() {
        // The test runner is a plain .exe, so the Run-key path is the one under test here, and
        // the WinRT calls behind the packaged path must fail cleanly rather than crash.
        assert!(!super::packaged());
        assert!(!super::launched_at_startup());
        assert!(!super::autostart_locked());
    }

    #[test]
    fn settings_roundtrip() {
        let _serial = crate::serial();
        let _wipe = super::TestSettings;
        super::save_setting("Probe", "Ctrl+Shift+Left click");
        assert_eq!(super::setting("Probe").as_deref(), Some("Ctrl+Shift+Left click"));
        super::save_setting("Probe", "Off");
        assert_eq!(super::setting("Probe").as_deref(), Some("Off"));
        super::save_setting("Probe", "ñ ✓ 日本");
        assert_eq!(super::setting("Probe").as_deref(), Some("ñ ✓ 日本"));
        assert_eq!(super::setting("DefinitelyNotThere"), None);
    }

    #[test]
    fn time_saved_counter_adds_up_and_survives_junk() {
        let _serial = crate::serial();
        let _wipe = super::TestSettings;
        use super::Saved;
        for name in ["SavedSeconds", "Nabs", "Steps"] {
            super::save_setting(name, "0");
        }
        assert_eq!(super::saved(), Saved::default());
        super::record_nab(false); // text: 20 s, 5 steps
        super::record_nab(true); // table: 60 s, 7 steps
        assert_eq!(super::saved(), Saved { seconds: 80, nabs: 2, steps: 12 });
        // Damaged values start again from zero rather than breaking anything.
        super::save_setting("SavedSeconds", "lots");
        super::save_setting("Nabs", "-3");
        super::save_setting("Steps", "");
        assert_eq!(super::saved(), Saved::default());
        super::record_nab(false);
        assert_eq!(super::saved(), Saved { seconds: 20, nabs: 1, steps: 5 });
    }

    #[test]
    fn autostart_toggle_roundtrip() {
        let _serial = crate::serial();
        let real = || unsafe {
            let mut buf = [0u16; 1024];
            let mut bytes = (buf.len() * 2) as u32;
            RegGetValueW(HKEY_CURRENT_USER, super::RUN_KEY, w!("Nab"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut bytes))
                .ok()
                .map(|_| String::from_utf16_lossy(&buf[..(bytes as usize / 2).min(buf.len())]))
        };
        let before = real();
        // Never leave the test entry behind (it would launch the test runner at login), even if an assert fails.
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                super::set_autostart(false);
            }
        }
        let _cleanup = Cleanup;
        super::set_autostart(true);
        assert!(super::autostart_enabled());
        super::set_autostart(false);
        assert!(!super::autostart_enabled());
        // The user's own start-with-Windows entry is untouched.
        assert_eq!(real(), before);
        assert_ne!(unsafe { super::RUN_VALUE.to_string() }.ok().as_deref(), Some("Nab"));
    }
}
