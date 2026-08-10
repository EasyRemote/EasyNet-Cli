//! Chrome process and target lifecycle.
//! ====================================
//!
//! File: plugins/browser/src/chrome.rs
//! Description: Resolve current Chrome, launch an isolated process, and attach
//!              one flat CDP target session.
//!
//! Protocol Responsibility:
//! - None. This module manages a local browser resource behind plugin abilities.
//!
//! Implementation Approach:
//! - Select the newest installed Stable system or EasyNet-owned Chrome for Testing.
//! - Always pair remote debugging with a non-default user-data directory.
//! - Connect to the browser endpoint and create/attach one target in flat mode.
//!
//! Usage Contract:
//! - Existing endpoints must be loopback.
//! - Dropping an owned process lease terminates it; connected browsers are not
//!   represented by a process lease and are never terminated.
//!
//! Architectural Position:
//! - Browser plugin resource adapter.

use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use url::Url;

use super::cdp::CdpClient;
use super::constants::{ABILITY_OPEN_SESSION, CHROME_DISCOVERY_TIMEOUT_SECONDS};
use super::errors::{BrowserError, BrowserResult};

#[derive(Clone, Debug)]
pub struct ChromeOpenOptions {
    pub url: String,
    pub headless: bool,
    pub cdp_endpoint: Option<String>,
    pub executable_path: Option<String>,
    pub profile: Option<String>,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserVersion {
    pub product: String,
    pub protocol_version: String,
    pub user_agent: String,
    pub js_version: String,
}

pub struct OpenedChromeTarget {
    pub client: Arc<CdpClient>,
    pub process: Option<ChromeProcessLease>,
    pub target_id: String,
    pub cdp_session_id: String,
    pub version: BrowserVersion,
    pub profile_mode: String,
    pub browser_owned: bool,
}

pub struct ChromeProcessLease {
    child: Option<Child>,
    profile_dir: PathBuf,
    remove_profile_on_drop: bool,
}

impl ChromeProcessLease {
    pub fn terminate(mut self) {
        self.terminate_inner();
    }

    fn terminate_inner(&mut self) {
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        if self.remove_profile_on_drop {
            let _ = fs::remove_dir_all(&self.profile_dir);
        }
    }
}

impl Drop for ChromeProcessLease {
    fn drop(&mut self) {
        self.terminate_inner();
    }
}

pub async fn open_target(options: ChromeOpenOptions) -> BrowserResult<OpenedChromeTarget> {
    let (endpoint, process, profile_mode) = if let Some(endpoint) = &options.cdp_endpoint {
        if options.executable_path.is_some() || options.profile.is_some() {
            return Err(BrowserError::InvalidArgument {
                ability: ABILITY_OPEN_SESSION,
                detail: "`cdp_endpoint` cannot be combined with `executable_path` or `profile`"
                    .to_string(),
            });
        }
        let endpoint = resolve_existing_endpoint(endpoint).await?;
        (endpoint, None, "external".to_string())
    } else {
        let launch_options = options.clone();
        let launched = tokio::task::spawn_blocking(move || launch_chrome(&launch_options))
            .await
            .map_err(|error| BrowserError::Unavailable {
                ability: ABILITY_OPEN_SESSION,
                detail: format!("Chrome launch task failed: {error}"),
            })??;
        (
            launched.browser_endpoint,
            Some(launched.process),
            launched.profile_mode,
        )
    };

    let client = match CdpClient::connect(&endpoint).await {
        Ok(client) => client,
        Err(error) => {
            drop(process);
            return Err(BrowserError::Unavailable {
                ability: ABILITY_OPEN_SESSION,
                detail: error.to_string(),
            });
        }
    };

    let opened = initialize_target(Arc::clone(&client), &options.url).await;
    let (target_id, cdp_session_id, version) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            client.shutdown().await;
            drop(process);
            return Err(error);
        }
    };

    Ok(OpenedChromeTarget {
        client,
        process,
        target_id,
        cdp_session_id,
        version,
        browser_owned: options.cdp_endpoint.is_none(),
        profile_mode,
    })
}

async fn initialize_target(
    client: Arc<CdpClient>,
    url: &str,
) -> BrowserResult<(String, String, BrowserVersion)> {
    let version = client
        .send_command("Browser.getVersion", None, None)
        .await
        .map_err(|error| BrowserError::Unavailable {
            ability: ABILITY_OPEN_SESSION,
            detail: error.to_string(),
        })?;
    let version = BrowserVersion {
        product: value_string(&version, "product", "unknown"),
        protocol_version: value_string(&version, "protocolVersion", "unknown"),
        user_agent: value_string(&version, "userAgent", "unknown"),
        js_version: value_string(&version, "jsVersion", "unknown"),
    };

    let target = client
        .send_command(
            "Target.createTarget",
            Some(json!({"url": url, "background": false})),
            None,
        )
        .await
        .map_err(|error| BrowserError::Cdp {
            ability: ABILITY_OPEN_SESSION,
            detail: error.to_string(),
        })?;
    let target_id = required_result_string(&target, "targetId")?;
    let attached = client
        .send_command(
            "Target.attachToTarget",
            Some(json!({"targetId": target_id, "flatten": true})),
            None,
        )
        .await
        .map_err(|error| BrowserError::Cdp {
            ability: ABILITY_OPEN_SESSION,
            detail: error.to_string(),
        })?;
    let cdp_session_id = required_result_string(&attached, "sessionId")?;

    for method in ["Page.enable", "Runtime.enable", "DOM.enable"] {
        client
            .send_command(method, None, Some(&cdp_session_id))
            .await
            .map_err(|error| BrowserError::Cdp {
                ability: ABILITY_OPEN_SESSION,
                detail: error.to_string(),
            })?;
    }
    client
        .send_command("Page.bringToFront", None, Some(&cdp_session_id))
        .await
        .map_err(|error| BrowserError::Cdp {
            ability: ABILITY_OPEN_SESSION,
            detail: error.to_string(),
        })?;

    Ok((target_id, cdp_session_id, version))
}

fn required_result_string(value: &Value, key: &str) -> BrowserResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| BrowserError::Cdp {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("CDP response omitted {key:?}"),
        })
}

fn value_string(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

struct LaunchedChrome {
    browser_endpoint: String,
    process: ChromeProcessLease,
    profile_mode: String,
}

fn launch_chrome(options: &ChromeOpenOptions) -> BrowserResult<LaunchedChrome> {
    let executable = resolve_chrome_executable(options.executable_path.as_deref())?;
    let (profile_dir, remove_profile_on_drop, profile_mode) =
        profile_directory(options.profile.as_deref())?;
    fs::create_dir_all(&profile_dir).map_err(|error| BrowserError::Unavailable {
        ability: ABILITY_OPEN_SESSION,
        detail: format!("create browser profile {}: {error}", profile_dir.display()),
    })?;

    let mut command = Command::new(&executable);
    command
        .arg("--remote-debugging-port=0")
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-popup-blocking")
        .arg(format!(
            "--window-size={},{}",
            options.viewport_width, options.viewport_height
        ));
    if options.headless {
        command.arg("--headless=new").arg("--hide-scrollbars");
    }
    command
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command.spawn().map_err(|error| BrowserError::Unavailable {
        ability: ABILITY_OPEN_SESSION,
        detail: format!("launch {}: {error}", executable.display()),
    })?;
    let mut process = ChromeProcessLease {
        child: Some(child),
        profile_dir: profile_dir.clone(),
        remove_profile_on_drop,
    };
    match wait_for_active_port(&profile_dir) {
        Ok(browser_endpoint) => Ok(LaunchedChrome {
            browser_endpoint,
            process,
            profile_mode,
        }),
        Err(error) => {
            process.terminate_inner();
            Err(error)
        }
    }
}

fn wait_for_active_port(profile_dir: &Path) -> BrowserResult<String> {
    let active_port = profile_dir.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(CHROME_DISCOVERY_TIMEOUT_SECONDS);
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(&active_port) {
            let mut lines = contents.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                if port.parse::<u16>().is_ok() && path.starts_with('/') {
                    return Ok(format!("ws://127.0.0.1:{port}{path}"));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(BrowserError::Unavailable {
        ability: ABILITY_OPEN_SESSION,
        detail: format!(
            "Chrome did not publish {} within {} seconds",
            active_port.display(),
            CHROME_DISCOVERY_TIMEOUT_SECONDS
        ),
    })
}

async fn resolve_existing_endpoint(endpoint: &str) -> BrowserResult<String> {
    let endpoint = endpoint.trim().to_string();
    let parsed = Url::parse(&endpoint).map_err(|error| BrowserError::InvalidArgument {
        ability: ABILITY_OPEN_SESSION,
        detail: format!("invalid cdp_endpoint: {error}"),
    })?;
    require_loopback(&parsed)?;
    match parsed.scheme() {
        "ws" => Ok(endpoint),
        "http" => tokio::task::spawn_blocking(move || discover_browser_websocket(&endpoint))
            .await
            .map_err(|error| BrowserError::Unavailable {
                ability: ABILITY_OPEN_SESSION,
                detail: format!("CDP discovery task failed: {error}"),
            })?,
        scheme => Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("cdp_endpoint scheme {scheme:?} is unsupported; use http or ws"),
        }),
    }
}

fn discover_browser_websocket(endpoint: &str) -> BrowserResult<String> {
    let base = endpoint.trim_end_matches('/');
    let response = ureq::get(&format!("{base}/json/version"))
        .timeout(Duration::from_secs(3))
        .call()
        .map_err(|error| BrowserError::Unavailable {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("GET {base}/json/version: {error}"),
        })?;
    let body: Value = response
        .into_json()
        .map_err(|error| BrowserError::Unavailable {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("parse {base}/json/version: {error}"),
        })?;
    let websocket = body
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| BrowserError::Unavailable {
            ability: ABILITY_OPEN_SESSION,
            detail: "CDP discovery response omitted webSocketDebuggerUrl".to_string(),
        })?;
    let parsed = Url::parse(websocket).map_err(|error| BrowserError::Unavailable {
        ability: ABILITY_OPEN_SESSION,
        detail: format!("invalid discovered WebSocket URL: {error}"),
    })?;
    require_loopback(&parsed)?;
    Ok(websocket.to_string())
}

fn require_loopback(url: &Url) -> BrowserResult<()> {
    let host = url.host_str().unwrap_or_default();
    let allowed = host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "[::1]"
        || host == "::1";
    if allowed {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("cdp_endpoint host {host:?} is not loopback"),
        })
    }
}

fn profile_directory(profile: Option<&str>) -> BrowserResult<(PathBuf, bool, String)> {
    match profile {
        Some(profile) => {
            validate_profile_name(profile)?;
            let root = dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".easynet")
                .join("browser")
                .join("profiles");
            Ok((root.join(profile), false, format!("persistent:{profile}")))
        }
        None => {
            let path = std::env::temp_dir()
                .join(format!("easynet-browser-{}", uuid::Uuid::new_v4().simple()));
            Ok((path, true, "ephemeral".to_string()))
        }
    }
}

fn validate_profile_name(profile: &str) -> BrowserResult<()> {
    let valid = !profile.is_empty()
        && profile.len() <= 64
        && profile
            .chars()
            .enumerate()
            .all(|(index, ch)| ch.is_ascii_alphanumeric() || (index > 0 && "._-".contains(ch)));
    if valid {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument {
            ability: ABILITY_OPEN_SESSION,
            detail: "profile must match ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$".to_string(),
        })
    }
}

fn resolve_chrome_executable(explicit: Option<&str>) -> BrowserResult<PathBuf> {
    let configured = explicit
        .map(str::to_string)
        .or_else(|| std::env::var("EASYNET_BROWSER_CHROME").ok());
    if let Some(path) = configured {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(BrowserError::Unavailable {
            ability: ABILITY_OPEN_SESSION,
            detail: format!("Chrome executable {} does not exist", path.display()),
        });
    }
    let mut candidates = easynet_chrome_for_testing_candidates();
    candidates.extend(system_chrome_candidates());
    candidates.retain(|path| path.is_file());
    candidates.sort_by_cached_key(|path| Reverse(executable_version_key(path)));
    if let Some(path) = candidates.into_iter().next() {
        return Ok(path);
    }
    Err(BrowserError::Unavailable {
        ability: ABILITY_OPEN_SESSION,
        detail: "no Chrome/Chromium executable found; install current Chrome, place Chrome for Testing under ~/.easynet/browser/chrome/<version>, or set EASYNET_BROWSER_CHROME".to_string(),
    })
}

fn easynet_chrome_for_testing_candidates() -> Vec<PathBuf> {
    let root = std::env::var_os("EASYNET_BROWSER_CHROME_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            dirs::home_dir().map(|home| home.join(".easynet").join("browser").join("chrome"))
        });
    let Some(root) = root else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut directories = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().ok().is_some_and(|kind| kind.is_dir()))
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| Reverse(version_key(&entry.file_name().to_string_lossy())));
    directories
        .into_iter()
        .filter_map(|entry| chrome_for_testing_binary(&entry.path()))
        .collect()
}

fn version_key(name: &str) -> Vec<u64> {
    name.trim_start_matches("chrome-")
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn executable_version_key(path: &Path) -> Vec<u64> {
    Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| version_key(&String::from_utf8_lossy(&output.stdout)))
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| version_key(&path.to_string_lossy()))
}

fn chrome_for_testing_binary(directory: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let candidates = [
        directory.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        directory.join("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        directory.join("chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
    ];
    #[cfg(target_os = "linux")]
    let candidates = [
        directory.join("chrome"),
        directory.join("chrome-linux64/chrome"),
        directory.join("chrome-linux/chrome"),
    ];
    #[cfg(target_os = "windows")]
    let candidates = [
        directory.join("chrome.exe"),
        directory.join("chrome-win64/chrome.exe"),
        directory.join("chrome-win32/chrome.exe"),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let candidates: [PathBuf; 0] = [];
    candidates.into_iter().find(|path| path.is_file())
}

fn system_chrome_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(target_os = "linux")]
    {
        [
            "/usr/bin/google-chrome-stable",
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(target_os = "windows")]
    {
        let mut paths = Vec::new();
        for root in [
            std::env::var_os("PROGRAMFILES"),
            std::env::var_os("PROGRAMFILES(X86)"),
            std::env::var_os("LOCALAPPDATA"),
        ]
        .into_iter()
        .flatten()
        {
            paths.push(PathBuf::from(root).join("Google/Chrome/Application/chrome.exe"));
        }
        paths
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_profile_names_are_path_component_safe() {
        assert!(validate_profile_name("work-1.prod").is_ok());
        assert!(validate_profile_name("../Default").is_err());
        assert!(validate_profile_name("a/b").is_err());
    }

    #[test]
    fn semantic_version_key_orders_numeric_components() {
        assert!(version_key("chrome-151.0.7922.77") > version_key("chrome-99.0.1"));
    }

    #[test]
    fn automatic_candidates_exclude_unstable_chrome_channels() {
        assert!(system_chrome_candidates()
            .iter()
            .all(|path| !path.to_string_lossy().contains("Canary")));
    }

    #[test]
    fn remote_cdp_is_not_silently_admitted() {
        let remote = Url::parse("ws://example.com:9222/devtools/browser/x").unwrap();
        assert!(require_loopback(&remote).is_err());
        let local = Url::parse("ws://127.0.0.1:9222/devtools/browser/x").unwrap();
        assert!(require_loopback(&local).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires an installed Chrome/Chromium executable"]
    async fn current_chrome_real_cdp_smoke() {
        let opened = open_target(ChromeOpenOptions {
            url: "data:text/html,<title>EasyNet CDP smoke</title><input id='shared'>".to_string(),
            headless: true,
            cdp_endpoint: None,
            executable_path: None,
            profile: None,
            viewport_width: 800,
            viewport_height: 600,
        })
        .await
        .expect("launch current Chrome and attach a flat CDP target");

        assert_ne!(opened.version.product, "unknown");
        assert_ne!(opened.version.protocol_version, "unknown");
        println!(
            "browser_product={} cdp_protocol={}",
            opened.version.product, opened.version.protocol_version
        );
        let evaluated = opened
            .client
            .send_command(
                "Runtime.evaluate",
                Some(json!({
                    "expression": "document.title",
                    "returnByValue": true,
                })),
                Some(&opened.cdp_session_id),
            )
            .await
            .expect("evaluate through the attached target session");
        assert_eq!(
            evaluated["result"]["value"],
            Value::String("EasyNet CDP smoke".to_string())
        );

        opened
            .client
            .send_command(
                "Runtime.evaluate",
                Some(json!({"expression":"document.querySelector('#shared').focus()"})),
                Some(&opened.cdp_session_id),
            )
            .await
            .expect("focus the shared page input");
        opened
            .client
            .send_command(
                "Input.insertText",
                Some(json!({"text":"agent"})),
                Some(&opened.cdp_session_id),
            )
            .await
            .expect("insert agent text through CDP");
        let input_value = opened
            .client
            .send_command(
                "Runtime.evaluate",
                Some(json!({
                    "expression": "document.querySelector('#shared').value",
                    "returnByValue": true,
                })),
                Some(&opened.cdp_session_id),
            )
            .await
            .expect("read the shared page input");
        assert_eq!(input_value["result"]["value"], "agent");

        let mut events = opened.client.subscribe();
        opened
            .client
            .send_command(
                "Page.startScreencast",
                Some(json!({"format":"jpeg","quality":50,"maxWidth":800,"maxHeight":600})),
                Some(&opened.cdp_session_id),
            )
            .await
            .expect("start a real viewport screencast");
        let frame = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.expect("CDP event stream");
                if event.session_id.as_deref() == Some(opened.cdp_session_id.as_str())
                    && event.method == "Page.screencastFrame"
                {
                    break event;
                }
            }
        })
        .await
        .expect("receive a real viewport frame");
        assert!(
            frame
                .params
                .get("data")
                .and_then(Value::as_str)
                .is_some_and(|data| data.len() > 100),
            "viewport frame must contain real encoded pixels"
        );
        if let Some(frame_id) = frame.params.get("sessionId").and_then(Value::as_u64) {
            opened
                .client
                .send_command(
                    "Page.screencastFrameAck",
                    Some(json!({"sessionId":frame_id})),
                    Some(&opened.cdp_session_id),
                )
                .await
                .expect("acknowledge the viewport frame");
        }
        opened
            .client
            .send_command("Page.stopScreencast", None, Some(&opened.cdp_session_id))
            .await
            .expect("stop the viewport screencast");

        let _ = opened
            .client
            .send_command(
                "Target.closeTarget",
                Some(json!({"targetId": opened.target_id})),
                None,
            )
            .await;
        if opened.browser_owned {
            let _ = opened
                .client
                .send_command("Browser.close", None, None)
                .await;
        }
        opened.client.shutdown().await;
        if let Some(process) = opened.process {
            process.terminate();
        }
    }
}
