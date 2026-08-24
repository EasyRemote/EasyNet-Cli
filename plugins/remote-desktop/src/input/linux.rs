// EasyNet RemoteApp — Linux X11 input backend
// ============================================
//
// X11 + XTest is loaded dynamically so the product binary can run on systems
// without X11 development packages. Pure Wayland remains fail-closed until the
// portal RemoteDesktop lifecycle is bound to the RemoteApp session.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::sync::{Mutex, OnceLock};

use libloading::Library;

use super::{
    map_pointer_point, InputApplyOutcome, KeyInputFrame, PointerInputFrame, PointerTargetGeometry,
};

const CURRENT_TIME: c_ulong = 0;
const MAX_WHEEL_STEPS_PER_AXIS: usize = 12;
const LINUX_XTEST_DENIED: &str = "linux_xtest_injection_denied";

type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type XCloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
type XFlush = unsafe extern "C" fn(*mut c_void) -> c_int;
type XInitThreads = unsafe extern "C" fn() -> c_int;
type XKeysymToKeycode = unsafe extern "C" fn(*mut c_void, c_ulong) -> u8;
type XTestQueryExtension =
    unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> c_int;
type XTestFakeMotionEvent =
    unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_ulong) -> c_int;
type XTestFakeButtonEvent = unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int;
type XTestFakeKeyEvent = unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int;

static BACKEND: OnceLock<Mutex<X11InputBackend>> = OnceLock::new();

pub(super) fn input_injection_available() -> bool {
    backend().is_ok()
}

pub(super) fn input_injection_backend() -> &'static str {
    "linux_x11_xtest"
}

pub(super) fn input_injection_unavailable_reason() -> Option<&'static str> {
    backend().err()
}

pub(super) fn request_input_injection_permission() -> bool {
    input_injection_available()
}

pub(super) fn apply_pointer_frame(
    frame: &PointerInputFrame,
    target: Option<PointerTargetGeometry>,
) -> InputApplyOutcome {
    let backend = match backend() {
        Ok(backend) => backend,
        Err(reason) => return InputApplyOutcome::rejected(reason),
    };
    let Ok(backend) = backend.lock() else {
        return InputApplyOutcome::rejected("linux_xtest_backend_poisoned");
    };
    let point = map_pointer_point(frame, target);
    if !backend.motion(point.x.round() as c_int, point.y.round() as c_int) {
        return InputApplyOutcome::rejected(LINUX_XTEST_DENIED);
    }
    let applied = match frame.action.as_str() {
        "move" => true,
        "down" | "up" => button_number(frame.button.unwrap_or(0))
            .is_some_and(|button| backend.button(button, frame.action == "down")),
        "wheel" => apply_wheel(&backend, frame),
        _ => return InputApplyOutcome::rejected("unsupported_pointer_action"),
    };
    if applied && backend.flush() {
        InputApplyOutcome::applied()
    } else {
        InputApplyOutcome::rejected(LINUX_XTEST_DENIED)
    }
}

pub(super) fn release_pointer_button(button: u8) -> InputApplyOutcome {
    let Some(button) = button_number(button) else {
        return InputApplyOutcome::rejected("unsupported_pointer_button");
    };
    let backend = match backend() {
        Ok(backend) => backend,
        Err(reason) => return InputApplyOutcome::rejected(reason),
    };
    let Ok(backend) = backend.lock() else {
        return InputApplyOutcome::rejected("linux_xtest_backend_poisoned");
    };
    if backend.button(button, false) && backend.flush() {
        InputApplyOutcome::applied()
    } else {
        InputApplyOutcome::rejected(LINUX_XTEST_DENIED)
    }
}

pub(super) fn apply_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
    let key_down = match frame.action.as_str() {
        "down" => true,
        "up" if !frame.repeat => false,
        "up" => return InputApplyOutcome::rejected("invalid_key_repeat"),
        _ => return InputApplyOutcome::rejected("unsupported_key_action"),
    };
    let Some(keysym) = x11_keysym(frame) else {
        return InputApplyOutcome::rejected("unsupported_key");
    };
    let backend = match backend() {
        Ok(backend) => backend,
        Err(reason) => return InputApplyOutcome::rejected(reason),
    };
    let Ok(backend) = backend.lock() else {
        return InputApplyOutcome::rejected("linux_xtest_backend_poisoned");
    };
    let keycode = unsafe { (backend.x_keysym_to_keycode)(backend.display, keysym) };
    if keycode == 0 || !backend.key(u32::from(keycode), key_down) || !backend.flush() {
        InputApplyOutcome::rejected(LINUX_XTEST_DENIED)
    } else {
        InputApplyOutcome::applied()
    }
}

pub(super) fn release_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
    apply_key_frame(frame)
}

fn backend() -> Result<&'static Mutex<X11InputBackend>, &'static str> {
    if let Some(backend) = BACKEND.get() {
        return Ok(backend);
    }
    let candidate = X11InputBackend::connect()?;
    let _ = BACKEND.set(Mutex::new(candidate));
    BACKEND.get().ok_or("linux_xtest_backend_unavailable")
}

struct X11InputBackend {
    display: *mut c_void,
    x_close_display: XCloseDisplay,
    x_flush: XFlush,
    x_keysym_to_keycode: XKeysymToKeycode,
    x_test_fake_motion_event: XTestFakeMotionEvent,
    x_test_fake_button_event: XTestFakeButtonEvent,
    x_test_fake_key_event: XTestFakeKeyEvent,
    _x11: Library,
    _xtst: Library,
}

// The Display is used only while the enclosing Mutex is held.
unsafe impl Send for X11InputBackend {}

impl X11InputBackend {
    fn connect() -> Result<Self, &'static str> {
        // DISPLAY may point at XWayland, but the selected Resource currently
        // carries no proof that it belongs to an XWayland client. Injecting
        // through XTest could therefore miss a native Wayland target while
        // reporting success. Fail closed until portal session identity is
        // committed into the RemoteApp binding.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return Err("linux_wayland_portal_remote_desktop_not_implemented");
        }
        if std::env::var_os("DISPLAY").is_none() {
            return Err("linux_x11_display_unavailable");
        }
        let x11 =
            load_library(&["libX11.so.6", "libX11.so"]).ok_or("linux_x11_library_unavailable")?;
        let xtst = load_library(&["libXtst.so.6", "libXtst.so"])
            .ok_or("linux_xtst_library_unavailable")?;
        let x_init_threads: XInitThreads = unsafe { load_symbol(&x11, b"XInitThreads\0")? };
        let x_open_display: XOpenDisplay = unsafe { load_symbol(&x11, b"XOpenDisplay\0")? };
        let x_close_display: XCloseDisplay = unsafe { load_symbol(&x11, b"XCloseDisplay\0")? };
        let x_flush: XFlush = unsafe { load_symbol(&x11, b"XFlush\0")? };
        let x_keysym_to_keycode: XKeysymToKeycode =
            unsafe { load_symbol(&x11, b"XKeysymToKeycode\0")? };
        let x_test_query_extension: XTestQueryExtension =
            unsafe { load_symbol(&xtst, b"XTestQueryExtension\0")? };
        let x_test_fake_motion_event: XTestFakeMotionEvent =
            unsafe { load_symbol(&xtst, b"XTestFakeMotionEvent\0")? };
        let x_test_fake_button_event: XTestFakeButtonEvent =
            unsafe { load_symbol(&xtst, b"XTestFakeButtonEvent\0")? };
        let x_test_fake_key_event: XTestFakeKeyEvent =
            unsafe { load_symbol(&xtst, b"XTestFakeKeyEvent\0")? };

        unsafe {
            let _ = x_init_threads();
        }
        let display = unsafe { x_open_display(std::ptr::null()) };
        if display.is_null() {
            return Err("linux_x11_display_unavailable");
        }
        let mut event_base = 0;
        let mut error_base = 0;
        let mut major = 0;
        let mut minor = 0;
        let extension_ready = unsafe {
            x_test_query_extension(
                display,
                &mut event_base,
                &mut error_base,
                &mut major,
                &mut minor,
            ) != 0
        };
        if !extension_ready {
            unsafe {
                x_close_display(display);
            }
            return Err("linux_xtest_extension_unavailable");
        }
        Ok(Self {
            display,
            x_close_display,
            x_flush,
            x_keysym_to_keycode,
            x_test_fake_motion_event,
            x_test_fake_button_event,
            x_test_fake_key_event,
            _x11: x11,
            _xtst: xtst,
        })
    }

    fn motion(&self, x: c_int, y: c_int) -> bool {
        unsafe { (self.x_test_fake_motion_event)(self.display, -1, x, y, CURRENT_TIME) != 0 }
    }

    fn button(&self, button: c_uint, pressed: bool) -> bool {
        unsafe {
            (self.x_test_fake_button_event)(
                self.display,
                button,
                c_int::from(pressed),
                CURRENT_TIME,
            ) != 0
        }
    }

    fn key(&self, keycode: c_uint, pressed: bool) -> bool {
        unsafe {
            (self.x_test_fake_key_event)(self.display, keycode, c_int::from(pressed), CURRENT_TIME)
                != 0
        }
    }

    fn flush(&self) -> bool {
        unsafe { (self.x_flush)(self.display) >= 0 }
    }
}

impl Drop for X11InputBackend {
    fn drop(&mut self) {
        if !self.display.is_null() {
            unsafe {
                (self.x_close_display)(self.display);
            }
            self.display = std::ptr::null_mut();
        }
    }
}

fn load_library(candidates: &[&str]) -> Option<Library> {
    candidates
        .iter()
        .find_map(|candidate| unsafe { Library::new(candidate).ok() })
}

unsafe fn load_symbol<T: Copy>(library: &Library, symbol: &[u8]) -> Result<T, &'static str> {
    unsafe { library.get::<T>(symbol) }
        .map(|symbol| *symbol)
        .map_err(|_| "linux_x11_symbol_unavailable")
}

fn button_number(button: u8) -> Option<c_uint> {
    match button {
        0 => Some(1),
        1 => Some(2),
        2 => Some(3),
        _ => None,
    }
}

fn apply_wheel(backend: &X11InputBackend, frame: &PointerInputFrame) -> bool {
    let vertical = wheel_steps(frame.delta_y);
    let horizontal = wheel_steps(frame.delta_x);
    if vertical == 0 && horizontal == 0 {
        return false;
    }
    wheel_buttons(vertical, 4, 5)
        .chain(wheel_buttons(horizontal, 6, 7))
        .all(|button| backend.button(button, true) && backend.button(button, false))
}

fn wheel_steps(delta: Option<f64>) -> i32 {
    let Some(delta) = delta.filter(|value| *value != 0.0) else {
        return 0;
    };
    let steps = (delta.abs() / 100.0).ceil() as usize;
    let bounded = steps.clamp(1, MAX_WHEEL_STEPS_PER_AXIS) as i32;
    if delta.is_sign_negative() {
        -bounded
    } else {
        bounded
    }
}

fn wheel_buttons(
    steps: i32,
    negative_button: c_uint,
    positive_button: c_uint,
) -> impl Iterator<Item = c_uint> {
    let button = if steps.is_negative() {
        negative_button
    } else {
        positive_button
    };
    std::iter::repeat_n(button, steps.unsigned_abs() as usize)
}

fn x11_keysym(frame: &KeyInputFrame) -> Option<c_ulong> {
    dom_code_keysym(&frame.code).or_else(|| unicode_keysym(&frame.key))
}

fn unicode_keysym(key: &str) -> Option<c_ulong> {
    let mut chars = key.chars();
    let value = chars.next()?;
    if chars.next().is_some() {
        return dom_code_keysym(key);
    }
    let codepoint = u32::from(value);
    Some(if codepoint <= 0xff {
        c_ulong::from(codepoint)
    } else {
        c_ulong::from(0x0100_0000 | codepoint)
    })
}

fn dom_code_keysym(code: &str) -> Option<c_ulong> {
    let keysym = match code {
        code if code.len() == 4 && code.starts_with("Key") => {
            c_ulong::from(code.as_bytes()[3].to_ascii_lowercase())
        }
        code if code.len() == 6 && code.starts_with("Digit") => c_ulong::from(code.as_bytes()[5]),
        "Enter" => 0xff0d,
        "Tab" => 0xff09,
        "Space" => 0x20,
        "Backspace" => 0xff08,
        "Escape" => 0xff1b,
        "ShiftLeft" => 0xffe1,
        "ShiftRight" => 0xffe2,
        "ControlLeft" => 0xffe3,
        "ControlRight" => 0xffe4,
        "AltLeft" => 0xffe9,
        "AltRight" => 0xffea,
        "MetaLeft" => 0xffeb,
        "MetaRight" => 0xffec,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Insert" => 0xff63,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "Minus" => 0x2d,
        "Equal" => 0x3d,
        "BracketLeft" => 0x5b,
        "BracketRight" => 0x5d,
        "Backslash" => 0x5c,
        "Semicolon" => 0x3b,
        "Quote" => 0x27,
        "Backquote" => 0x60,
        "Comma" => 0x2c,
        "Period" => 0x2e,
        "Slash" => 0x2f,
        _ => return None,
    };
    Some(keysym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_wheel_expansion_is_bounded() {
        assert_eq!(wheel_steps(Some(10_000.0)), 12);
        assert_eq!(wheel_steps(Some(-10_000.0)), -12);
        assert_eq!(wheel_steps(Some(0.0)), 0);
        assert_eq!(wheel_steps(None), 0);
        assert_eq!(wheel_buttons(3, 4, 5).collect::<Vec<_>>(), vec![5, 5, 5]);
        assert_eq!(wheel_buttons(-2, 4, 5).collect::<Vec<_>>(), vec![4, 4]);
    }

    #[test]
    fn linux_dom_key_mapping_is_deterministic() {
        assert_eq!(dom_code_keysym("KeyA"), Some(c_ulong::from(b'a')));
        assert_eq!(dom_code_keysym("ArrowLeft"), Some(0xff51));
        assert_eq!(unicode_keysym("é"), Some(0xe9));
        assert_eq!(unicode_keysym("你"), Some(0x0100_4f60));
        assert_eq!(unicode_keysym("ab"), None);
    }
}
