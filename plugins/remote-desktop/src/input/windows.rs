// EasyNet RemoteApp — Windows input backend
// ==========================================
//
// Device-local User32 SendInput implementation. Admission, consent, target
// validation, sequence ordering, and audit events remain owned by input.rs.

use std::mem::size_of;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_ADD, VK_APPS, VK_BACK, VK_CAPITAL,
    VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT,
    VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MULTIPLY, VK_NEXT, VK_NUMLOCK,
    VK_NUMPAD0, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA,
    VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PAUSE, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SNAPSHOT, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::keyboard::PhysicalKey;
use super::wheel::windows_native_units;
use super::{
    map_pointer_point, InputApplyOutcome, KeyInputFrame, PointerInputFrame, PointerTargetGeometry,
    TargetInputGuardProof,
};

const WINDOWS_SEND_INPUT_DENIED: &str = "windows_send_input_denied";

pub(super) fn input_injection_available() -> bool {
    virtual_desktop().is_some()
}

pub(super) fn input_injection_backend() -> &'static str {
    "windows_user32_sendinput"
}

pub(super) fn input_injection_unavailable_reason() -> Option<&'static str> {
    (!input_injection_available()).then_some("windows_virtual_desktop_unavailable")
}

pub(super) fn request_input_injection_permission() -> bool {
    input_injection_available()
}

pub(super) fn apply_pointer_frame(
    frame: &PointerInputFrame,
    target: Option<PointerTargetGeometry>,
    _target_guard: Option<&TargetInputGuardProof>,
) -> InputApplyOutcome {
    let Some(desktop) = virtual_desktop() else {
        return InputApplyOutcome::rejected("windows_virtual_desktop_unavailable");
    };
    let point = map_pointer_point(frame, target);
    let (absolute_x, absolute_y) = desktop.absolute_point(point.x, point.y);
    let movement = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;

    let mut inputs = Vec::with_capacity(2);
    match frame.action.as_str() {
        "move" => inputs.push(mouse_input(absolute_x, absolute_y, 0, movement)),
        "down" | "up" => {
            let Some(button_flag) = button_flag(frame.action.as_str(), frame.button.unwrap_or(0))
            else {
                return InputApplyOutcome::rejected("unsupported_pointer_button");
            };
            inputs.push(mouse_input(
                absolute_x,
                absolute_y,
                0,
                movement | button_flag,
            ));
        }
        "wheel" => {
            let vertical = windows_native_units(frame.delta_y.map(|value| -value));
            let horizontal = windows_native_units(frame.delta_x);
            if vertical == 0 && horizontal == 0 {
                return InputApplyOutcome::rejected("empty_wheel_delta");
            }
            if vertical != 0 {
                inputs.push(mouse_input(
                    absolute_x,
                    absolute_y,
                    vertical as u32,
                    movement | MOUSEEVENTF_WHEEL,
                ));
            }
            if horizontal != 0 {
                inputs.push(mouse_input(
                    absolute_x,
                    absolute_y,
                    horizontal as u32,
                    movement | MOUSEEVENTF_HWHEEL,
                ));
            }
        }
        _ => return InputApplyOutcome::rejected("unsupported_pointer_action"),
    }
    send_inputs(&inputs)
}

pub(super) fn release_pointer_button(button: u8) -> InputApplyOutcome {
    let Some(flag) = button_flag("up", button) else {
        return InputApplyOutcome::rejected("unsupported_pointer_button");
    };
    send_inputs(&[mouse_input(0, 0, 0, flag)])
}

pub(super) fn apply_key_frame(
    frame: &KeyInputFrame,
    _target_guard: Option<&TargetInputGuardProof>,
) -> InputApplyOutcome {
    let Some(key) = windows_key(frame) else {
        return InputApplyOutcome::rejected("unsupported_key");
    };
    let key_up = match frame.action.as_str() {
        "down" => false,
        "up" if !frame.repeat => true,
        "up" => return InputApplyOutcome::rejected("invalid_key_repeat"),
        _ => return InputApplyOutcome::rejected("unsupported_key_action"),
    };
    let mut flags = if key_up { KEYEVENTF_KEYUP } else { 0 };
    if key.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key.virtual_key,
                wScan: unsafe {
                    MapVirtualKeyW(u32::from(key.virtual_key), MAPVK_VK_TO_VSC) as u16
                },
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_inputs(&[input])
}

pub(super) fn release_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
    apply_key_frame(frame, None)
}

fn send_inputs(inputs: &[INPUT]) -> InputApplyOutcome {
    let applied = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if applied as usize == inputs.len() {
        InputApplyOutcome::applied()
    } else {
        InputApplyOutcome::rejected(WINDOWS_SEND_INPUT_DENIED)
    }
}

fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn button_flag(action: &str, button: u8) -> Option<u32> {
    match (action, button) {
        ("down", 0) => Some(MOUSEEVENTF_LEFTDOWN),
        ("up", 0) => Some(MOUSEEVENTF_LEFTUP),
        ("down", 1) => Some(MOUSEEVENTF_MIDDLEDOWN),
        ("up", 1) => Some(MOUSEEVENTF_MIDDLEUP),
        ("down", 2) => Some(MOUSEEVENTF_RIGHTDOWN),
        ("up", 2) => Some(MOUSEEVENTF_RIGHTUP),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct VirtualDesktop {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl VirtualDesktop {
    fn absolute_point(self, x: f64, y: f64) -> (i32, i32) {
        (
            absolute_axis(x, self.x, self.width),
            absolute_axis(y, self.y, self.height),
        )
    }
}

fn virtual_desktop() -> Option<VirtualDesktop> {
    let desktop = VirtualDesktop {
        x: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        y: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) },
        height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) },
    };
    (desktop.width > 1 && desktop.height > 1).then_some(desktop)
}

fn absolute_axis(value: f64, origin: i32, span: i32) -> i32 {
    (((value - f64::from(origin)) * 65_535.0 / f64::from(span - 1))
        .round()
        .clamp(0.0, 65_535.0)) as i32
}

#[derive(Clone, Copy)]
struct WindowsKey {
    virtual_key: u16,
    extended: bool,
}

fn windows_key(frame: &KeyInputFrame) -> Option<WindowsKey> {
    dom_code_virtual_key(&frame.code).or_else(|| key_virtual_key(&frame.key))
}

fn dom_code_virtual_key(code: &str) -> Option<WindowsKey> {
    let (virtual_key, extended) = match PhysicalKey::from_dom_code(code)? {
        PhysicalKey::Letter(letter) => (u16::from(letter), false),
        PhysicalKey::Digit(digit) => (u16::from(digit), false),
        PhysicalKey::Function(function) => (VK_F1 + u16::from(function - 1), false),
        PhysicalKey::NumpadDigit(digit) => (VK_NUMPAD0 + u16::from(digit - b'0'), false),
        PhysicalKey::Enter => (VK_RETURN, false),
        PhysicalKey::NumpadEnter => (VK_RETURN, true),
        PhysicalKey::Tab => (VK_TAB, false),
        PhysicalKey::Space => (VK_SPACE, false),
        PhysicalKey::Backspace => (VK_BACK, false),
        PhysicalKey::Escape => (VK_ESCAPE, false),
        PhysicalKey::CapsLock => (VK_CAPITAL, false),
        PhysicalKey::NumLock => (VK_NUMLOCK, true),
        PhysicalKey::ScrollLock => (VK_SCROLL, false),
        PhysicalKey::PrintScreen => (VK_SNAPSHOT, true),
        PhysicalKey::Pause => (VK_PAUSE, false),
        PhysicalKey::ContextMenu => (VK_APPS, true),
        PhysicalKey::ShiftLeft => (VK_LSHIFT, false),
        PhysicalKey::ShiftRight => (VK_RSHIFT, false),
        PhysicalKey::ControlLeft => (VK_LCONTROL, false),
        PhysicalKey::ControlRight => (VK_RCONTROL, true),
        PhysicalKey::AltLeft => (VK_LMENU, false),
        PhysicalKey::AltRight => (VK_RMENU, true),
        PhysicalKey::MetaLeft => (VK_LWIN, true),
        PhysicalKey::MetaRight => (VK_RWIN, true),
        PhysicalKey::ArrowLeft => (VK_LEFT, true),
        PhysicalKey::ArrowRight => (VK_RIGHT, true),
        PhysicalKey::ArrowUp => (VK_UP, true),
        PhysicalKey::ArrowDown => (VK_DOWN, true),
        PhysicalKey::Insert => (VK_INSERT, true),
        PhysicalKey::Delete => (VK_DELETE, true),
        PhysicalKey::Home => (VK_HOME, true),
        PhysicalKey::End => (VK_END, true),
        PhysicalKey::PageUp => (VK_PRIOR, true),
        PhysicalKey::PageDown => (VK_NEXT, true),
        PhysicalKey::Minus => (VK_OEM_MINUS, false),
        PhysicalKey::Equal => (VK_OEM_PLUS, false),
        PhysicalKey::BracketLeft => (VK_OEM_4, false),
        PhysicalKey::BracketRight => (VK_OEM_6, false),
        PhysicalKey::Backslash => (VK_OEM_5, false),
        PhysicalKey::Semicolon => (VK_OEM_1, false),
        PhysicalKey::Quote => (VK_OEM_7, false),
        PhysicalKey::Backquote => (VK_OEM_3, false),
        PhysicalKey::Comma => (VK_OEM_COMMA, false),
        PhysicalKey::Period => (VK_OEM_PERIOD, false),
        PhysicalKey::Slash => (VK_OEM_2, false),
        PhysicalKey::NumpadDecimal => (VK_DECIMAL, false),
        PhysicalKey::NumpadMultiply => (VK_MULTIPLY, false),
        PhysicalKey::NumpadAdd => (VK_ADD, false),
        PhysicalKey::NumpadSubtract => (VK_SUBTRACT, false),
        PhysicalKey::NumpadDivide => (VK_DIVIDE, true),
        PhysicalKey::NumpadEqual => return None,
    };
    Some(WindowsKey {
        virtual_key,
        extended,
    })
}

fn key_virtual_key(key: &str) -> Option<WindowsKey> {
    if key.chars().count() == 1 {
        let value = unsafe { VkKeyScanW(key.encode_utf16().next()?) };
        if value != -1 {
            return Some(WindowsKey {
                virtual_key: (value as u16) & 0xff,
                extended: false,
            });
        }
    }
    dom_code_virtual_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_desktop_absolute_mapping_clamps_multi_monitor_coordinates() {
        let desktop = VirtualDesktop {
            x: -1_920,
            y: 0,
            width: 3_840,
            height: 1_080,
        };

        assert_eq!(desktop.absolute_point(-1_920.0, 0.0), (0, 0));
        assert_eq!(desktop.absolute_point(1_919.0, 1_079.0), (65_535, 65_535));
        assert_eq!(desktop.absolute_point(-9_999.0, 9_999.0), (0, 65_535));
    }

    #[test]
    fn windows_dom_key_mapping_marks_extended_keys() {
        let right_control = dom_code_virtual_key("ControlRight").expect("right control");
        assert_eq!(right_control.virtual_key, VK_RCONTROL);
        assert!(right_control.extended);

        let left_control = dom_code_virtual_key("ControlLeft").expect("left control");
        assert_eq!(left_control.virtual_key, VK_LCONTROL);
        assert!(!left_control.extended);

        let right_shift = dom_code_virtual_key("ShiftRight").expect("right shift");
        assert_eq!(right_shift.virtual_key, VK_RSHIFT);
        assert!(!right_shift.extended);

        let numpad_enter = dom_code_virtual_key("NumpadEnter").expect("numpad enter");
        assert_eq!(numpad_enter.virtual_key, VK_RETURN);
        assert!(numpad_enter.extended);
    }
}
