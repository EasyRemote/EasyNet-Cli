// EasyNet CLI — daemon-local media resource bootstrap
// ===================================================
//
// Registers the local host's default media resources into
// ~/.easynet/resources.json so media abilities have resource_ura
// subjects to bind against. Handlers still perform live availability
// checks at invocation time; this module only makes stable URAs
// discoverable through meta.list_resources.

#[cfg(not(target_os = "macos"))]
use std::collections::BTreeMap;
use std::collections::{BTreeSet, HashSet};

use serde_json::{json, Value};

use crate::daemon::persistence::resources::{
    self, upsert_resource, ResourceBinding, ResourceEntry, ResourceType, ResourceUpsert,
    ResourcesFile,
};

pub const REMOTE_TARGET_FRESHNESS_TTL_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTargetInventoryRefresh {
    pub observed_at_ms: u64,
    pub freshness_ttl_ms: u64,
    pub resources: Vec<ResourceEntry>,
    pub retired_count: usize,
    pub screen_target_discovery_available: bool,
}

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
    screen_target_discovery: ScreenTargetDiscoveryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScreenTargetDiscoveryState {
    #[default]
    NotAttempted,
    Scanned,
    Unavailable,
}

impl ScreenTargetDiscoveryState {
    fn permits_stale_prune(self) -> bool {
        matches!(self, Self::Scanned)
    }
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
    prune_retired_local_device_owner_rows(&mut file, owner_agent);
    if discovered.screen_target_discovery.permits_stale_prune() {
        prune_stale_auto_screen_targets(&mut file, realm, owner_agent, &discovered.resources);
    }
    for resource in discovered.resources {
        apply_discovered_resource(&mut file, realm, owner_agent, resource)?;
    }
    resources::save(&file)?;
    Ok(file.resources.len())
}

/// Refresh the daemon-local live remote desktop target inventory.
///
/// This is the mutable counterpart to `meta.list_resources`. It is owned by
/// the daemon resource inventory layer, not by the remote desktop plugin:
/// callers use it to obtain a fresh display/window/application projection
/// before invoking product-specific remote desktop session abilities.
pub fn refresh_remote_targets(
    realm: &str,
    owner_agent: &str,
) -> anyhow::Result<RemoteTargetInventoryRefresh> {
    refresh_remote_targets_with_save_policy(realm, owner_agent, RemoteTargetSavePolicy::Always)
}

/// Observe the same host-local target inventory for
/// `resource.watch_remote_targets`.
///
/// The returned projection always carries fresh observation timestamps, but the
/// persistent resource cache is written only when the stable inventory changes
/// (added/removed targets or target identity/geometry metadata changes). This
/// keeps long-lived watches from rewriting `resources.json` on every polling
/// tick solely because freshness fields changed.
pub fn watch_remote_target_inventory(
    realm: &str,
    owner_agent: &str,
) -> anyhow::Result<RemoteTargetInventoryRefresh> {
    refresh_remote_targets_with_save_policy(
        realm,
        owner_agent,
        RemoteTargetSavePolicy::IfStableInventoryChanged,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteTargetSavePolicy {
    Always,
    IfStableInventoryChanged,
}

fn refresh_remote_targets_with_save_policy(
    realm: &str,
    owner_agent: &str,
    save_policy: RemoteTargetSavePolicy,
) -> anyhow::Result<RemoteTargetInventoryRefresh> {
    let realm = realm.trim();
    if realm.is_empty() {
        return Ok(RemoteTargetInventoryRefresh {
            observed_at_ms: unix_ms_now(),
            freshness_ttl_ms: REMOTE_TARGET_FRESHNESS_TTL_MS,
            resources: Vec::new(),
            retired_count: 0,
            screen_target_discovery_available: false,
        });
    }
    let observed_at_ms = unix_ms_now();
    let mut file = resources::load()?;
    let before_signature = stable_remote_target_cache_signature(&file, realm, owner_agent);
    let discovered = discover_remote_target_resources();
    let refresh =
        apply_remote_target_refresh(&mut file, realm, owner_agent, discovered, observed_at_ms)?;
    let after_signature = stable_remote_target_cache_signature(&file, realm, owner_agent);
    if save_policy == RemoteTargetSavePolicy::Always || before_signature != after_signature {
        resources::save(&file)?;
    }
    Ok(refresh)
}

fn stable_remote_target_cache_signature(
    file: &ResourcesFile,
    realm: &str,
    owner_agent: &str,
) -> BTreeSet<String> {
    file.resources
        .iter()
        .filter(|resource| {
            resource.owner_agent == owner_agent
                && resource_ura_belongs_to_realm(resource, realm)
                && is_remote_target_kind(resource.kind)
        })
        .map(stable_remote_target_entry_signature)
        .collect()
}

fn stable_remote_target_entry_signature(resource: &ResourceEntry) -> String {
    let mut metadata = resource.metadata.clone();
    if let Value::Object(map) = &mut metadata {
        map.remove("observed_at_ms");
        map.remove("freshness_ttl_ms");
        map.remove("freshness");
    }
    serde_json::to_string(&json!({
        "resource_ura": resource.resource_ura,
        "owner_agent": resource.owner_agent,
        "type": resource.kind.as_str(),
        "binding": resource.binding.as_str(),
        "hardware_id": resource.hardware_id,
        "display_name": resource.display_name,
        "metadata": metadata,
    }))
    .expect("stable remote target cache signature serializes")
}

fn prune_retired_local_device_owner_rows(file: &mut ResourcesFile, owner_agent: &str) {
    file.resources.retain(|resource| {
        resource.binding != ResourceBinding::LocalDevice || resource.owner_agent == owner_agent
    });
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
                ResourceType::Display | ResourceType::Application | ResourceType::Window
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
        let auto_prunable_screen_target =
            matches!(
                resource.kind,
                ResourceType::Display | ResourceType::Application | ResourceType::Window
            ) && (resource.metadata.get("auto_prune").and_then(Value::as_bool) == Some(true)
                || auto_bootstrap_screen_target);
        let owned_by_this_daemon =
            resource.owner_agent == owner_agent && resource_ura_belongs_to_realm(resource, realm);
        !auto_prunable_screen_target
            || !owned_by_this_daemon
            || live.contains(resource.hardware_id.as_str())
    });
}

fn apply_remote_target_refresh(
    file: &mut ResourcesFile,
    realm: &str,
    owner_agent: &str,
    discovered: DiscoveredResources,
    observed_at_ms: u64,
) -> anyhow::Result<RemoteTargetInventoryRefresh> {
    prune_retired_local_device_owner_rows(file, owner_agent);
    let live_targets = discovered
        .resources
        .into_iter()
        .filter(|resource| is_remote_target_kind(resource.kind))
        .map(|resource| annotate_live_remote_target(resource, observed_at_ms))
        .collect::<Vec<_>>();
    let live_hardware_ids = live_targets
        .iter()
        .map(|resource| resource.hardware_id.clone())
        .collect::<HashSet<_>>();
    let before_prune_count = file.resources.len();
    if discovered.screen_target_discovery.permits_stale_prune() {
        prune_stale_auto_screen_targets(file, realm, owner_agent, &live_targets);
    }
    let retired_count = before_prune_count.saturating_sub(file.resources.len());
    apply_discovered_resources_indexed(file, realm, owner_agent, live_targets)?;
    let resources = file
        .resources
        .iter()
        .filter(|resource| {
            resource.owner_agent == owner_agent
                && live_hardware_ids.contains(resource.hardware_id.as_str())
                && resource_ura_belongs_to_realm(resource, realm)
                && is_remote_target_kind(resource.kind)
        })
        .cloned()
        .collect();
    Ok(RemoteTargetInventoryRefresh {
        observed_at_ms,
        freshness_ttl_ms: REMOTE_TARGET_FRESHNESS_TTL_MS,
        resources,
        retired_count,
        screen_target_discovery_available: discovered.screen_target_discovery.permits_stale_prune(),
    })
}

fn is_remote_target_kind(kind: ResourceType) -> bool {
    matches!(
        kind,
        ResourceType::Display | ResourceType::Application | ResourceType::Window
    )
}

fn annotate_live_remote_target(
    mut resource: DiscoveredResource,
    observed_at_ms: u64,
) -> DiscoveredResource {
    let metadata = resource.metadata.as_object_mut();
    if let Some(metadata) = metadata {
        metadata.insert(
            "discovery_source".to_string(),
            Value::String("resource.refresh_remote_targets".to_string()),
        );
        metadata.insert("auto_prune".to_string(), Value::Bool(true));
        metadata.insert(
            "availability".to_string(),
            Value::String("available".to_string()),
        );
        metadata.insert(
            "observed_at_ms".to_string(),
            Value::Number(serde_json::Number::from(observed_at_ms)),
        );
        metadata.insert(
            "freshness_ttl_ms".to_string(),
            Value::Number(serde_json::Number::from(REMOTE_TARGET_FRESHNESS_TTL_MS)),
        );
        metadata.insert(
            "freshness".to_string(),
            live_remote_target_freshness(observed_at_ms),
        );
        metadata.insert("stale_reason".to_string(), Value::Null);
        metadata.insert(
            "inventory_source".to_string(),
            Value::String("daemon_resource_inventory".to_string()),
        );
    }
    resource
}

fn live_remote_target_freshness(observed_at_ms: u64) -> Value {
    json!({
        "observed_at_ms": observed_at_ms,
        "stale_after_ms": observed_at_ms.saturating_add(REMOTE_TARGET_FRESHNESS_TTL_MS),
        "source": "live_refresh",
    })
}

fn discover_remote_target_resources() -> DiscoveredResources {
    let mut discovered = DiscoveredResources::default();
    discovered.resources.extend(discover_displays());
    match discover_screen_targets() {
        Ok(targets) => {
            discovered.screen_target_discovery = ScreenTargetDiscoveryState::Scanned;
            discovered.resources.extend(targets);
        }
        Err(err) => {
            discovered.screen_target_discovery = ScreenTargetDiscoveryState::Unavailable;
            crate::op_event!(
                component = media_resource_bootstrap,
                kind = remote_target_refresh_failed,
                reason = err.to_string(),
            );
        }
    }
    discovered
}

fn unix_ms_now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn resource_ura_belongs_to_realm(
    resource: &crate::daemon::persistence::resources::ResourceEntry,
    realm: &str,
) -> bool {
    crate::core::ura::parse_ura(&resource.resource_ura)
        .map(|parsed| parsed.realm == realm)
        .unwrap_or(false)
}

fn apply_discovered_resource(
    file: &mut ResourcesFile,
    realm: &str,
    owner_agent: &str,
    mut resource: DiscoveredResource,
) -> anyhow::Result<()> {
    annotate_host_device_ura(owner_agent, &mut resource.metadata)?;
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
    )?;
    Ok(())
}

fn apply_discovered_resources_indexed(
    file: &mut ResourcesFile,
    realm: &str,
    owner_agent: &str,
    resources: Vec<DiscoveredResource>,
) -> anyhow::Result<()> {
    let mut upsert_inputs = Vec::with_capacity(resources.len());
    for mut resource in resources {
        annotate_host_device_ura(owner_agent, &mut resource.metadata)?;
        upsert_inputs.push((
            resource.kind,
            resource.hardware_id,
            resource.display_name,
            resource.metadata,
        ));
    }
    let specs =
        upsert_inputs.iter().map(
            |(kind, hardware_id, display_name, metadata)| ResourceUpsert {
                realm,
                owner_agent,
                kind: *kind,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name,
                metadata: metadata.clone(),
            },
        );
    resources::upsert_resources_indexed(file, specs)?;
    Ok(())
}

fn annotate_host_device_ura(owner_agent: &str, metadata: &mut Value) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(owner_agent)?;
    let (device_id, _system_agent_id) = parsed.device_agent_ids().ok_or_else(|| {
        anyhow::anyhow!(
            "media local-device resource owner_agent must be a device-sponsored SystemAgent URA, got {owner_agent}"
        )
    })?;
    let host_device_ura = crate::core::ura::device_ura(&parsed.realm, device_id);
    let metadata = metadata.as_object_mut().ok_or_else(|| {
        anyhow::anyhow!("media local-device resource metadata must be a JSON object")
    })?;
    metadata.insert(
        "host_device_ura".to_string(),
        Value::String(host_device_ura),
    );
    Ok(())
}

fn discover_default_resources() -> DiscoveredResources {
    let mut discovered = DiscoveredResources::default();
    if let Some(mic) = discover_default_mic() {
        discovered.resources.push(mic);
    }
    discovered.resources.extend(discover_displays());
    match discover_screen_targets() {
        Ok(targets) => {
            discovered.screen_target_discovery = ScreenTargetDiscoveryState::Scanned;
            discovered.resources.extend(targets);
        }
        Err(err) => {
            discovered.screen_target_discovery = ScreenTargetDiscoveryState::Unavailable;
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
        .filter_map(|(idx, monitor)| {
            let id = monitor.id().ok();
            let id_string = id.as_ref().map(ToString::to_string);
            let hardware_id = match display_hardware_id_from_monitor_id(id_string.as_deref()) {
                Some(hardware_id) => hardware_id,
                None => {
                    crate::op_event!(
                        component = media_resource_bootstrap,
                        kind = display_without_stable_identity_skipped,
                        monitor_index = idx,
                    );
                    return None;
                }
            };
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
            Some(DiscoveredResource {
                kind: ResourceType::Display,
                hardware_id,
                display_name: name,
                metadata: json!({
                    "backend": "xcap",
                    "monitor_index": idx,
                    "monitor_id": id,
                    "hardware_identity_source": "xcap_monitor_id",
                    "x": x,
                    "y": y,
                    "width": width,
                    "height": height,
                    "is_primary": is_primary,
                    "discovery_source": "auto_bootstrap",
                    "auto_prune": true,
                }),
            })
        })
        .collect()
}

fn display_hardware_id_from_monitor_id(monitor_id: Option<&str>) -> Option<String> {
    monitor_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("display:xcap:{value}"))
}

#[cfg(target_os = "macos")]
fn discover_screen_targets() -> anyhow::Result<Vec<DiscoveredResource>> {
    macos_screen_targets::discover()
}

#[cfg(not(target_os = "macos"))]
fn discover_screen_targets() -> anyhow::Result<Vec<DiscoveredResource>> {
    // Windows and Linux keep using xcap for now, but this call is intentionally
    // isolated so Win32 EnumWindows and Linux portal/window-manager discovery can
    // replace it without touching resource persistence or pruning semantics.
    discover_screen_targets_with_xcap()
}

#[cfg(not(target_os = "macos"))]
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
            None,
            None,
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
    display_id: Option<u32>,
    bundle_id: Option<String>,
    app_identity: Option<String>,
    window_ids: Vec<u32>,
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
        display_id: Option<u32>,
        bundle_id: Option<&str>,
    ) {
        self.window_count += 1;
        self.window_ids.push(window_id);
        self.window_ids.sort_unstable();
        self.window_ids.dedup();
        if self.display_id.is_none() {
            self.display_id = display_id;
        }
        if self.bundle_id.is_none() {
            self.bundle_id = bundle_id.map(ToOwned::to_owned);
        }
        if self.app_identity.is_none() {
            self.app_identity = bundle_id.map(ToOwned::to_owned);
        }
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

    fn window_set_epoch(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.display_id.hash(&mut hasher);
        self.bundle_id.hash(&mut hasher);
        self.primary_pid.hash(&mut hasher);
        self.window_ids.hash(&mut hasher);
        hasher.finish()
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
    use crate::daemon::persistence::resources::ResourceType;
    use objc2_app_kit::NSRunningApplication;

    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFIndex = isize;
    type CFNumberRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;
    type CGDirectDisplayID = u32;
    type CGError = i32;
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
        fn CGGetDisplaysWithRect(
            rect: CGRect,
            maxDisplays: u32,
            displays: *mut CGDirectDisplayID,
            matchingDisplayCount: *mut u32,
        ) -> CGError;
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
        let mut apps: BTreeMap<String, (String, AppAggregate)> = BTreeMap::new();
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
            let display_id = display_id_for_rect(rect);
            let bundle_id = pid.and_then(bundle_id_for_pid);
            let app_identity = bundle_id.as_deref();
            if let Some(app_key) = app_aggregate_key(display_id, pid, app_identity) {
                apps.entry(app_key)
                    .or_insert_with(|| (app_name.clone(), AppAggregate::default()))
                    .1
                    .record_window(
                        window_id,
                        pid,
                        title.as_deref(),
                        area,
                        false,
                        bounds,
                        display_id,
                        app_identity,
                    );
            }
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
                    "display_id": display_id,
                    "bundle_id": bundle_id,
                    "app_identity": app_identity,
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

        out.extend(apps.into_iter().map(|(_key, (app_name, app))| {
            let window_set_epoch = app.window_set_epoch();
            let display_id = app
                .display_id
                .expect("macOS application aggregate keys require display_id");
            let identity_suffix = app
                .bundle_id
                .as_deref()
                .map(|bundle_id| format!("bundle:{bundle_id}"))
                .or_else(|| app.primary_pid.map(|pid| format!("pid:{pid}")))
                .expect("macOS application aggregate keys require bundle_id or primary_pid");
            DiscoveredResource {
                kind: ResourceType::Application,
                hardware_id: format!("application:macos:cgwindow:{display_id}:{identity_suffix}"),
                display_name: format!("{app_name} on display {display_id}"),
                metadata: serde_json::json!({
                    "backend": "macos_core_graphics",
                    "capture_target": "application",
                    "discovery_source": "auto_bootstrap",
                    "discovery_scope": "display_scoped_windows",
                    "auto_prune": true,
                    "platform_backend": "core_graphics_cgwindowlist",
                    "display_scoped": true,
                    "display_id": display_id,
                    "app_name": app_name,
                    "bundle_id": app.bundle_id,
                    "app_identity": app.app_identity,
                    "window_count": app.window_count,
                    "resolved_window_ids": app.window_ids,
                    "window_set_epoch": window_set_epoch,
                    "target_identity_epoch": window_set_epoch,
                    "primary_window_id": app.primary_window_id,
                    "primary_pid": app.primary_pid,
                    "primary_title": app.primary_title,
                    "primary_x": app.primary_bounds.and_then(|bounds| bounds.x),
                    "primary_y": app.primary_bounds.and_then(|bounds| bounds.y),
                    "primary_width": app.primary_bounds.and_then(|bounds| bounds.width),
                    "primary_height": app.primary_bounds.and_then(|bounds| bounds.height),
                }),
            }
        }));
        Ok(out)
    }

    fn display_id_for_rect(rect: CGRect) -> Option<u32> {
        let mut displays = [0_u32; 8];
        let mut count = 0_u32;
        let error = unsafe {
            CGGetDisplaysWithRect(
                rect,
                displays.len() as u32,
                displays.as_mut_ptr(),
                &mut count,
            )
        };
        if error != 0 || count == 0 {
            return None;
        }
        displays
            .first()
            .copied()
            .filter(|display_id| *display_id != 0)
    }

    fn bundle_id_for_pid(pid: u32) -> Option<String> {
        let app =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)?;
        app.bundleIdentifier()
            .map(|bundle_id| bundle_id.to_string())
            .map(|bundle_id| bundle_id.trim().to_string())
            .filter(|bundle_id| !bundle_id.is_empty())
    }

    fn app_aggregate_key(
        display_id: Option<u32>,
        pid: Option<u32>,
        app_identity: Option<&str>,
    ) -> Option<String> {
        let display_id = display_id?;
        app_identity
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|identity| format!("{display_id}:bundle:{identity}"))
            .or_else(|| pid.map(|pid| format!("{display_id}:pid:{pid}")))
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
    use crate::daemon::persistence::resources::{self, filter_by_kinds};

    #[test]
    fn apply_discovered_resource_mints_stable_resource_ura() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id: "camera:nokhwa:index:0".into(),
                display_name: "Default camera".into(),
                metadata: json!({"camera_index": 0}),
            },
        )
        .expect("seed default camera");
        let first = file.resources[0].resource_ura.clone();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id: "camera:nokhwa:index:0".into(),
                display_name: "Renamed camera".into(),
                metadata: json!({"camera_index": 0}),
            },
        )
        .expect("update default camera");

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].resource_ura, first);
        assert_eq!(file.resources[0].display_name, "Renamed camera");
        assert_eq!(
            file.resources[0].owner_agent,
            "easynet:///r/acme/agent/device.node-1.media"
        );
        assert_eq!(
            file.resources[0]
                .metadata
                .get("host_device_ura")
                .and_then(Value::as_str),
            Some("easynet:///r/acme/device/node-1")
        );
    }

    #[test]
    fn display_hardware_identity_requires_non_empty_platform_monitor_id() {
        assert_eq!(
            display_hardware_id_from_monitor_id(Some("42")),
            Some("display:xcap:42".to_string())
        );
        assert_eq!(
            display_hardware_id_from_monitor_id(Some("  42  ")),
            Some("display:xcap:42".to_string())
        );
        assert_eq!(display_hardware_id_from_monitor_id(None), None);
        assert_eq!(display_hardware_id_from_monitor_id(Some("")), None);
        assert_eq!(display_hardware_id_from_monitor_id(Some("   ")), None);
    }

    #[test]
    fn display_hardware_identity_does_not_synthesize_index_size_name_fallback() {
        let fallback_shape = "display:xcap:0:1920x1080:Built-in Display";

        assert_ne!(
            display_hardware_id_from_monitor_id(None).as_deref(),
            Some(fallback_shape),
            "missing monitor id must not become a synthetic persisted hardware id"
        );
    }

    #[test]
    fn seed_default_device_resources_writes_queryable_resources_file() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let count =
            seed_default_device_resources("acme", "easynet:///r/acme/agent/device.node-1.media")
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
    fn prune_retired_local_device_owner_rows_removes_previous_device_projection() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.old-device.media",
            DiscoveredResource {
                kind: ResourceType::Mic,
                hardware_id: "mic:cpal:default".into(),
                display_name: "Old mic".into(),
                metadata: json!({"backend": "cpal"}),
            },
        )
        .expect("seed old device mic");
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.current-device.media",
            DiscoveredResource {
                kind: ResourceType::Camera,
                hardware_id: "camera:nokhwa:index:0".into(),
                display_name: "Current camera".into(),
                metadata: json!({"backend": "nokhwa"}),
            },
        )
        .expect("seed current device camera");

        prune_retired_local_device_owner_rows(
            &mut file,
            "easynet:///r/acme/agent/device.current-device.media",
        );

        assert_eq!(file.resources.len(), 1);
        assert_eq!(
            file.resources[0].owner_agent,
            "easynet:///r/acme/agent/device.current-device.media"
        );
        assert_eq!(file.resources[0].hardware_id, "camera:nokhwa:index:0");
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_live_window_and_removes_closed_one() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Live".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed live window");
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:20:200".into(),
                display_name: "Closed".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed closed window");
        prune_stale_auto_screen_targets(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
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
    fn prune_stale_auto_screen_targets_removes_stale_display_rows() {
        let mut file = ResourcesFile::default();
        for hardware_id in ["display:xcap:live", "display:xcap:stale"] {
            apply_discovered_resource(
                &mut file,
                "acme",
                "easynet:///r/acme/agent/device.node-1.media",
                DiscoveredResource {
                    kind: ResourceType::Display,
                    hardware_id: hardware_id.into(),
                    display_name: hardware_id.into(),
                    metadata: json!({
                        "backend": "xcap",
                        "discovery_source": "auto_bootstrap",
                        "auto_prune": true
                    }),
                },
            )
            .expect("seed display");
        }

        prune_stale_auto_screen_targets(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            &[DiscoveredResource {
                kind: ResourceType::Display,
                hardware_id: "display:xcap:live".into(),
                display_name: "display:xcap:live".into(),
                metadata: json!({"backend": "xcap"}),
            }],
        );

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "display:xcap:live");
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_unmarked_resources() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Operator-managed target".into(),
                metadata: json!({"backend": "xcap"}),
            },
        )
        .expect("seed operator-managed target");

        prune_stale_auto_screen_targets(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            &[],
        );

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "window:xcap:10:100");
    }

    #[test]
    fn prune_stale_auto_screen_targets_keeps_other_owner_rows() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Closed local".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed local window");
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-2.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:20:200".into(),
                display_name: "Other owner".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed other owner window");
        apply_discovered_resource(
            &mut file,
            "other",
            "easynet:///r/other/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Application,
                hardware_id: "application:xcap:com.other.App".into(),
                display_name: "Other realm".into(),
                metadata: json!({"backend": "xcap"}),
            },
        )
        .expect("seed other realm application");

        prune_stale_auto_screen_targets(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            &[],
        );

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
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Previously Seen".into(),
                metadata: json!({"backend": "xcap"}),
            },
        )
        .expect("seed previous window");
        let discovered = DiscoveredResources {
            resources: Vec::new(),
            screen_target_discovery: ScreenTargetDiscoveryState::Unavailable,
        };

        if discovered.screen_target_discovery.permits_stale_prune() {
            prune_stale_auto_screen_targets(
                &mut file,
                "acme",
                "easynet:///r/acme/agent/device.node-1.media",
                &discovered.resources,
            );
        }

        assert_eq!(file.resources.len(), 1);
        assert_eq!(file.resources[0].hardware_id, "window:xcap:10:100");
    }

    #[test]
    fn stable_remote_target_cache_signature_ignores_freshness_metadata() {
        let owner_agent = "easynet:///r/acme/agent/device.node-1.media";
        let mut first = ResourcesFile::default();
        apply_discovered_resource(
            &mut first,
            "acme",
            owner_agent,
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Window".into(),
                metadata: json!({
                    "backend": "xcap",
                    "observed_at_ms": 10,
                    "freshness_ttl_ms": 5_000,
                    "freshness": {
                        "observed_at_ms": 10,
                        "stale_after_ms": 5_010,
                        "source": "live_refresh",
                    },
                    "bounds": {"x": 1, "y": 2, "width": 300, "height": 200}
                }),
            },
        )
        .expect("seed first window");

        let mut second = first.clone();
        second.resources[0]
            .metadata
            .as_object_mut()
            .expect("metadata object")
            .insert("observed_at_ms".to_string(), json!(20));
        second.resources[0]
            .metadata
            .as_object_mut()
            .expect("metadata object")
            .insert("freshness_ttl_ms".to_string(), json!(10_000));
        second.resources[0]
            .metadata
            .as_object_mut()
            .expect("metadata object")
            .insert(
                "freshness".to_string(),
                json!({
                    "observed_at_ms": 20,
                    "stale_after_ms": 10_020,
                    "source": "live_refresh",
                }),
            );

        assert_eq!(
            stable_remote_target_cache_signature(&first, "acme", owner_agent),
            stable_remote_target_cache_signature(&second, "acme", owner_agent),
            "watch polling must not rewrite the cache solely for freshness metadata"
        );

        second.resources[0].display_name = "Window moved".to_string();
        assert_ne!(
            stable_remote_target_cache_signature(&first, "acme", owner_agent),
            stable_remote_target_cache_signature(&second, "acme", owner_agent),
            "stable inventory changes must still trigger cache persistence"
        );
    }

    #[test]
    fn successful_empty_screen_target_scan_prunes_stale_auto_targets() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:10:100".into(),
                display_name: "Previously Seen".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed previous window");
        let discovered = DiscoveredResources {
            resources: Vec::new(),
            screen_target_discovery: ScreenTargetDiscoveryState::Scanned,
        };

        if discovered.screen_target_discovery.permits_stale_prune() {
            prune_stale_auto_screen_targets(
                &mut file,
                "acme",
                "easynet:///r/acme/agent/device.node-1.media",
                &discovered.resources,
            );
        }

        assert!(
            file.resources.is_empty(),
            "authoritative empty scans should prune stale auto-bootstrap targets"
        );
    }

    #[test]
    fn remote_target_refresh_annotates_live_projection_and_prunes_stale_targets() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:stale".into(),
                display_name: "Closed".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed stale window");

        let refresh = apply_remote_target_refresh(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResources {
                resources: vec![DiscoveredResource {
                    kind: ResourceType::Window,
                    hardware_id: "window:xcap:live".into(),
                    display_name: "Live".into(),
                    metadata: json!({
                        "backend": "xcap",
                        "capture_target": "window",
                        "window_id": 10,
                        "pid": 20,
                    }),
                }],
                screen_target_discovery: ScreenTargetDiscoveryState::Scanned,
            },
            123_456,
        )
        .expect("apply refresh");

        assert_eq!(refresh.retired_count, 1);
        assert_eq!(refresh.resources.len(), 1);
        let live = &refresh.resources[0];
        assert_eq!(live.hardware_id, "window:xcap:live");
        assert_eq!(
            live.owner_agent,
            "easynet:///r/acme/agent/device.node-1.media"
        );
        assert_eq!(
            live.metadata.get("host_device_ura").and_then(Value::as_str),
            Some("easynet:///r/acme/device/node-1")
        );
        assert_eq!(
            live.metadata.get("availability").and_then(Value::as_str),
            Some("available")
        );
        assert_eq!(
            live.metadata.get("observed_at_ms").and_then(Value::as_u64),
            Some(123_456)
        );
        assert_eq!(
            live.metadata
                .get("freshness_ttl_ms")
                .and_then(Value::as_u64),
            Some(REMOTE_TARGET_FRESHNESS_TTL_MS)
        );
        assert_eq!(
            live.metadata
                .pointer("/freshness/source")
                .and_then(Value::as_str),
            Some("live_refresh")
        );
        assert_eq!(
            live.metadata
                .pointer("/freshness/observed_at_ms")
                .and_then(Value::as_u64),
            Some(123_456)
        );
        assert_eq!(
            live.metadata
                .pointer("/freshness/stale_after_ms")
                .and_then(Value::as_u64),
            Some(123_456 + REMOTE_TARGET_FRESHNESS_TTL_MS)
        );
        assert!(file
            .resources
            .iter()
            .all(|resource| resource.hardware_id != "window:xcap:stale"));
    }

    #[test]
    fn unavailable_screen_target_refresh_excludes_stale_rows_without_pruning_cache() {
        let mut file = ResourcesFile::default();
        apply_discovered_resource(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResource {
                kind: ResourceType::Window,
                hardware_id: "window:xcap:previous".into(),
                display_name: "Previous".into(),
                metadata: json!({"backend": "xcap", "auto_prune": true}),
            },
        )
        .expect("seed previous window");

        let refresh = apply_remote_target_refresh(
            &mut file,
            "acme",
            "easynet:///r/acme/agent/device.node-1.media",
            DiscoveredResources {
                resources: Vec::new(),
                screen_target_discovery: ScreenTargetDiscoveryState::Unavailable,
            },
            123_456,
        )
        .expect("apply refresh");

        assert_eq!(refresh.resources.len(), 0);
        assert_eq!(refresh.retired_count, 0);
        assert_eq!(
            file.resources[0].hardware_id, "window:xcap:previous",
            "failed discovery must not delete cached rows; the live projection excludes them"
        );
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
            Some(42),
            Some("com.example.app"),
        );
        aggregate.record_window(
            11,
            Some(100),
            Some("Focused"),
            9_000,
            true,
            ScreenTargetBounds::new(Some(300), Some(400), Some(90), Some(100)),
            Some(42),
            Some("com.example.app"),
        );

        assert_eq!(aggregate.primary_window_id, Some(11));
        assert_eq!(aggregate.display_id, Some(42));
        assert_eq!(aggregate.bundle_id.as_deref(), Some("com.example.app"));
        assert_eq!(aggregate.app_identity.as_deref(), Some("com.example.app"));
        assert_eq!(aggregate.window_ids, vec![10, 11]);
        assert_ne!(aggregate.window_set_epoch(), 0);
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
