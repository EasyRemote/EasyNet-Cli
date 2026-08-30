// EasyNet RemoteApp — canonical browser physical-key contract
// ============================================================
//
// `KeyboardEvent.code` names a physical key independently from the browser's
// active text layout. Parse that vocabulary once; OS adapters only translate
// the resulting key identity into their native representation.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhysicalKey {
    Letter(u8),
    Digit(u8),
    Function(u8),
    NumpadDigit(u8),
    Enter,
    NumpadEnter,
    Tab,
    Space,
    Backspace,
    Escape,
    CapsLock,
    NumLock,
    ScrollLock,
    PrintScreen,
    Pause,
    ContextMenu,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    MetaLeft,
    MetaRight,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,
    NumpadDecimal,
    NumpadMultiply,
    NumpadAdd,
    NumpadSubtract,
    NumpadDivide,
    NumpadEqual,
}

impl PhysicalKey {
    pub(super) fn from_dom_code(code: &str) -> Option<Self> {
        if let Some(letter) = single_ascii_suffix(code, "Key", b'A', b'Z') {
            return Some(Self::Letter(letter));
        }
        if let Some(digit) = single_ascii_suffix(code, "Digit", b'0', b'9') {
            return Some(Self::Digit(digit));
        }
        if let Some(digit) = single_ascii_suffix(code, "Numpad", b'0', b'9') {
            return Some(Self::NumpadDigit(digit));
        }
        if let Some(function) = code
            .strip_prefix('F')
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| (1..=12).contains(value))
        {
            return Some(Self::Function(function));
        }
        Some(match code {
            "Enter" => Self::Enter,
            "NumpadEnter" => Self::NumpadEnter,
            "Tab" => Self::Tab,
            "Space" => Self::Space,
            "Backspace" => Self::Backspace,
            "Escape" => Self::Escape,
            "CapsLock" => Self::CapsLock,
            "NumLock" => Self::NumLock,
            "ScrollLock" => Self::ScrollLock,
            "PrintScreen" => Self::PrintScreen,
            "Pause" => Self::Pause,
            "ContextMenu" => Self::ContextMenu,
            "ShiftLeft" => Self::ShiftLeft,
            "ShiftRight" => Self::ShiftRight,
            "ControlLeft" => Self::ControlLeft,
            "ControlRight" => Self::ControlRight,
            "AltLeft" => Self::AltLeft,
            "AltRight" => Self::AltRight,
            "MetaLeft" => Self::MetaLeft,
            "MetaRight" => Self::MetaRight,
            "ArrowLeft" => Self::ArrowLeft,
            "ArrowRight" => Self::ArrowRight,
            "ArrowUp" => Self::ArrowUp,
            "ArrowDown" => Self::ArrowDown,
            "Insert" => Self::Insert,
            "Delete" => Self::Delete,
            "Home" => Self::Home,
            "End" => Self::End,
            "PageUp" => Self::PageUp,
            "PageDown" => Self::PageDown,
            "Minus" => Self::Minus,
            "Equal" => Self::Equal,
            "BracketLeft" => Self::BracketLeft,
            "BracketRight" => Self::BracketRight,
            "Backslash" => Self::Backslash,
            "Semicolon" => Self::Semicolon,
            "Quote" => Self::Quote,
            "Backquote" => Self::Backquote,
            "Comma" => Self::Comma,
            "Period" => Self::Period,
            "Slash" => Self::Slash,
            "NumpadDecimal" => Self::NumpadDecimal,
            "NumpadMultiply" => Self::NumpadMultiply,
            "NumpadAdd" => Self::NumpadAdd,
            "NumpadSubtract" => Self::NumpadSubtract,
            "NumpadDivide" => Self::NumpadDivide,
            "NumpadEqual" => Self::NumpadEqual,
            _ => return None,
        })
    }
}

fn single_ascii_suffix(code: &str, prefix: &str, minimum: u8, maximum: u8) -> Option<u8> {
    let suffix = code.strip_prefix(prefix)?.as_bytes();
    (suffix.len() == 1 && (minimum..=maximum).contains(&suffix[0])).then_some(suffix[0])
}

#[cfg(test)]
mod tests {
    use super::PhysicalKey;

    #[test]
    fn parses_the_supported_browser_physical_key_vocabulary() {
        assert_eq!(
            PhysicalKey::from_dom_code("KeyA"),
            Some(PhysicalKey::Letter(b'A'))
        );
        assert_eq!(
            PhysicalKey::from_dom_code("Digit7"),
            Some(PhysicalKey::Digit(b'7'))
        );
        assert_eq!(
            PhysicalKey::from_dom_code("F12"),
            Some(PhysicalKey::Function(12))
        );
        assert_eq!(
            PhysicalKey::from_dom_code("Numpad9"),
            Some(PhysicalKey::NumpadDigit(b'9'))
        );
        assert_eq!(
            PhysicalKey::from_dom_code("ControlRight"),
            Some(PhysicalKey::ControlRight)
        );
        assert_eq!(PhysicalKey::from_dom_code("F13"), None);
        assert_eq!(PhysicalKey::from_dom_code("KeyAA"), None);
        assert_eq!(PhysicalKey::from_dom_code("Unidentified"), None);
    }
}
