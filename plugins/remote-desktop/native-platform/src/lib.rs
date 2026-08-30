//! Canonical native identity and capture-eligibility port for RemoteApp.
//!
//! This private crate is the only authority for platform process generations
//! and native window ownership used by inventory, observation, capture, focus,
//! and input. It deliberately contains no session, product lifecycle, Axon, or
//! SDK abstractions.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInstance {
    pid: u32,
    stable_id: String,
    start_ticks: Option<u64>,
    boot_id: Option<String>,
    creation_filetime_ticks: Option<u64>,
}

impl ProcessInstance {
    pub fn resolve(pid: u32) -> anyhow::Result<Self> {
        platform_process_instance(pid)
    }

    pub const fn pid(&self) -> u32 {
        self.pid
    }

    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    pub const fn start_ticks(&self) -> Option<u64> {
        self.start_ticks
    }

    pub fn boot_id(&self) -> Option<&str> {
        self.boot_id.as_deref()
    }

    pub const fn creation_filetime_ticks(&self) -> Option<u64> {
        self.creation_filetime_ticks
    }

    pub fn verify(&self, expected_stable_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.stable_id == expected_stable_id,
            "process instance changed: expected {expected_stable_id:?}, observed {:?}",
            self.stable_id
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn platform_process_instance(pid: u32) -> anyhow::Result<ProcessInstance> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| anyhow::anyhow!("read /proc/{pid}/stat: {error}"))?;
    let start_ticks = parse_linux_process_start_ticks(&stat)?;
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| anyhow::anyhow!("read Linux boot id: {error}"))?
        .trim()
        .to_string();
    anyhow::ensure!(!boot_id.is_empty(), "Linux boot id is empty");
    Ok(ProcessInstance {
        pid,
        stable_id: format!("linux:{boot_id}:{pid}:{start_ticks}"),
        start_ticks: Some(start_ticks),
        boot_id: Some(boot_id),
        creation_filetime_ticks: None,
    })
}

#[cfg(target_os = "windows")]
fn platform_process_instance(pid: u32) -> anyhow::Result<ProcessInstance> {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(anyhow::anyhow!(
            "open Windows process {pid} for creation-time proof: {}",
            io::Error::last_os_error()
        ));
    }
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let succeeded =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe { CloseHandle(process) };
    if succeeded == 0 {
        return Err(anyhow::anyhow!(
            "read Windows process {pid} creation time: {}",
            io::Error::last_os_error()
        ));
    }
    let creation_filetime_ticks =
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    anyhow::ensure!(
        creation_filetime_ticks != 0,
        "Windows process {pid} returned a zero creation time"
    );
    Ok(ProcessInstance {
        pid,
        stable_id: format!("windows:{pid}:{creation_filetime_ticks}"),
        start_ticks: None,
        boot_id: None,
        creation_filetime_ticks: Some(creation_filetime_ticks),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_process_instance(pid: u32) -> anyhow::Result<ProcessInstance> {
    anyhow::bail!(
        "process-instance identity is unsupported on {} for pid {pid}",
        std::env::consts::OS
    )
}

#[cfg(target_os = "linux")]
fn parse_linux_process_start_ticks(stat: &str) -> anyhow::Result<u64> {
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing command terminator"))?;
    let start_ticks = stat[command_end + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing starttime field"))?
        .parse::<u64>()
        .map_err(|error| anyhow::anyhow!("parse Linux process starttime: {error}"))?;
    anyhow::ensure!(start_ticks > 0, "Linux process starttime must be positive");
    Ok(start_ticks)
}

#[cfg(target_os = "linux")]
pub struct PlatformWindowProcessIdentityProvider {
    connection: xcb::Connection,
}

#[cfg(target_os = "linux")]
impl PlatformWindowProcessIdentityProvider {
    pub fn connect() -> anyhow::Result<Self> {
        let (connection, _) =
            xcb::Connection::connect_with_extensions(None, &[xcb::Extension::Res], &[])
                .map_err(|error| anyhow::anyhow!("connect to X server with X-Resource: {error}"))?;
        require_xres_1_2(&connection)?;
        Ok(Self { connection })
    }

    pub fn resolve_window(&self, window_id: u64) -> anyhow::Result<Option<ProcessInstance>> {
        let window_id = u32::try_from(window_id)
            .map_err(|_| anyhow::anyhow!("X11 window id {window_id} is out of range"))?;
        resolve_x11_local_client_pid(&self.connection, window_id)?
            .map(ProcessInstance::resolve)
            .transpose()
    }

    pub fn resolve_process(&self, pid: u32) -> anyhow::Result<ProcessInstance> {
        ProcessInstance::resolve(pid)
    }
}

#[cfg(target_os = "linux")]
pub fn require_xres_1_2(connection: &xcb::Connection) -> anyhow::Result<()> {
    use xcb::res;

    let version = connection
        .wait_for_reply(connection.send_request(&res::QueryVersion {
            client_major: 1,
            client_minor: 2,
        }))
        .map_err(|error| anyhow::anyhow!("query X-Resource version: {error}"))?;
    anyhow::ensure!(
        (version.server_major(), version.server_minor()) >= (1, 2),
        "X-Resource 1.2 is required for local-client PID resolution; server provides {}.{}",
        version.server_major(),
        version.server_minor()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn resolve_x11_local_client_pid(
    connection: &xcb::Connection,
    window_id: u32,
) -> anyhow::Result<Option<u32>> {
    use xcb::res;

    let specs = [res::ClientIdSpec {
        client: window_id,
        mask: res::ClientIdMask::LOCAL_CLIENT_PID,
    }];
    let reply = connection
        .wait_for_reply(connection.send_request(&res::QueryClientIds { specs: &specs }))
        .map_err(|error| {
            anyhow::anyhow!("query X-Resource local-client PID for window {window_id}: {error}")
        })?;
    Ok(reply.ids().find_map(|client_id| {
        client_id
            .spec()
            .mask
            .contains(res::ClientIdMask::LOCAL_CLIENT_PID)
            .then(|| client_id.value().first().copied())
            .flatten()
    }))
}

#[cfg(target_os = "windows")]
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformWindowProcessIdentityProvider;

#[cfg(target_os = "windows")]
impl PlatformWindowProcessIdentityProvider {
    pub fn connect() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn resolve_window(&self, window_id: u64) -> anyhow::Result<Option<ProcessInstance>> {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

        let hwnd = window_id as usize as HWND;
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return Ok(None);
        }
        let mut pid = 0_u32;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 || pid == 0 {
            return Ok(None);
        }
        ProcessInstance::resolve(pid).map(Some)
    }

    pub fn resolve_process(&self, pid: u32) -> anyhow::Result<ProcessInstance> {
        ProcessInstance::resolve(pid)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[derive(Debug, Default, Clone, Copy)]
pub struct PlatformWindowProcessIdentityProvider;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl PlatformWindowProcessIdentityProvider {
    pub fn connect() -> anyhow::Result<Self> {
        anyhow::bail!(
            "window process identity is unsupported on {}",
            std::env::consts::OS
        )
    }

    pub fn resolve_window(&self, _window_id: u64) -> anyhow::Result<Option<ProcessInstance>> {
        Ok(None)
    }

    pub fn resolve_process(&self, pid: u32) -> anyhow::Result<ProcessInstance> {
        ProcessInstance::resolve(pid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureEligibleSurface {
    width: u32,
    height: u32,
    minimized: bool,
    layer: Option<i64>,
    alpha: Option<f64>,
}

impl CaptureEligibleSurface {
    pub const fn xcap(width: u32, height: u32, minimized: bool) -> Self {
        Self {
            width,
            height,
            minimized,
            layer: None,
            alpha: None,
        }
    }

    /// A macOS window that ScreenCaptureKit can address independently of the
    /// currently active Space. `SCContentFilter` supports desktop-independent
    /// windows, so `isOnScreen` is presentation metadata rather than capture
    /// eligibility.
    pub const fn macos_shareable(width: u32, height: u32, layer: i64, alpha: f64) -> Self {
        Self {
            width,
            height,
            minimized: false,
            layer: Some(layer),
            alpha: Some(alpha),
        }
    }

    pub const fn is_eligible(self) -> bool {
        self.width > 0
            && self.height > 0
            && !self.minimized
            && match self.layer {
                Some(layer) => layer == 0,
                None => true,
            }
            && match self.alpha {
                Some(alpha) => alpha > 0.01,
                None => true,
            }
    }
}

/// One CoreGraphics inventory generation used to close ScreenCaptureKit's
/// missing-alpha metadata seam. Callers capture this snapshot once, then use
/// it for every window admitted into the same target resolution operation.
#[cfg(target_os = "macos")]
pub struct MacosWindowSurfaceSnapshot {
    alpha_by_window_id: std::collections::BTreeMap<u32, f64>,
}

#[cfg(target_os = "macos")]
impl MacosWindowSurfaceSnapshot {
    pub fn capture() -> anyhow::Result<Self> {
        use std::ffi::{c_char, c_void, CString};
        use std::ptr;

        type CFArrayRef = *const c_void;
        type CFDictionaryRef = *const c_void;
        type CFIndex = isize;
        type CFNumberRef = *const c_void;
        type CFStringRef = *const c_void;

        const KCG_NULL_WINDOW_ID: u32 = 0;
        const KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
        const KCF_NUMBER_DOUBLE_TYPE: i32 = 13;
        const KCF_NUMBER_SINT64_TYPE: i32 = 4;
        const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
        }

        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
            fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
            fn CFDictionaryGetValueIfPresent(
                dictionary: CFDictionaryRef,
                key: *const c_void,
                value: *mut *const c_void,
            ) -> u8;
            fn CFNumberGetValue(number: CFNumberRef, number_type: i32, value: *mut c_void) -> u8;
            fn CFRelease(value: *const c_void);
            fn CFStringCreateWithCString(
                allocator: *const c_void,
                value: *const c_char,
                encoding: u32,
            ) -> CFStringRef;
        }

        struct CfOwned(*const c_void);

        impl CfOwned {
            fn string(value: &str) -> anyhow::Result<Self> {
                let value = CString::new(value)?;
                let object = unsafe {
                    CFStringCreateWithCString(ptr::null(), value.as_ptr(), KCF_STRING_ENCODING_UTF8)
                };
                anyhow::ensure!(!object.is_null(), "CoreFoundation string allocation failed");
                Ok(Self(object))
            }
        }

        impl Drop for CfOwned {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { CFRelease(self.0) };
                }
            }
        }

        fn dictionary_number<T: Default>(
            dictionary: CFDictionaryRef,
            key: *const c_void,
            number_type: i32,
        ) -> Option<T> {
            let mut value = ptr::null();
            if unsafe { CFDictionaryGetValueIfPresent(dictionary, key, &mut value) } == 0
                || value.is_null()
            {
                return None;
            }
            let mut output = T::default();
            (unsafe {
                CFNumberGetValue(
                    value as CFNumberRef,
                    number_type,
                    (&raw mut output).cast::<c_void>(),
                )
            } != 0)
                .then_some(output)
        }

        let window_number = CfOwned::string("kCGWindowNumber")?;
        let window_alpha = CfOwned::string("kCGWindowAlpha")?;
        let array = unsafe {
            CGWindowListCopyWindowInfo(KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, KCG_NULL_WINDOW_ID)
        };
        anyhow::ensure!(
            !array.is_null(),
            "CoreGraphics window inventory returned null"
        );
        let array = CfOwned(array);
        let count = unsafe { CFArrayGetCount(array.0) };
        anyhow::ensure!(
            count >= 0,
            "CoreGraphics window inventory returned a negative count"
        );

        let mut alpha_by_window_id = std::collections::BTreeMap::new();
        for index in 0..count {
            let dictionary = unsafe { CFArrayGetValueAtIndex(array.0, index) as CFDictionaryRef };
            if dictionary.is_null() {
                continue;
            }
            let Some(window_id) =
                dictionary_number::<i64>(dictionary, window_number.0, KCF_NUMBER_SINT64_TYPE)
                    .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let Some(alpha) =
                dictionary_number::<f64>(dictionary, window_alpha.0, KCF_NUMBER_DOUBLE_TYPE)
            else {
                continue;
            };
            alpha_by_window_id.insert(window_id, alpha);
        }
        Ok(Self { alpha_by_window_id })
    }

    pub fn alpha_for(&self, window_id: u32) -> Option<f64> {
        self.alpha_by_window_id.get(&window_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::CaptureEligibleSurface;

    #[test]
    fn capture_eligibility_rejects_empty_minimized_or_overlay_surfaces() {
        assert!(CaptureEligibleSurface::xcap(1280, 720, false).is_eligible());
        assert!(!CaptureEligibleSurface::xcap(0, 720, false).is_eligible());
        assert!(!CaptureEligibleSurface::xcap(1280, 720, true).is_eligible());
        assert!(CaptureEligibleSurface::macos_shareable(1280, 720, 0, 1.0).is_eligible());
        assert!(!CaptureEligibleSurface::macos_shareable(1280, 720, 1, 1.0).is_eligible());
        assert!(!CaptureEligibleSurface::macos_shareable(1280, 720, 0, 0.0).is_eligible());
    }
}
