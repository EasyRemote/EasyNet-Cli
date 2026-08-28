// EasyNet RemoteApp — Linux X11 input backend
// ============================================
//
// X11 + XTest is loaded dynamically so the product binary can run on systems
// without X11 development packages. Pure Wayland remains fail-closed until the
// portal RemoteDesktop lifecycle is bound to the RemoteApp session.

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::sync::{Mutex, OnceLock};

use libloading::Library;

#[cfg(feature = "native-media")]
use xcb::{res, x, xtest, Extension, Xid, XidNew};

use super::keyboard::PhysicalKey;
use super::wheel::x11_detent_steps;
use super::{
    map_pointer_point, InputApplyOutcome, KeyInputFrame, PointerInputFrame, PointerTargetGeometry,
    TargetInputGuardProof,
};
#[cfg(feature = "native-media")]
use crate::daemon::plugins::remote_desktop::session::now_ms;

#[cfg(feature = "native-media")]
const CURRENT_TIME: u32 = 0;
const LINUX_XTEST_DENIED: &str = "linux_xtest_injection_denied";
#[cfg(feature = "native-media")]
const LINUX_XTEST_PARTIAL_EFFECT: &str = "linux_xtest_partial_effect";
#[cfg(feature = "native-media")]
const MAX_ATOMIC_TARGET_WINDOWS: usize = 32;

#[cfg(feature = "native-media")]
const KEY_PRESS: u8 = 2;
#[cfg(feature = "native-media")]
const KEY_RELEASE: u8 = 3;
#[cfg(feature = "native-media")]
const BUTTON_PRESS: u8 = 4;
#[cfg(feature = "native-media")]
const BUTTON_RELEASE: u8 = 5;
#[cfg(feature = "native-media")]
const MOTION_NOTIFY: u8 = 6;

type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type XCloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
type XInitThreads = unsafe extern "C" fn() -> c_int;
type XKeysymToKeycode = unsafe extern "C" fn(*mut c_void, c_ulong) -> u8;

static BACKEND: OnceLock<Mutex<Option<X11TargetInputExecutor>>> = OnceLock::new();

pub(super) fn input_injection_available() -> bool {
    with_backend(|_| Ok(())).is_ok()
}

pub(super) fn input_injection_backend() -> &'static str {
    "linux_x11_xcb_atomic_xtest"
}

pub(super) fn input_injection_unavailable_reason() -> Option<&'static str> {
    with_backend(|_| Ok(())).err()
}

pub(super) fn request_input_injection_permission() -> bool {
    input_injection_available()
}

pub(super) fn apply_pointer_frame(
    frame: &PointerInputFrame,
    target: Option<PointerTargetGeometry>,
    target_guard: Option<&TargetInputGuardProof>,
) -> InputApplyOutcome {
    // Validate the complete native operation before posting any XTest event.
    // A rejected frame must be side-effect free; in particular, an invalid
    // button or zero-distance wheel frame must not move the global cursor.
    let operation = match pointer_operation(frame) {
        Ok(operation) => operation,
        Err(reason) => return InputApplyOutcome::rejected(reason),
    };
    let point = map_pointer_point(frame, target);
    let coordinates = match atomic_coordinates(point.x, point.y) {
        Ok(coordinates) => coordinates,
        Err(reason) => return InputApplyOutcome::rejected(reason),
    };
    match with_backend(|backend| backend.execute_pointer(operation, coordinates, target_guard)) {
        Ok(validation) => InputApplyOutcome::applied().with_target_guard_validation(validation),
        Err(reason) => InputApplyOutcome::rejected(reason),
    }
}

pub(super) fn release_pointer_button(button: u8) -> InputApplyOutcome {
    let Some(button) = button_number(button) else {
        return InputApplyOutcome::rejected("unsupported_pointer_button");
    };
    match with_backend(|backend| backend.execute_release_button(button)) {
        Ok(()) => InputApplyOutcome::applied(),
        Err(reason) => InputApplyOutcome::rejected(reason),
    }
}

pub(super) fn apply_key_frame(
    frame: &KeyInputFrame,
    target_guard: Option<&TargetInputGuardProof>,
) -> InputApplyOutcome {
    let key_down = match frame.action.as_str() {
        "down" => true,
        "up" if !frame.repeat => false,
        "up" => return InputApplyOutcome::rejected("invalid_key_repeat"),
        _ => return InputApplyOutcome::rejected("unsupported_key_action"),
    };
    let Some(keysym) = x11_keysym(frame) else {
        return InputApplyOutcome::rejected("unsupported_key");
    };
    match with_backend(|backend| backend.execute_key(keysym, key_down, target_guard)) {
        Ok(validation) => InputApplyOutcome::applied().with_target_guard_validation(validation),
        Err(reason) => InputApplyOutcome::rejected(reason),
    }
}

pub(super) fn release_key_frame(frame: &KeyInputFrame) -> InputApplyOutcome {
    apply_key_frame(frame, None)
}

fn with_backend<T>(
    operation: impl FnOnce(&mut X11TargetInputExecutor) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let backend = BACKEND.get_or_init(|| Mutex::new(None));
    let mut backend = backend.lock().map_err(|_| "linux_xtest_backend_poisoned")?;
    if backend.is_none() {
        *backend = Some(X11TargetInputExecutor::connect()?);
    }
    let result = operation(
        backend
            .as_mut()
            .expect("Linux X11 input executor exists after connection"),
    );
    if result
        .as_ref()
        .is_err_and(|reason| backend_error_requires_reconnect(reason))
    {
        // Every protocol/connection failure retires the connection. The next
        // frame reconnects instead of pinning the process to a poisoned X11
        // Display for the remainder of the daemon lifetime.
        backend.take();
    }
    result
}

fn backend_error_requires_reconnect(reason: &&'static str) -> bool {
    matches!(
        *reason,
        LINUX_XTEST_DENIED
            | "linux_target_pointer_query_failed"
            | "linux_target_focus_query_failed"
            | "linux_target_window_tree_query_failed"
            | "linux_target_owner_query_failed"
            | "linux_x11_server_grab_failed"
            | "linux_x11_server_grab_barrier_failed"
            | "linux_x11_server_ungrab_failed"
            | "linux_x11_server_ungrab_barrier_failed"
    )
}

struct X11TargetInputExecutor {
    #[cfg(feature = "native-media")]
    connection: xcb::Connection,
    #[cfg(feature = "native-media")]
    root: x::Window,
    keymap_display: *mut c_void,
    x_close_display: XCloseDisplay,
    x_keysym_to_keycode: XKeysymToKeycode,
    _x11: Library,
}

// Both connections are used only while the enclosing Mutex is held.
unsafe impl Send for X11TargetInputExecutor {}

impl X11TargetInputExecutor {
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
        let x_init_threads: XInitThreads = unsafe { load_symbol(&x11, b"XInitThreads\0")? };
        let x_open_display: XOpenDisplay = unsafe { load_symbol(&x11, b"XOpenDisplay\0")? };
        let x_close_display: XCloseDisplay = unsafe { load_symbol(&x11, b"XCloseDisplay\0")? };
        let x_keysym_to_keycode: XKeysymToKeycode =
            unsafe { load_symbol(&x11, b"XKeysymToKeycode\0")? };

        unsafe {
            let _ = x_init_threads();
        }
        let keymap_display = unsafe { x_open_display(std::ptr::null()) };
        if keymap_display.is_null() {
            return Err("linux_x11_display_unavailable");
        }

        #[cfg(not(feature = "native-media"))]
        {
            unsafe { x_close_display(keymap_display) };
            return Err("linux_atomic_target_input_requires_native_media");
        }

        #[cfg(feature = "native-media")]
        let (connection, screen_number) =
            xcb::Connection::connect_with_extensions(None, &[Extension::Res, Extension::Test], &[])
                .map_err(|_| "linux_x11_display_unavailable")?;
        #[cfg(feature = "native-media")]
        let root = connection
            .get_setup()
            .roots()
            .nth(screen_number as usize)
            .map(|screen| screen.root())
            .ok_or("linux_x11_root_unavailable")?;
        #[cfg(feature = "native-media")]
        connection
            .wait_for_reply(connection.send_request(&xtest::GetVersion {
                major_version: 2,
                minor_version: 2,
            }))
            .map_err(|_| "linux_xtest_extension_unavailable")?;
        #[cfg(feature = "native-media")]
        connection
            .wait_for_reply(connection.send_request(&res::QueryVersion {
                client_major: 1,
                client_minor: 2,
            }))
            .map_err(|_| "linux_xres_extension_unavailable")?;

        Ok(Self {
            #[cfg(feature = "native-media")]
            connection,
            #[cfg(feature = "native-media")]
            root,
            keymap_display,
            x_close_display,
            x_keysym_to_keycode,
            _x11: x11,
        })
    }

    #[cfg(feature = "native-media")]
    fn execute_pointer(
        &self,
        operation: PointerOperation,
        coordinates: (i16, i16),
        target_guard: Option<&TargetInputGuardProof>,
    ) -> Result<Option<TargetInputGuardProof>, &'static str> {
        self.execute(target_guard, Some(coordinates), |executor| {
            executor.inject_pointer(operation, coordinates)
        })
    }

    #[cfg(not(feature = "native-media"))]
    fn execute_pointer(
        &self,
        _operation: PointerOperation,
        _coordinates: (i16, i16),
        _target_guard: Option<&TargetInputGuardProof>,
    ) -> Result<Option<TargetInputGuardProof>, &'static str> {
        Err("linux_atomic_target_input_requires_native_media")
    }

    #[cfg(feature = "native-media")]
    fn execute_key(
        &self,
        keysym: c_ulong,
        pressed: bool,
        target_guard: Option<&TargetInputGuardProof>,
    ) -> Result<Option<TargetInputGuardProof>, &'static str> {
        let keycode = unsafe { (self.x_keysym_to_keycode)(self.keymap_display, keysym) };
        if keycode == 0 {
            return Err("unsupported_key");
        }
        self.execute(target_guard, None, |executor| {
            executor.fake_input(if pressed { KEY_PRESS } else { KEY_RELEASE }, keycode, 0, 0)
        })
    }

    #[cfg(not(feature = "native-media"))]
    fn execute_key(
        &self,
        _keysym: c_ulong,
        _pressed: bool,
        _target_guard: Option<&TargetInputGuardProof>,
    ) -> Result<Option<TargetInputGuardProof>, &'static str> {
        Err("linux_atomic_target_input_requires_native_media")
    }

    #[cfg(feature = "native-media")]
    fn execute_release_button(&self, button: u8) -> Result<(), &'static str> {
        self.fake_input(BUTTON_RELEASE, button, 0, 0)?;
        self.barrier()
    }

    #[cfg(not(feature = "native-media"))]
    fn execute_release_button(&self, _button: u8) -> Result<(), &'static str> {
        Err("linux_atomic_target_input_requires_native_media")
    }

    #[cfg(feature = "native-media")]
    fn execute(
        &self,
        target_guard: Option<&TargetInputGuardProof>,
        pointer: Option<(i16, i16)>,
        inject: impl FnOnce(&Self) -> Result<(), &'static str>,
    ) -> Result<Option<TargetInputGuardProof>, &'static str> {
        let Some(target_guard) = target_guard else {
            inject(self)?;
            self.barrier()?;
            return Ok(None);
        };

        let grab = X11ServerGrab::begin(&self.connection)?;
        let guard_acquired_at_ms = now_ms();
        self.validate_target(target_guard, pointer)?;
        let validated_at_ms = now_ms();
        inject(self)?;
        self.barrier()?;
        let injected_at_ms = now_ms();
        grab.release()?;
        let guard_released_at_ms = now_ms();
        Ok(Some(target_guard.clone().with_x11_atomicity(
            guard_acquired_at_ms,
            validated_at_ms,
            injected_at_ms,
            guard_released_at_ms,
        )))
    }

    #[cfg(feature = "native-media")]
    fn validate_target(
        &self,
        proof: &TargetInputGuardProof,
        pointer: Option<(i16, i16)>,
    ) -> Result<(), &'static str> {
        let authorized = proof.authorized_window_ids();
        if authorized.is_empty() || authorized.len() > MAX_ATOMIC_TARGET_WINDOWS {
            return Err("linux_target_window_set_invalid");
        }
        let expected_pid = proof
            .expected_pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or("linux_target_owner_identity_unavailable")?;
        let expected_process_instance_id = proof
            .expected_process_instance_id()
            .ok_or("linux_target_process_instance_identity_unavailable")?;
        let current_process_instance_id = crate::daemon::ability::builtins::resources::media::linux_x11_window_owner::LinuxProcessInstance::resolve(expected_pid)
            .map_err(|_| "linux_target_process_instance_unavailable")?
            .stable_id();
        if current_process_instance_id != expected_process_instance_id {
            return Err("linux_target_process_instance_identity_mismatch");
        }
        let authorized = authorized
            .iter()
            .map(|window_id| {
                u32::try_from(*window_id)
                    .map(x::Window::new)
                    .map_err(|_| "linux_target_window_id_invalid")
            })
            .collect::<Result<Vec<_>, _>>()?;

        for window in &authorized {
            if self.local_client_pid(*window)? != Some(expected_pid) {
                return Err("linux_target_owner_identity_mismatch");
            }
        }

        let active = if let Some((x, y)) = pointer {
            let translated = self
                .connection
                .wait_for_reply(self.connection.send_request(&x::TranslateCoordinates {
                    src_window: self.root,
                    dst_window: self.root,
                    src_x: x,
                    src_y: y,
                }))
                .map_err(|_| "linux_target_pointer_query_failed")?;
            if !translated.same_screen() || translated.child() == x::Window::none() {
                return Err("target_input_guard_pointer_outside_target_surface");
            }
            translated.child()
        } else {
            let focus = self
                .connection
                .wait_for_reply(self.connection.send_request(&x::GetInputFocus {}))
                .map_err(|_| "linux_target_focus_query_failed")?
                .focus();
            if focus == x::Window::none() || focus.resource_id() == 1 {
                return Err("target_input_guard_not_focused");
            }
            focus
        };

        if authorized
            .iter()
            .any(|candidate| self.windows_related(active, *candidate).unwrap_or(false))
        {
            Ok(())
        } else if pointer.is_some() {
            Err("target_input_guard_pointer_occluded")
        } else {
            Err("target_input_guard_not_focused")
        }
    }

    #[cfg(feature = "native-media")]
    fn windows_related(&self, left: x::Window, right: x::Window) -> Result<bool, &'static str> {
        Ok(self.ancestor_chain(left)?.contains(&right)
            || self.ancestor_chain(right)?.contains(&left))
    }

    #[cfg(feature = "native-media")]
    fn ancestor_chain(&self, mut window: x::Window) -> Result<Vec<x::Window>, &'static str> {
        let mut chain = Vec::with_capacity(8);
        for _ in 0..64 {
            chain.push(window);
            if window == self.root || window == x::Window::none() {
                return Ok(chain);
            }
            let tree = self
                .connection
                .wait_for_reply(self.connection.send_request(&x::QueryTree { window }))
                .map_err(|_| "linux_target_window_tree_query_failed")?;
            let parent = tree.parent();
            if parent == window {
                return Err("linux_target_window_tree_cycle");
            }
            window = parent;
        }
        Err("linux_target_window_tree_too_deep")
    }

    #[cfg(feature = "native-media")]
    fn local_client_pid(&self, window: x::Window) -> Result<Option<u32>, &'static str> {
        let specs = [res::ClientIdSpec {
            client: window.resource_id(),
            mask: res::ClientIdMask::LOCAL_CLIENT_PID,
        }];
        let reply = self
            .connection
            .wait_for_reply(
                self.connection
                    .send_request(&res::QueryClientIds { specs: &specs }),
            )
            .map_err(|_| "linux_target_owner_query_failed")?;
        Ok(reply.ids().find_map(|client_id| {
            client_id
                .spec()
                .mask
                .contains(res::ClientIdMask::LOCAL_CLIENT_PID)
                .then(|| client_id.value().first().copied())
                .flatten()
        }))
    }

    #[cfg(feature = "native-media")]
    fn inject_pointer(
        &self,
        operation: PointerOperation,
        (x, y): (i16, i16),
    ) -> Result<(), &'static str> {
        self.fake_input(MOTION_NOTIFY, 0, x, y)?;
        match operation {
            PointerOperation::Move => Ok(()),
            PointerOperation::Button { button, pressed } => self.fake_input(
                if pressed {
                    BUTTON_PRESS
                } else {
                    BUTTON_RELEASE
                },
                button,
                0,
                0,
            ),
            PointerOperation::Wheel {
                vertical,
                horizontal,
            } => apply_wheel(self, vertical, horizontal),
        }
        .map_err(|_| LINUX_XTEST_PARTIAL_EFFECT)
    }

    #[cfg(feature = "native-media")]
    fn fake_input(
        &self,
        event_type: u8,
        detail: u8,
        root_x: i16,
        root_y: i16,
    ) -> Result<(), &'static str> {
        let cookie = self.connection.send_request_checked(&xtest::FakeInput {
            r#type: event_type,
            detail,
            time: CURRENT_TIME,
            root: self.root,
            root_x,
            root_y,
            deviceid: 0,
        });
        self.connection
            .check_request(cookie)
            .map_err(|_| LINUX_XTEST_DENIED)
    }

    #[cfg(feature = "native-media")]
    fn barrier(&self) -> Result<(), &'static str> {
        self.connection
            .wait_for_reply(self.connection.send_request(&x::GetInputFocus {}))
            .map(|_| ())
            .map_err(|_| LINUX_XTEST_DENIED)
    }
}

impl Drop for X11TargetInputExecutor {
    fn drop(&mut self) {
        if !self.keymap_display.is_null() {
            unsafe {
                (self.x_close_display)(self.keymap_display);
            }
            self.keymap_display = std::ptr::null_mut();
        }
    }
}

#[cfg(feature = "native-media")]
struct X11ServerGrab<'a> {
    connection: &'a xcb::Connection,
    released: bool,
}

#[cfg(feature = "native-media")]
impl<'a> X11ServerGrab<'a> {
    fn begin(connection: &'a xcb::Connection) -> Result<Self, &'static str> {
        let cookie = connection.send_request_checked(&x::GrabServer {});
        connection
            .check_request(cookie)
            .map_err(|_| "linux_x11_server_grab_failed")?;
        connection
            .wait_for_reply(connection.send_request(&x::GetInputFocus {}))
            .map_err(|_| "linux_x11_server_grab_barrier_failed")?;
        Ok(Self {
            connection,
            released: false,
        })
    }

    fn release(mut self) -> Result<(), &'static str> {
        self.release_inner()?;
        self.released = true;
        Ok(())
    }

    fn release_inner(&self) -> Result<(), &'static str> {
        let cookie = self.connection.send_request_checked(&x::UngrabServer {});
        self.connection
            .check_request(cookie)
            .map_err(|_| "linux_x11_server_ungrab_failed")?;
        self.connection
            .wait_for_reply(self.connection.send_request(&x::GetInputFocus {}))
            .map(|_| ())
            .map_err(|_| "linux_x11_server_ungrab_barrier_failed")
    }
}

#[cfg(feature = "native-media")]
impl Drop for X11ServerGrab<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.release_inner();
        }
    }
}

fn atomic_coordinates(x: f64, y: f64) -> Result<(i16, i16), &'static str> {
    if !x.is_finite()
        || !y.is_finite()
        || x < f64::from(i16::MIN)
        || x > f64::from(i16::MAX)
        || y < f64::from(i16::MIN)
        || y > f64::from(i16::MAX)
    {
        return Err("linux_xtest_coordinates_out_of_range");
    }
    Ok((x.round() as i16, y.round() as i16))
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

fn button_number(button: u8) -> Option<u8> {
    match button {
        0 => Some(1),
        1 => Some(2),
        2 => Some(3),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerOperation {
    Move,
    Button { button: u8, pressed: bool },
    Wheel { vertical: i32, horizontal: i32 },
}

fn pointer_operation(frame: &PointerInputFrame) -> Result<PointerOperation, &'static str> {
    match frame.action.as_str() {
        "move" => Ok(PointerOperation::Move),
        "down" | "up" => button_number(frame.button.unwrap_or(0))
            .map(|button| PointerOperation::Button {
                button,
                pressed: frame.action == "down",
            })
            .ok_or("unsupported_pointer_button"),
        "wheel" => {
            let vertical = x11_detent_steps(frame.delta_y);
            let horizontal = x11_detent_steps(frame.delta_x);
            if vertical == 0 && horizontal == 0 {
                Err("invalid_wheel_delta")
            } else {
                Ok(PointerOperation::Wheel {
                    vertical,
                    horizontal,
                })
            }
        }
        _ => Err("unsupported_pointer_action"),
    }
}

#[cfg(feature = "native-media")]
fn apply_wheel(
    backend: &X11TargetInputExecutor,
    vertical: i32,
    horizontal: i32,
) -> Result<(), &'static str> {
    wheel_buttons(vertical, 4, 5)
        .chain(wheel_buttons(horizontal, 6, 7))
        .try_for_each(|button| {
            backend.fake_input(BUTTON_PRESS, button, 0, 0)?;
            backend.fake_input(BUTTON_RELEASE, button, 0, 0)
        })
}

fn wheel_buttons(steps: i32, negative_button: u8, positive_button: u8) -> impl Iterator<Item = u8> {
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
    let keysym = match PhysicalKey::from_dom_code(code)? {
        PhysicalKey::Letter(letter) => c_ulong::from(letter.to_ascii_lowercase()),
        PhysicalKey::Digit(digit) => c_ulong::from(digit),
        PhysicalKey::Function(function) => 0xffbe + c_ulong::from(function - 1),
        PhysicalKey::NumpadDigit(digit) => 0xffb0 + c_ulong::from(digit - b'0'),
        PhysicalKey::Enter => 0xff0d,
        PhysicalKey::NumpadEnter => 0xff8d,
        PhysicalKey::Tab => 0xff09,
        PhysicalKey::Space => 0x20,
        PhysicalKey::Backspace => 0xff08,
        PhysicalKey::Escape => 0xff1b,
        PhysicalKey::CapsLock => 0xffe5,
        PhysicalKey::NumLock => 0xff7f,
        PhysicalKey::ScrollLock => 0xff14,
        PhysicalKey::PrintScreen => 0xff61,
        PhysicalKey::Pause => 0xff13,
        PhysicalKey::ContextMenu => 0xff67,
        PhysicalKey::ShiftLeft => 0xffe1,
        PhysicalKey::ShiftRight => 0xffe2,
        PhysicalKey::ControlLeft => 0xffe3,
        PhysicalKey::ControlRight => 0xffe4,
        PhysicalKey::AltLeft => 0xffe9,
        PhysicalKey::AltRight => 0xffea,
        PhysicalKey::MetaLeft => 0xffeb,
        PhysicalKey::MetaRight => 0xffec,
        PhysicalKey::ArrowLeft => 0xff51,
        PhysicalKey::ArrowUp => 0xff52,
        PhysicalKey::ArrowRight => 0xff53,
        PhysicalKey::ArrowDown => 0xff54,
        PhysicalKey::Insert => 0xff63,
        PhysicalKey::Delete => 0xffff,
        PhysicalKey::Home => 0xff50,
        PhysicalKey::End => 0xff57,
        PhysicalKey::PageUp => 0xff55,
        PhysicalKey::PageDown => 0xff56,
        PhysicalKey::Minus => 0x2d,
        PhysicalKey::Equal => 0x3d,
        PhysicalKey::BracketLeft => 0x5b,
        PhysicalKey::BracketRight => 0x5d,
        PhysicalKey::Backslash => 0x5c,
        PhysicalKey::Semicolon => 0x3b,
        PhysicalKey::Quote => 0x27,
        PhysicalKey::Backquote => 0x60,
        PhysicalKey::Comma => 0x2c,
        PhysicalKey::Period => 0x2e,
        PhysicalKey::Slash => 0x2f,
        PhysicalKey::NumpadDecimal => 0xffae,
        PhysicalKey::NumpadMultiply => 0xffaa,
        PhysicalKey::NumpadAdd => 0xffab,
        PhysicalKey::NumpadSubtract => 0xffad,
        PhysicalKey::NumpadDivide => 0xffaf,
        PhysicalKey::NumpadEqual => 0xffbd,
    };
    Some(keysym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_wheel_expansion_is_bounded() {
        assert_eq!(x11_detent_steps(Some(10_000.0)), 12);
        assert_eq!(x11_detent_steps(Some(-10_000.0)), -12);
        assert_eq!(x11_detent_steps(Some(0.0)), 0);
        assert_eq!(x11_detent_steps(None), 0);
        assert_eq!(wheel_buttons(3, 4, 5).collect::<Vec<_>>(), vec![5, 5, 5]);
        assert_eq!(wheel_buttons(-2, 4, 5).collect::<Vec<_>>(), vec![4, 4]);
    }

    #[test]
    fn rejected_pointer_operations_are_resolved_before_native_injection() {
        let frame = |action: &str, button: Option<u8>, delta_y: Option<f64>| PointerInputFrame {
            action: action.to_string(),
            x: 10.0,
            y: 20.0,
            normalized_x: None,
            normalized_y: None,
            target_width: None,
            target_height: None,
            target_geometry_revision: None,
            target_focus_epoch: None,
            button,
            delta_x: None,
            delta_y,
            sent_at_ms: None,
            client_sequence: None,
        };

        assert_eq!(
            pointer_operation(&frame("down", Some(9), None)),
            Err("unsupported_pointer_button")
        );
        assert_eq!(
            pointer_operation(&frame("wheel", None, Some(0.0))),
            Err("invalid_wheel_delta")
        );
        assert_eq!(
            pointer_operation(&frame("unknown", None, None)),
            Err("unsupported_pointer_action")
        );
        assert!(matches!(
            pointer_operation(&frame("down", Some(0), None)),
            Ok(PointerOperation::Button {
                button: 1,
                pressed: true
            })
        ));
    }

    #[test]
    fn linux_dom_key_mapping_is_deterministic() {
        assert_eq!(dom_code_keysym("KeyA"), Some(c_ulong::from(b'a')));
        assert_eq!(dom_code_keysym("ArrowLeft"), Some(0xff51));
        assert_eq!(dom_code_keysym("F12"), Some(0xffc9));
        assert_eq!(dom_code_keysym("NumpadEnter"), Some(0xff8d));
        assert_eq!(unicode_keysym("é"), Some(0xe9));
        assert_eq!(unicode_keysym("你"), Some(0x0100_4f60));
        assert_eq!(unicode_keysym("ab"), None);
    }
}
