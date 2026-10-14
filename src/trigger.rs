//! Shortcuts: modifier keys plus a key or a mouse button. Pure logic, testable on any OS.
//!
//! Stored and shown in the same human form, e.g. `Ctrl+Shift+Left click`, `Ctrl+Alt+T`, `Off`.

// Same bit values as Win32 MOD_ALT / MOD_CONTROL / MOD_SHIFT / MOD_WIN, so they pass straight to RegisterHotKey.
pub const ALT: u8 = 1;
pub const CTRL: u8 = 2;
pub const SHIFT: u8 = 4;
pub const WIN: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Key(u16), // Windows virtual-key code
    Mouse(Button),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Trigger {
    pub mods: u8,
    pub input: Input,
}

pub const DEFAULT_MOUSE: Trigger = Trigger { mods: CTRL | SHIFT, input: Input::Mouse(Button::Left) };
pub const DEFAULT_KEY: Trigger = Trigger { mods: ALT, input: Input::Key(0x20) }; // Alt+Space

const MODS: [(u8, &str); 4] = [(CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift"), (WIN, "Win")];
const BUTTONS: [(Button, &str); 5] = [
    (Button::Left, "Left click"),
    (Button::Right, "Right click"),
    (Button::Middle, "Middle click"),
    (Button::Back, "Back button"),
    (Button::Forward, "Forward button"),
];
const KEYS: [(u16, &str); 28] = [
    (0x08, "Backspace"),
    (0x09, "Tab"),
    (0x0D, "Enter"),
    (0x13, "Pause"),
    (0x1B, "Esc"),
    (0x20, "Space"),
    (0x21, "PageUp"),
    (0x22, "PageDown"),
    (0x23, "End"),
    (0x24, "Home"),
    (0x25, "Left"),
    (0x26, "Up"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x2C, "PrtSc"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
];

pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_F4: u16 = 0x73;

fn is_fkey(vk: u16) -> bool {
    (0x70..=0x87).contains(&vk)
}

/// Shift, Ctrl, Alt and the Windows keys on their own are not a shortcut.
pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        _ if is_fkey(vk) => format!("F{}", vk - 0x6F),
        _ => KEYS.iter().find(|k| k.0 == vk).map(|k| k.1.to_string()).unwrap_or_else(|| format!("#{vk}")),
    }
}

fn parse_key(s: &str) -> Option<u16> {
    let up = s.to_ascii_uppercase();
    if let [c] = up.as_bytes() {
        if c.is_ascii_alphanumeric() {
            return Some(*c as u16);
        }
    }
    if let Some(n) = up.strip_prefix("NUM ").and_then(|n| n.parse::<u16>().ok()).filter(|n| *n <= 9) {
        return Some(0x60 + n);
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(0x6F + n);
    }
    if let Some(n) = s.strip_prefix('#').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=0xFE).contains(n)) {
        return Some(n);
    }
    KEYS.iter().find(|k| k.1.eq_ignore_ascii_case(s)).map(|k| k.0)
}

/// `Ctrl+Shift+Left click`. Modifiers always in the order Ctrl, Alt, Shift, Win.
pub fn format(t: Option<Trigger>) -> String {
    let Some(t) = t else { return "Off".into() };
    let mut parts: Vec<String> = MODS.iter().filter(|m| t.mods & m.0 != 0).map(|m| m.1.to_string()).collect();
    parts.push(match t.input {
        Input::Key(vk) => key_name(vk),
        Input::Mouse(b) => BUTTONS.iter().find(|x| x.0 == b).map(|x| x.1).unwrap_or("?").to_string(),
    });
    parts.join("+")
}

/// Inverse of [`format`]. `None` = not understood; `Some(None)` = "Off".
pub fn parse(s: &str) -> Option<Option<Trigger>> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("off") {
        return Some(None);
    }
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (last, mods) = parts.split_last()?;
    let mut m = 0;
    for p in mods {
        m |= match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => CTRL,
            "alt" => ALT,
            "shift" => SHIFT,
            "win" | "windows" => WIN,
            _ => return None,
        };
    }
    let input = match BUTTONS.iter().find(|b| b.1.eq_ignore_ascii_case(last)) {
        Some(b) => Input::Mouse(b.0),
        None => Input::Key(parse_key(last)?),
    };
    Some(Some(Trigger { mods: m, input }))
}

/// Is this a sensible shortcut? Errors are shown to the user as-is.
pub fn validate(t: Trigger) -> Result<(), &'static str> {
    let strong = t.mods & (CTRL | ALT | WIN) != 0;
    match t.input {
        Input::Mouse(_) if !strong => Err("Hold Ctrl, Alt or Win too, so normal clicks keep working."),
        Input::Mouse(_) => Ok(()),
        Input::Key(vk) if is_modifier(vk) => Err("Add a normal key too."),
        Input::Key(VK_ESCAPE) => Err("Esc is used to cancel — pick another key."),
        Input::Key(VK_DELETE) if t.mods & (CTRL | ALT) == CTRL | ALT => Err("Windows reserves Ctrl+Alt+Delete."),
        Input::Key(VK_F4 | VK_TAB) if t.mods == ALT => Err("Alt+F4 and Alt+Tab belong to Windows — pick another."),
        Input::Key(VK_TAB) if t.mods == CTRL => Err("Ctrl+Tab switches tabs everywhere — pick another."),
        Input::Key(0x4C) if t.mods == WIN => Err("Win+L locks the PC — pick another."),
        Input::Key(0x41 | 0x43 | 0x56 | 0x58 | 0x5A) if t.mods == CTRL => Err("That's copy, paste, undo or select-all in every app — pick another."),
        Input::Key(vk) if is_fkey(vk) => Ok(()),
        Input::Key(_) if !strong => Err("Use Ctrl, Alt or Win with that key (or an F-key)."),
        Input::Key(_) => Ok(()),
    }
}

/// Pack into a u32 for lock-free sharing with the mouse hook thread. 0 = off.
pub fn encode(t: Option<Trigger>) -> u32 {
    let Some(t) = t else { return 0 };
    let (kind, code) = match t.input {
        Input::Key(vk) => (1u32, vk as u32),
        Input::Mouse(b) => (2, BUTTONS.iter().position(|x| x.0 == b).unwrap_or(0) as u32),
    };
    1 << 31 | (t.mods as u32) << 24 | kind << 16 | code
}

pub fn decode(v: u32) -> Option<Trigger> {
    if v & (1 << 31) == 0 {
        return None;
    }
    let mods = ((v >> 24) & 0x0F) as u8;
    let code = v & 0xFFFF;
    let input = match (v >> 16) & 0xFF {
        1 => Input::Key(code as u16),
        2 => Input::Mouse(BUTTONS.get(code as usize)?.0),
        _ => return None,
    };
    Some(Trigger { mods, input })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(mods: u8, input: Input) -> Trigger {
        Trigger { mods, input }
    }

    #[test]
    fn defaults_format_nicely() {
        assert_eq!(format(Some(DEFAULT_MOUSE)), "Ctrl+Shift+Left click");
        assert_eq!(format(Some(DEFAULT_KEY)), "Alt+Space");
        assert_eq!(format(None), "Off");
        assert!(validate(DEFAULT_MOUSE).is_ok());
        assert!(validate(DEFAULT_KEY).is_ok());
    }

    #[test]
    fn modifier_order_is_canonical() {
        assert_eq!(format(Some(t(WIN | SHIFT | ALT | CTRL, Input::Key(0x41)))), "Ctrl+Alt+Shift+Win+A");
    }

    #[test]
    fn parse_roundtrips_everything() {
        let mut all = vec![];
        for mods in 0..16u8 {
            for (b, _) in BUTTONS {
                all.push(t(mods, Input::Mouse(b)));
            }
            for vk in 1..=0xFEu16 {
                all.push(t(mods, Input::Key(vk)));
            }
        }
        for x in all {
            let s = format(Some(x));
            assert_eq!(parse(&s), Some(Some(x)), "{s}");
            assert_eq!(decode(encode(Some(x))), Some(x), "{s}");
        }
        assert_eq!(parse("Off"), Some(None));
        assert_eq!(parse(" off "), Some(None));
        assert_eq!(decode(encode(None)), None);
    }

    #[test]
    fn parse_is_forgiving_but_not_sloppy() {
        assert_eq!(parse("control+shift+left click"), Some(Some(DEFAULT_MOUSE)));
        assert_eq!(parse("alt + space"), Some(Some(DEFAULT_KEY)));
        assert_eq!(parse("Ctrl + Alt + t"), Some(Some(Trigger { mods: CTRL | ALT, input: Input::Key(b'T' as u16) })));
        assert_eq!(parse("Windows+F5"), Some(Some(t(WIN, Input::Key(0x74)))));
        assert_eq!(parse("Ctrl+Alt+num 3"), Some(Some(t(CTRL | ALT, Input::Key(0x63)))));
        assert_eq!(parse("Ctrl+Banana"), None);
        assert_eq!(parse("Hyper+T"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("Ctrl+F25"), None);
        assert_eq!(parse("Ctrl+#0"), None);
        assert_eq!(parse("Ctrl+Num 12"), None);
    }

    #[test]
    fn validation_rules() {
        let ok = |x| validate(x).is_ok();
        // mouse
        assert!(!ok(t(0, Input::Mouse(Button::Left))));
        assert!(!ok(t(SHIFT, Input::Mouse(Button::Left)))); // shift-click selects text everywhere
        assert!(ok(t(ALT, Input::Mouse(Button::Middle))));
        assert!(ok(t(CTRL, Input::Mouse(Button::Back))));
        // keys
        assert!(!ok(t(0, Input::Key(b'T' as u16))));
        assert!(!ok(t(SHIFT, Input::Key(b'T' as u16))));
        assert!(ok(t(0, Input::Key(0x7B)))); // F12 alone
        assert!(ok(t(SHIFT, Input::Key(0x70)))); // Shift+F1
        assert!(!ok(t(CTRL, Input::Key(0x11)))); // just Ctrl
        assert!(!ok(t(CTRL | ALT, Input::Key(VK_ESCAPE))));
        assert!(!ok(t(CTRL | ALT, Input::Key(VK_DELETE))));
        assert!(ok(t(CTRL, Input::Key(VK_DELETE))));
        assert!(ok(t(WIN | SHIFT, Input::Key(b'X' as u16))));
        // Combinations that would hijack something everyone relies on.
        assert!(!ok(t(ALT, Input::Key(VK_F4))));
        assert!(!ok(t(ALT, Input::Key(VK_TAB))));
        assert!(!ok(t(CTRL, Input::Key(VK_TAB))));
        assert!(!ok(t(CTRL, Input::Key(VK_ESCAPE))));
        assert!(!ok(t(WIN, Input::Key(b'L' as u16))));
        for k in *b"ACVXZ" {
            assert!(!ok(t(CTRL, Input::Key(k as u16))), "Ctrl+{}", k as char);
            assert!(ok(t(CTRL | ALT, Input::Key(k as u16))), "Ctrl+Alt+{}", k as char);
        }
        assert!(ok(t(CTRL | SHIFT, Input::Key(VK_F4)))); // only plain Alt+F4 is off limits
        assert!(ok(t(ALT, Input::Key(0x20)))); // the default
    }

    #[test]
    fn modifier_keys_detected() {
        for vk in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] {
            assert!(is_modifier(vk), "{vk:#x}");
        }
        for vk in [0x41, 0x70, 0x20, 0x5D] {
            assert!(!is_modifier(vk), "{vk:#x}");
        }
    }

    #[test]
    fn key_names() {
        assert_eq!(key_name(0x41), "A");
        assert_eq!(key_name(0x37), "7");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x87), "F24");
        assert_eq!(key_name(0x2C), "PrtSc");
        assert_eq!(key_name(0x65), "Num 5");
        assert_eq!(key_name(0xE2), "#226");
    }
}
