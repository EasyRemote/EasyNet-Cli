// EasyNet RemoteApp — Linux X11 target-focus backend
// ===================================================
//
// Xlib/XCB are loaded dynamically so headless daemon builds do not acquire a
// hard GUI-system dependency. Pure Wayland remains fail-closed until a portal
// RemoteDesktop session identity is committed into the target binding.

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::sync::OnceLock;

use easynet_remoteapp_native_platform::PlatformWindowProcessIdentityProvider;
use libloading::Library;

use super::{
    RemoteAppTargetBinding, RemoteAppTargetFocusError, TargetFocusFailureReason,
    TargetTrackerSnapshot,
};

const CLIENT_MESSAGE: u8 = 33;
const CURRENT_TIME: u32 = 0;
const SUBSTRUCTURE_NOTIFY_MASK: u32 = 1 << 19;
const SUBSTRUCTURE_REDIRECT_MASK: u32 = 1 << 20;

static XLIB_THREADS: OnceLock<Result<(), &'static str>> = OnceLock::new();

type XInitThreads = unsafe extern "C" fn() -> c_int;
type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type XCloseDisplay = unsafe extern "C" fn(*mut c_void) -> c_int;
type XDefaultScreen = unsafe extern "C" fn(*mut c_void) -> c_int;
type XRootWindow = unsafe extern "C" fn(*mut c_void, c_int) -> c_ulong;
type XInternAtom = unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong;
type XcbConnect = unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut c_void;
type XcbConnectionHasError = unsafe extern "C" fn(*mut c_void) -> c_int;
type XcbDisconnect = unsafe extern "C" fn(*mut c_void);
type XcbFlush = unsafe extern "C" fn(*mut c_void) -> c_int;
type XcbRequestCheck = unsafe extern "C" fn(*mut c_void, XcbVoidCookie) -> *mut XcbGenericError;
type XcbSendEventChecked =
    unsafe extern "C" fn(*mut c_void, u8, u32, u32, *const c_char) -> XcbVoidCookie;

#[repr(C)]
#[derive(Clone, Copy)]
struct XcbVoidCookie {
    sequence: u32,
}

#[repr(C)]
struct XcbGenericError {
    response_type: u8,
    error_code: u8,
    sequence: u16,
    resource_id: u32,
    minor_code: u16,
    major_code: u8,
    pad0: u8,
    pad: [u32; 5],
    full_sequence: u32,
}

#[repr(C)]
struct XcbClientMessageEvent {
    response_type: u8,
    format: u8,
    sequence: u16,
    window: u32,
    message_type: u32,
    data: [u32; 5],
}

const _: () = assert!(std::mem::size_of::<XcbClientMessageEvent>() == 32);

struct X11ServerTarget {
    root: u32,
    active_window_atom: u32,
}

struct XcbFocusConnection {
    connection: *mut c_void,
    disconnect: XcbDisconnect,
    flush: XcbFlush,
    request_check: XcbRequestCheck,
    send_event_checked: XcbSendEventChecked,
    _library: Library,
}

pub(super) fn request_focus(
    binding: &RemoteAppTargetBinding,
    _snapshot: &TargetTrackerSnapshot,
    window_id: u64,
) -> Result<&'static str, RemoteAppTargetFocusError> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusUnsupported,
            "Wayland target focus requires a bound portal RemoteDesktop session",
        ));
    }
    if std::env::var_os("DISPLAY").is_none() {
        return Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusUnsupported,
            "Linux X11 display is unavailable",
        ));
    }
    let window = u32::try_from(window_id).map_err(|_| {
        RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusStale,
            format!("selected X11 window id {window_id} is out of range"),
        )
    })?;
    let provider = PlatformWindowProcessIdentityProvider::connect()
        .map_err(|error| stale(format!("initialize Linux process identity: {error}")))?;
    let observed = provider
        .resolve_window(window_id)
        .map_err(|error| stale(format!("resolve selected X11 window owner: {error}")))?
        .ok_or_else(|| stale(format!("selected X11 window {window_id} has no XRes owner")))?;
    let expected_pid = binding
        .native_locator()
        .pid()
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| stale("selected X11 window has no committed owner pid"))?;
    let expected_process_instance_id = binding
        .native_locator()
        .process_instance_id()
        .ok_or_else(|| stale("selected X11 window has no committed process instance"))?;
    if observed.pid() != expected_pid || observed.stable_id() != expected_process_instance_id {
        return Err(stale(format!(
            "selected X11 window {window_id} changed owner process instance"
        )));
    }
    let server_target = X11ServerTarget::discover()?;
    let transport = XcbFocusConnection::connect()?;

    // EWMH source indication 2 identifies a pager/window-management request.
    // The WM retains focus-stealing policy while the exact committed XID is
    // preserved. A fresh xcap snapshot remains the authoritative proof.
    let event = XcbClientMessageEvent {
        response_type: CLIENT_MESSAGE,
        format: 32,
        sequence: 0,
        window,
        message_type: server_target.active_window_atom,
        data: [2, CURRENT_TIME, 0, 0, 0],
    };
    transport.send_checked(
        server_target.root,
        &event,
        SUBSTRUCTURE_REDIRECT_MASK | SUBSTRUCTURE_NOTIFY_MASK,
    )?;
    Ok("linux_x11_ewmh_verified_snapshot")
}

impl X11ServerTarget {
    fn discover() -> Result<Self, RemoteAppTargetFocusError> {
        let library = load_library(&["libX11.so.6", "libX11.so"])
            .ok_or_else(|| focus_failed("X11 client library is unavailable"))?;
        let x_init_threads: XInitThreads = unsafe { load_symbol(&library, b"XInitThreads\0")? };
        let initialized = XLIB_THREADS.get_or_init(|| {
            (unsafe { x_init_threads() } != 0)
                .then_some(())
                .ok_or("XInitThreads failed")
        });
        if let Err(detail) = initialized {
            return Err(focus_failed(*detail));
        }
        let x_open_display: XOpenDisplay = unsafe { load_symbol(&library, b"XOpenDisplay\0")? };
        let x_close_display: XCloseDisplay = unsafe { load_symbol(&library, b"XCloseDisplay\0")? };
        let x_default_screen: XDefaultScreen =
            unsafe { load_symbol(&library, b"XDefaultScreen\0")? };
        let x_root_window: XRootWindow = unsafe { load_symbol(&library, b"XRootWindow\0")? };
        let x_intern_atom: XInternAtom = unsafe { load_symbol(&library, b"XInternAtom\0")? };
        let display = unsafe { x_open_display(std::ptr::null()) };
        if display.is_null() {
            return Err(focus_failed(
                "could not connect to the selected X11 display",
            ));
        }
        let screen = unsafe { x_default_screen(display) };
        let root = unsafe { x_root_window(display, screen) };
        let active_window_atom =
            unsafe { x_intern_atom(display, c"_NET_ACTIVE_WINDOW".as_ptr(), 1) };
        unsafe { x_close_display(display) };
        let root = u32::try_from(root)
            .ok()
            .filter(|root| *root != 0)
            .ok_or_else(|| focus_failed("selected X11 root window is unavailable"))?;
        let active_window_atom = u32::try_from(active_window_atom)
            .ok()
            .filter(|atom| *atom != 0)
            .ok_or_else(|| {
                RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusUnsupported,
                    "X11 window manager does not expose _NET_ACTIVE_WINDOW",
                )
            })?;
        Ok(Self {
            root,
            active_window_atom,
        })
    }
}

impl XcbFocusConnection {
    fn connect() -> Result<Self, RemoteAppTargetFocusError> {
        let library = load_library(&["libxcb.so.1", "libxcb.so"])
            .ok_or_else(|| focus_failed("XCB client library is unavailable"))?;
        let connect: XcbConnect = unsafe { load_symbol(&library, b"xcb_connect\0")? };
        let connection_has_error: XcbConnectionHasError =
            unsafe { load_symbol(&library, b"xcb_connection_has_error\0")? };
        let disconnect: XcbDisconnect = unsafe { load_symbol(&library, b"xcb_disconnect\0")? };
        let flush: XcbFlush = unsafe { load_symbol(&library, b"xcb_flush\0")? };
        let request_check: XcbRequestCheck =
            unsafe { load_symbol(&library, b"xcb_request_check\0")? };
        let send_event_checked: XcbSendEventChecked =
            unsafe { load_symbol(&library, b"xcb_send_event_checked\0")? };
        let mut screen = 0;
        let connection = unsafe { connect(std::ptr::null(), &mut screen) };
        if connection.is_null() || unsafe { connection_has_error(connection) } != 0 {
            if !connection.is_null() {
                unsafe { disconnect(connection) };
            }
            return Err(focus_failed("could not open a checked XCB connection"));
        }
        Ok(Self {
            connection,
            disconnect,
            flush,
            request_check,
            send_event_checked,
            _library: library,
        })
    }

    fn send_checked(
        &self,
        root: u32,
        event: &XcbClientMessageEvent,
        event_mask: u32,
    ) -> Result<(), RemoteAppTargetFocusError> {
        let cookie = unsafe {
            (self.send_event_checked)(
                self.connection,
                0,
                root,
                event_mask,
                (event as *const XcbClientMessageEvent).cast(),
            )
        };
        let protocol_error = unsafe { (self.request_check)(self.connection, cookie) };
        if !protocol_error.is_null() {
            let error_code = unsafe { (*protocol_error).error_code };
            unsafe { libc::free(protocol_error.cast()) };
            return Err(focus_failed(format!(
                "X11 focus request failed with protocol error {error_code}"
            )));
        }
        if unsafe { (self.flush)(self.connection) } <= 0 {
            return Err(focus_failed("X11 focus request flush failed"));
        }
        Ok(())
    }
}

impl Drop for XcbFocusConnection {
    fn drop(&mut self) {
        if !self.connection.is_null() {
            unsafe { (self.disconnect)(self.connection) };
            self.connection = std::ptr::null_mut();
        }
    }
}

fn load_library(candidates: &[&str]) -> Option<Library> {
    candidates
        .iter()
        .find_map(|candidate| unsafe { Library::new(candidate).ok() })
}

unsafe fn load_symbol<T: Copy>(
    library: &Library,
    symbol: &[u8],
) -> Result<T, RemoteAppTargetFocusError> {
    unsafe { library.get::<T>(symbol) }
        .map(|symbol| *symbol)
        .map_err(|_| focus_failed("required X11 focus symbol is unavailable"))
}

fn focus_failed(detail: impl Into<String>) -> RemoteAppTargetFocusError {
    RemoteAppTargetFocusError::new(TargetFocusFailureReason::TargetFocusFailed, detail)
}

fn stale(detail: impl Into<String>) -> RemoteAppTargetFocusError {
    RemoteAppTargetFocusError::new(TargetFocusFailureReason::TargetFocusStale, detail)
}
