// EasyNet CLI — daemon-local media resource bootstrap
// ===================================================
//
// Registers the local host's default media resources into
// ~/.easynet/resources.json so media abilities have resource_ura
// subjects to bind against. Handlers still perform live availability
// checks at invocation time; this module only makes stable URAs
// discoverable through meta.list_resources.

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Value};

use crate::persistence::resources::{
    self, upsert_resource, ResourceBinding, ResourceType, ResourceUpsert, ResourcesFile,
};

#[derive(Debug, Clone, PartialEq)]
struct DiscoveredResource {
    kind: ResourceType,
    hardware_id: String,
    display_name: String,
    metadata: Value,
}

#[derive(Debug, Default)]
struct DiscoveredResources {
    resources: Vec<DiscoveredResource>,
    screen_targets_scanned: bool,
}

/// Seed/update the resources table for this daemon's local media
/// surfaces. Returns the number of resource rows known after the
/// pass. This is best-effort: individual platform probes may fail
/// without aborting daemon boot, but a malformed existing
/// resources.json is still returned as an error.
pub fn seed_default_device_resources(realm: &str, owner_agent: &str) -> anyhow::Result<usize> {
    let realm = realm.trim();
    if realm.is_empty() {
        return Ok(0);
    }

    let mut file = resources::load()?;
    let discovered = discover_default_resources();
    if discovered.screen_targets_scanned {
        prune_stale_auto_screen_targets(&mut file, realm, owner_agent, &discovered.resources);
    }
    for resource in discovered.resources {
        apply_discovered_resource(&mut file, realm, owner_agent, resource);
    }
    resources::save(&file)?;
    Ok(file.resources.len())
}

fn prune_stale_auto_screen_targets(
    file: &mut ResourcesFile,
    realm: &str,
    owner_agent: &str,
    discovered: &[DiscoveredResource],
) {
    let live: HashSet<&str> = discovered
        .iter()
        .filter(|resource| {
            matches!(
                resource.kind,
                ResourceType::Application | ResourceType::Window
            )
        })
        .map(|resource| resource.hardware_id.as_str())
        .collect();
    file.resources.retain(|resource| {
        let auto_bootstrap_screen_target = resource
            .metadata
            .get("discovery_source")
            .and_then(Value::as_str)
            == Some("auto_bootstrap");
        let legacy_bootstrap_screen_target = resource
            .metadata
            .get("capture_target")
            .and_then(Value::as_str)
            .is_some_and(|target| matches!(target, "window" | "application"))
            && resource
                .metadata
                .get("backend")
                .and_then(Value::as_str)
                .is_some_and(|backend| matches!(backend, "xcap" | "macos_core_graphics"));
        let auto_prunable_screen_target =
            matches!(
                resource.kind,
                ResourceType::Application | ResourceType::Window
            ) && (resource.metadata.get("auto_prune").and_then(Value::as_bool) == Some(true)
                || auto_bootstrap_screen_target
                || legacy_bootstrap_screen_target);
        let owned_by_this_daemon =
            resource.owner_agent == owner_agent && resource_ura_belongs_to_realm(resource, realm);
        !auto_prunable_screen_target
            || !owned_by_this_daemon
            || live.contains(resource.hardware_id.as_str())
    });
}

fn resource_ura_belongs_to_realm(
    resource: &crate::persistence::resources::ResourceEntry,
    realm: &str,
) -> bool {
    crate::ura::parse_ura(&resource.resource_ura)
        .map(|parsed| parsed.realm == realm)
        .unwrap_or(false)
}

fn apply_discovered_resource(
    file: &mut ResourcesFile,
    realm: &str,
    owner_agent: &str,
    resource: DiscoveredResource,
) {
    upsert_resource(
        file,
        ResourceUpsert {
            realm,
            owner_agent,
            kind: resource.kind,
            binding: ResourceBinding::LocalDevice,
            hardware_id: &resource.hardware_id,
            display_name: &resource.display_name,
            metadata: resource.metadata,
        },
    );
}

fn discover_default_resources() -> DiscoveredResources {
    let mut discovered = DiscoveredResources::default();
    if let Some(mic) = discover_default_mic() {
        discovered.resources.push(mic);
    }
    discovered.resources.extend(discover_displays());
    match discover_screen_targets() {
        Ok(targets) => {
            discovered.screen_targets_scanned = true;
            discovered.resources.extend(targets);
        }
        Err(err) => {
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = screen_target_discovery_failed,
                reason = err.to_string(),
            );
        }
    }
    discovered.resources.extend(discover_cameras());
    discovered
}

fn discover_default_mic() -> Option<DiscoveredResource> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let device = host.default_input_device()?;
    let name = device
        .name()
        .unwrap_or_else(|_| "Default microphone".into());
    let config = device.default_input_config().ok();
    let metadata = match config {
        Some(cfg) => json!({
            "backend": "cpal",
            "is_default": true,
            "host": format!("{:?}", host.id()),
            "sample_rate": cfg.sample_rate().0,
            "channels": cfg.channels(),
            "sample_format": format!("{:?}", cfg.sample_format()),
        }),
        None => json!({
            "backend": "cpal",
            "is_default": true,
            "host": format!("{:?}", host.id()),
        }),
    };
    Some(DiscoveredResource {
        kind: ResourceType::Mic,
        hardware_id: format!("mic:cpal:default:{name}"),
        display_name: name,
        metadata,
    })
}

fn discover_displays() -> Vec<DiscoveredResource> {
    let Ok(monitors) = xcap::Monitor::all() else {
        return Vec::new();
    };
    monitors
        .into_iter()
        .enumerate()
        .map(|(idx, monitor)| {
            let id = monitor.id().ok();
            let name = monitor
                .name()
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| format!("Display {}", idx + 1));
            let width = monitor.width().ok();
            let height = monitor.height().ok();
            let x = monitor.x().ok();
            let y = monitor.y().ok();
            let is_primary = monitor.is_primary().ok();
            let fallback_id = format!(
                "display:xcap:{idx}:{}x{}:{name}",
                width.unwrap_or(0),
                height.unwrap_or(0)
            );
            DiscoveredResource {
                kind: ResourceType::Display,
                hardware_id: id
                    .map(|v| format!("display:xcap:{v}"))
                    .unwrap_or(fallback_id),
                display_name: name,
                metadata: json!({
                    "backend": "xcap",
                    "monitor_index": idx,
                    "monitor_id": id,
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height,
                    "is_primary": is_primary,
                }),
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn discover_screen_targets() -> anyhow::Result<Vec<DiscoveredResource>> {
    match macos_screen_targets::discover() {
        Ok(targets) if !targets.is_empty() => Ok(targets),
        Ok(_) => {
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = native_screen_target_discovery_empty,
                fallback = "xcap",
            );
            discover_screen_targets_with_xcap()
        }
        Err(err) => {
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = native_screen_target_discovery_failed,
                reason = err.to_string(),
                fallback = "xcap",
            );
            discover_screen_targets_with_xcap()
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn discover_screen_targets() -> anyhow::Result<Vec<DiscoveredResource>> {
    // Windows and Linux keep using xcap for now, but this call is intentionally
    // isolated so Win32 EnumWindows and Linux portal/window-manager discovery can
    // replace it without touching resource persistence or pruning semantics.
    discover_screen_targets_with_xcap()
}

fn discover_screen_targets_with_xcap() -> anyhow::Result<Vec<DiscoveredResource>> {
    let windows =
        xcap::Window::all().map_err(|err| anyhow::anyhow!("xcap Window::all failed: {err}"))?;
    let mut out = Vec::new();
    let mut apps: BTreeMap<String, AppAggregate> = BTreeMap::new();
    for window in windows {
        let id = match window.id() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let pid = window.pid().ok();
        let app_name = window
            .app_name()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown application".to_string());
        let title = window
            .title()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let x = window.x().ok().map(i64::from);
        let y = window.y().ok().map(i64::from);
        let width = window.width().ok();
        let height = window.height().ok();
        if !is_remote_capture_candidate(&app_name, width, height) {
            continue;
        }
        let minimized = window.is_minimized().ok();
        if minimized == Some(true) {
            continue;
        }
        let focused = window.is_focused().ok();
        let display_name = match &title {
            Some(title) => format!("{app_name} - {title}"),
            None => app_name.clone(),
        };
        let area = screen_target_area(width, height);
        let bounds = ScreenTargetBounds::new(x, y, width, height);
        apps.entry(app_name.clone()).or_default().record_window(
            id,
            pid,
            title.as_deref(),
            area,
            focused == Some(true),
            bounds,
        );
        out.push(DiscoveredResource {
            kind: ResourceType::Window,
            hardware_id: format!("window:xcap:{}:{id}", pid.unwrap_or(0)),
            display_name,
            metadata: json!({
                "backend": "xcap",
                "capture_target": "window",
                "discovery_source": "auto_bootstrap",
                "discovery_scope": "current_visible_windows",
                "auto_prune": true,
                "platform_backend": "xcap_window_all",
                "window_id": id,
                "pid": pid,
                "app_name": app_name,
                "title": title,
                "x": x,
                "y": y,
                "width": width,
                "height": height,
                "focused": focused,
                "minimized": minimized,
            }),
        });
    }
    out.extend(apps.into_iter().map(|(app_name, app)| DiscoveredResource {
        kind: ResourceType::Application,
        hardware_id: format!("application:xcap:{app_name}"),
        display_name: app_name.clone(),
        metadata: json!({
            "backend": "xcap",
            "capture_target": "application",
            "discovery_source": "auto_bootstrap",
            "discovery_scope": "current_visible_windows",
            "auto_prune": true,
            "platform_backend": "xcap_window_all",
            "app_name": app_name,
            "window_count": app.window_count,
            "primary_window_id": app.primary_window_id,
            "primary_pid": app.primary_pid,
            "primary_title": app.primary_title,
            "primary_x": app.primary_bounds.and_then(|bounds| bounds.x),
            "primary_y": app.primary_bounds.and_then(|bounds| bounds.y),
            "primary_width": app.primary_bounds.and_then(|bounds| bounds.width),
            "primary_height": app.primary_bounds.and_then(|bounds| bounds.height),
        }),
    }));
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenTargetBounds {
    x: Option<i64>,
    y: Option<i64>,
    width: Option<u32>,
    height: Option<u32>,
}

impl ScreenTargetBounds {
    fn new(x: Option<i64>, y: Option<i64>, width: Option<u32>, height: Option<u32>) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct AppAggregate {
    window_count: usize,
    primary_window_id: Option<u32>,
    primary_pid: Option<u32>,
    primary_title: Option<String>,
    primary_bounds: Option<ScreenTargetBounds>,
    primary_area: u64,
}

impl AppAggregate {
    fn record_window(
        &mut self,
        window_id: u32,
        pid: Option<u32>,
        title: Option<&str>,
        area: u64,
        focused: bool,
        bounds: ScreenTargetBounds,
    ) {
        self.window_count += 1;
        let better_primary =
            focused || self.primary_window_id.is_none() || area > self.primary_area;
        if better_primary {
            self.primary_window_id = Some(window_id);
            self.primary_pid = pid;
            self.primary_title = title.map(ToOwned::to_owned);
            self.primary_bounds = Some(bounds);
            self.primary_area = area;
        }
    }
}

fn screen_target_area(width: Option<u32>, height: Option<u32>) -> u64 {
    u64::from(width.unwrap_or(0)) * u64::from(height.unwrap_or(0))
}

#[cfg(target_os = "macos")]
mod macos_screen_targets {
    use std::collections::BTreeMap;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr;

    use super::{screen_target_area, AppAggregate, DiscoveredResource, ScreenTargetBounds};
    use crate::persistence::resources::ResourceType;

    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFIndex = isize;
    type CFNumberRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;
    type CGWindowID = u32;

    const KCG_NULL_WINDOW_ID: CGWindowID = 0;
    const KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const KCF_NUMBER_DOUBLE_TYPE: i32 = 13;
    const KCF_NUMBER_SINT64_TYPE: i32 = 4;
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
        fn CFDictionaryGetValueIfPresent(
            dict: CFDictionaryRef,
            key: *const c_void,
            value: *mut *const c_void,
        ) -> u8;
        fn CFGetTypeID(value: CFTypeRef) -> CFTypeID;
        fn CFNumberGetTypeID() -> CFTypeID;
        fn CFNumberGetValue(number: CFNumberRef, theType: i32, valuePtr: *mut c_void) -> u8;
        fn CFRelease(value: *const c_void);
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            cStr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            theString: CFStringRef,
            buffer: *mut c_char,
            bufferSize: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFStringGetCStringPtr(theString: CFStringRef, encoding: u32) -> *const c_char;
        fn CFStringGetTypeID() -> CFTypeID;
    }

    struct CfOwned(*const c_void);

    impl CfOwned {
        fn new_string(value: &str) -> anyhow::Result<Self> {
            let value = CString::new(value)?;
            // SAFETY: CoreFoundation copies the nul-terminated bytes into a new
            // CFString and returns a retained object that this wrapper releases.
            let ptr = unsafe {
                CFStringCreateWithCString(ptr::null(), value.as_ptr(), KCF_STRING_ENCODING_UTF8)
            };
            if ptr.is_null() {
                anyhow::bail!("CFStringCreateWithCString returned null");
            }
            Ok(Self(ptr))
        }

        fn as_ptr(&self) -> *const c_void {
            self.0
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: CfOwned only wraps retained CoreFoundation objects
                // created in this module.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    struct WindowKeys {
        alpha: CfOwned,
        bounds: CfOwned,
        layer: CfOwned,
        name: CfOwned,
        number: CfOwned,
        owner_name: CfOwned,
        owner_pid: CfOwned,
    }

    impl WindowKeys {
        fn new() -> anyhow::Result<Self> {
            Ok(Self {
                alpha: CfOwned::new_string("kCGWindowAlpha")?,
                bounds: CfOwned::new_string("kCGWindowBounds")?,
                layer: CfOwned::new_string("kCGWindowLayer")?,
                name: CfOwned::new_string("kCGWindowName")?,
                number: CfOwned::new_string("kCGWindowNumber")?,
                owner_name: CfOwned::new_string("kCGWindowOwnerName")?,
                owner_pid: CfOwned::new_string("kCGWindowOwnerPID")?,
            })
        }
    }

    pub(super) fn discover() -> anyhow::Result<Vec<DiscoveredResource>> {
        let keys = WindowKeys::new()?;
        // SAFETY: CGWindowListCopyWindowInfo returns a retained CFArray of
        // dictionaries. We release the array after copying out primitive values.
        let array = unsafe {
            CGWindowListCopyWindowInfo(KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, KCG_NULL_WINDOW_ID)
        };
        if array.is_null() {
            anyhow::bail!("CGWindowListCopyWindowInfo returned null");
        }
        let array = CfOwned(array);
        let count = unsafe { CFArrayGetCount(array.as_ptr()) };
        if count < 0 {
            anyhow::bail!("CGWindowListCopyWindowInfo returned a negative count");
        }

        let mut out = Vec::new();
        let mut apps: BTreeMap<String, AppAggregate> = BTreeMap::new();
        for idx in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(array.as_ptr(), idx) as CFDictionaryRef };
            if dict.is_null() {
                continue;
            }

            let app_name = get_string(dict, keys.owner_name.as_ptr())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Unknown application".to_string());
            let title = get_string(dict, keys.name.as_ptr())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let width_height = get_rect(dict, keys.bounds.as_ptr()).and_then(|rect| {
                let width = positive_dimension(rect.size.width)?;
                let height = positive_dimension(rect.size.height)?;
                Some((rect, width, height))
            });
            let Some((rect, width, height)) = width_height else {
                continue;
            };
            if !super::is_remote_capture_candidate(&app_name, Some(width), Some(height)) {
                continue;
            }
            let layer = get_i64(dict, keys.layer.as_ptr()).unwrap_or(0);
            if layer != 0 {
                continue;
            }
            let alpha = get_f64(dict, keys.alpha.as_ptr()).unwrap_or(1.0);
            if alpha <= 0.01 {
                continue;
            }
            let Some(window_id) =
                get_i64(dict, keys.number.as_ptr()).and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let pid = get_i64(dict, keys.owner_pid.as_ptr()).and_then(|value| {
                if value >= 0 {
                    u32::try_from(value).ok()
                } else {
                    None
                }
            });
            let display_name = match &title {
                Some(title) => format!("{app_name} - {title}"),
                None => app_name.clone(),
            };
            let area = screen_target_area(Some(width), Some(height));
            let bounds = ScreenTargetBounds::new(
                Some(rect.origin.x.round() as i64),
                Some(rect.origin.y.round() as i64),
                Some(width),
                Some(height),
            );
            apps.entry(app_name.clone()).or_default().record_window(
                window_id,
                pid,
                title.as_deref(),
                area,
                false,
                bounds,
            );
            out.push(DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: format!("window:macos:cgwindow:{}:{window_id}", pid.unwrap_or(0)),
                display_name,
                metadata: serde_json::json!({
                    "backend": "macos_core_graphics",
                    "capture_target": "window",
                    "discovery_source": "auto_bootstrap",
                    "discovery_scope": "all_windows",
                    "auto_prune": true,
                    "platform_backend": "core_graphics_cgwindowlist",
                    "window_id": window_id,
                    "pid": pid,
                    "app_name": app_name,
                    "title": title,
                    "width": width,
                    "height": height,
                    "x": rect.origin.x.round() as i64,
                    "y": rect.origin.y.round() as i64,
                    "layer": layer,
                    "alpha": alpha,
                }),
            });
        }

        out.extend(apps.into_iter().map(|(app_name, app)| DiscoveredResource {
            kind: ResourceType::Application,
            hardware_id: format!("application:macos:cgwindow:{app_name}"),
            display_name: app_name.clone(),
            metadata: serde_json::json!({
                "backend": "macos_core_graphics",
                "capture_target": "application",
                "discovery_source": "auto_bootstrap",
                "discovery_scope": "all_windows",
                "auto_prune": true,
                "platform_backend": "core_graphics_cgwindowlist",
                "app_name": app_name,
                "window_count": app.window_count,
                "primary_window_id": app.primary_window_id,
                "primary_pid": app.primary_pid,
                "primary_title": app.primary_title,
                "primary_x": app.primary_bounds.and_then(|bounds| bounds.x),
                "primary_y": app.primary_bounds.and_then(|bounds| bounds.y),
                "primary_width": app.primary_bounds.and_then(|bounds| bounds.width),
                "primary_height": app.primary_bounds.and_then(|bounds| bounds.height),
            }),
        }));
        Ok(out)
    }

    fn positive_dimension(value: f64) -> Option<u32> {
        if value.is_finite() && value > 0.0 && value <= f64::from(u32::MAX) {
            Some(value.round() as u32)
        } else {
            None
        }
    }

    fn get_value(dict: CFDictionaryRef, key: *const c_void) -> Option<*const c_void> {
        let mut value = ptr::null();
        let found = unsafe { CFDictionaryGetValueIfPresent(dict, key, &mut value) };
        (found != 0 && !value.is_null()).then_some(value)
    }

    fn get_string(dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
        let value = get_value(dict, key)? as CFStringRef;
        let is_string = unsafe { CFGetTypeID(value) == CFStringGetTypeID() };
        if !is_string {
            return None;
        }
        let ptr = unsafe { CFStringGetCStringPtr(value, KCF_STRING_ENCODING_UTF8) };
        if !ptr.is_null() {
            return unsafe { CStr::from_ptr(ptr) }
                .to_str()
                .ok()
                .map(ToOwned::to_owned);
        }
        let mut buffer = [0 as c_char; 4096];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                KCF_STRING_ENCODING_UTF8,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_str()
            .ok()
            .map(ToOwned::to_owned)
    }

    fn get_i64(dict: CFDictionaryRef, key: *const c_void) -> Option<i64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0_i64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_SINT64_TYPE,
                &mut out as *mut i64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_f64(dict: CFDictionaryRef, key: *const c_void) -> Option<f64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0.0_f64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_DOUBLE_TYPE,
                &mut out as *mut f64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_rect(dict: CFDictionaryRef, key: *const c_void) -> Option<CGRect> {
        let value = get_value(dict, key)? as CFDictionaryRef;
        let mut rect = CGRect::default();
        let ok = unsafe { CGRectMakeWithDictionaryRepresentation(value, &mut rect) };
        (ok != 0).then_some(rect)
    }
}

fn is_remote_capture_candidate(app_name: &str, width: Option<u32>, height: Option<u32>) -> bool {
    if matches!(
        app_name,
        "Accessibility"
            | "AutoFill"
            | "Control Center"
            | "CursorUIViewService"
            | "Dock"
            | "LinkedNotesUIService"
            | "Notification Center"
            | "Simplified Chinese Input Method"
            | "Spotlight"
            | "TextInputSwitcher"
            | "Universal Control"
            | "Window Server"
            | "loginwindow"
    ) {
        return false;
    }
    width.unwrap_or(0) >= 160 && height.unwrap_or(0) >= 120
}

fn discover_cameras() -> Vec<DiscoveredResource> {
    use nokhwa::utils::{ApiBackend, CameraIndex};

    let Ok(cameras) = nokhwa::query(ApiBackend::Auto) else {
        return Vec::new();
    };
    if cameras.is_empty() {
        return Vec::new();
    }
    cameras
        .into_iter()
        .enumerate()
        .map(|(idx, info)| {
            let name = info.human_name();
            let name = if name.trim().is_empty() {
                format!("Camera {}", idx + 1)
            } else {
                name
            };
            let camera_index = match info.index() {
                CameraIndex::Index(n) => Some(*n),
                CameraIndex::String(s) => s.parse::<u32>().ok(),
            }
            .unwrap_or(idx as u32);
            let misc = info.misc();
            let hardware_id = if misc.trim().is_empty() {
                format!("camera:nokhwa:index:{camera_index}")
            } else {
                format!("camera:nokhwa:{misc}")
            };
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id,
                display_name: name,
                metadata: json!({
                    "backend": "nokhwa",
                    "camera_index": camera_index,
                    "description": info.description(),
                    "misc": misc,
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{self, filter_by_kinds};

    #[test]
    fn apply_discovered_resource_mints_stable_resource_ura() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id: "camera:nokhwa:index:0".into(),
                display_name: "Default camera".into(),
                metadata: json!({"camera_index": 0}),
            },
        );
        let first = file.resources[0].resource_ura.clone();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id: "camera:nokhwa:index:0".into(),
                display_name: "Renamed camera".into(),
                metadata: json!({"camera_index": 0}),
            },
        );

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].resource_ura, first);
        assert_eq!(file.resources[0].display_name, "Renamed camera");
    }

    #[test]
    fn seed_default_device_resources_writes_queryable_resources_file() {
        let _g = crate::cli::test_support::HomeGuard::new();
        let count = seed_default_device_resources("acme", "easynet:///r/acme/device/node-1")
            .expect("seed resources");

        let file = resources::load().expect("load resources");
        assert_eq!(
            file.resources.len(),
            count,
            "seed return value must match persisted resource rows"
        );
        for camera in filter_by_kinds(&file, &[ResourceType::Camera]) {
            assert_ne!(camera.display_name, "Default camera");
            assert_ne!(camera.hardware_id, "camera:nokhwa:index:0");
            assert_ne!(
                camera.metadata.get("probe").and_then(|v| v.as_str()),
                Some("deferred_until_invocation"),
                "bootstrap must not persist speculative camera resources"
            );
        }
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_live_window_and_removes_closed_one() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Live".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        );
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:20:200".into(),
                display_name: "Closed".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        );
        prune_stale_auto_screen_targets(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            &[DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Live".into(),
                metadata: json!({"backend": "xcap"}),
            }],
        );

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "window:xcap:10:100");
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_unmarked_resources() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Operator-managed target".into(),
                metadata: json!({"backend": "xcap"}),
            },
        );

        prune_stale_auto_screen_targets(&mut file, "acme", "easynet:///r/acme/device/node-1", &[]);

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "window:xcap:10:100");
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_other_owner_rows() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Closed local".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        );
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-2",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:20:200".into(),
                display_name: "Other owner".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        );
        apply_discovered_resource(
            &mut file,
            "other",
            "easynet:///r/other/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Application,
                hardware_id: "application:xcap:com.other.App".into(),
                display_name: "Other realm".into(),
                metadata: json!({"backend": "xcap"}),
            },
        );

        prune_stale_auto_screen_targets(&mut file, "acme", "easynet:///r/acme/device/node-1", &[]);

        assert_eq!(file.resources.len(), 2);
        assert!(file
            .resources
            .iter()
            .any(|resource| resource.hardware_id == "window:xcap:20:200"));
        assert!(file
            .resources
            .iter()
            .any(|resource| resource.hardware_id == "application:xcap:com.other.App"));
    }

    #[test]
    fn failed_screen_target_scan_does_not_prune_existing_windows() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/device/node-1",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Previously Seen".into(),
                metadata: json!({"backend": "xcap"}),
            },
        );
        let discovered = DiscoveredResources {
            resources: Vec::new(),
            screen_targets_scanned: false,
        };

        if discovered.screen_targets_scanned {
            prune_stale_auto_screen_targets(
                &mut file,
                "acme",
                "easynet:///r/acme/device/node-1",
                &discovered.resources,
            );
        }

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "window:xcap:10:100");
    }

    #[test]
    fn app_aggregate_preserves_primary_window_bounds() {
        let mut aggregate = AppAggregate::default();
        aggregate.record_window(
            10,
            Some(100),
            Some("Small"),
            10_000,
            false,
            ScreenTargetBounds::new(Some(10), Some(20), Some(100), Some(100)),
        );
        aggregate.record_window(
            11,
            Some(100),
            Some("Focused"),
            9_000,
            true,
            ScreenTargetBounds::new(Some(300), Some(400), Some(90), Some(100)),
        );

        assert_eq!(aggregate.primary_window_id, Some(11));
        assert_eq!(
            aggregate.primary_bounds,
            Some(ScreenTargetBounds::new(
                Some(300),
                Some(400),
                Some(90),
                Some(100)
            ))
        );
    }
}
