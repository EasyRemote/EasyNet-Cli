use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const EXPECTED_DESCRIPTOR_COUNT: usize = 195;
const NAMED_SURFACES: &[&str] = &[
    "browser",
    "file_transfer",
    "media",
    "pages",
    "remote_desktop",
    "terminal",
    "voice",
];

#[derive(Debug)]
struct ExecutionContract {
    path: PathBuf,
    exposure: String,
    call_mode: String,
    dedicated_surface: String,
    subject_contract_kind: String,
    capability_state: String,
}

#[test]
fn every_descriptor_has_one_honest_execution_surface() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_descriptors(&root.join("ability-descriptors"), &mut files);
    collect_descriptors(&root.join("plugins"), &mut files);
    files.sort();

    let mut contracts = BTreeMap::new();
    for path in files {
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let value: toml::Value = toml::from_str(&body)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        let name = required_string(&value, "name", &path);
        let previous = contracts.insert(
            name.clone(),
            ExecutionContract {
                path: path.clone(),
                exposure: required_string(&value, "exposure", &path),
                call_mode: required_string(&value, "call_mode", &path),
                dedicated_surface: required_string(&value, "dedicated_surface", &path),
                subject_contract_kind: required_string(&value, "subject_contract_kind", &path),
                capability_state: required_string(&value, "capability_state", &path),
            },
        );
        assert!(
            previous.is_none(),
            "ability {name:?} has more than one descriptor: {} and {}",
            previous.unwrap().path.display(),
            path.display()
        );
    }

    assert_eq!(
        contracts.len(),
        EXPECTED_DESCRIPTOR_COUNT,
        "descriptor additions/removals must update the complete execution-surface matrix"
    );

    for (name, contract) in &contracts {
        let surface = contract.dedicated_surface.as_str();
        if surface == "none" {
            assert_ne!(
                contract.subject_contract_kind,
                "dedicated-surface",
                "{name} declares a dedicated subject without a named provider surface: {}",
                contract.path.display()
            );
        } else {
            assert!(
                NAMED_SURFACES.contains(&surface),
                "{name} declares unknown dedicated surface {surface:?}: {}",
                contract.path.display()
            );
            assert_eq!(
                contract.subject_contract_kind,
                "dedicated-surface",
                "{name} dedicated lifecycle must own subject materialization: {}",
                contract.path.display()
            );
        }

        if contract.call_mode == "bidi" && contract.exposure != "internal" {
            assert_ne!(
                surface,
                "none",
                "public bidi ability {name} has no lifecycle-aware frontend surface: {}",
                contract.path.display()
            );
        }
        assert!(
            matches!(
                contract.capability_state.as_str(),
                "cutover_ready" | "provider_backed" | "seam" | "unsupported"
            ),
            "{name} has unknown capability state {:?}: {}",
            contract.capability_state,
            contract.path.display()
        );
    }

    assert_contract(&contracts, "session.open", "internal", "none");
    for name in [
        "ability.deploy",
        "ability.publish",
        "ability.uninstall",
        "ability.unpublish",
        "admin.status",
        "fs.edit",
        "fs.list",
        "fs.read",
        "fs.stat",
        "fs.write",
        "http.request",
        "meta.describe",
        "meta.list_abilities",
        "meta.list_resources",
        "node.describe",
        "node.remove",
        "observe.health",
        "observe.network_health",
        "plugin.activate_realtime",
        "plugin.reload",
        "plugin.status",
        "process.exec",
        "resource.refresh_remote_targets",
        "resource.watch_remote_targets",
        "session.list",
        "shell.run",
    ] {
        assert_subject_contract_kind(&contracts, name, "authenticated-user");
    }
    for name in [
        "meta.list_resources",
        "resource.refresh_remote_targets",
        "resource.watch_remote_targets",
    ] {
        assert_contract(&contracts, name, "operator", "none");
    }
    for name in ["speaker.publish", "voice.subscribe", "voice.transcribe"] {
        assert_capability_state(&contracts, name, "unsupported");
    }
    for (name, contract) in &contracts {
        if name.starts_with("browser.") {
            assert_contract(&contracts, name, "operator", "browser");
        }
        if name.starts_with("remote_desktop.") {
            assert_contract(&contracts, name, "operator", "remote_desktop");
        }
        let _ = contract;
    }
}

fn assert_capability_state(
    contracts: &BTreeMap<String, ExecutionContract>,
    name: &str,
    capability_state: &str,
) {
    let contract = contracts
        .get(name)
        .unwrap_or_else(|| panic!("missing descriptor {name}"));
    assert_eq!(
        contract.capability_state, capability_state,
        "{name} capability state"
    );
}

fn assert_contract(
    contracts: &BTreeMap<String, ExecutionContract>,
    name: &str,
    exposure: &str,
    surface: &str,
) {
    let contract = contracts
        .get(name)
        .unwrap_or_else(|| panic!("missing descriptor {name}"));
    assert_eq!(contract.exposure, exposure, "{name} exposure");
    assert_eq!(contract.dedicated_surface, surface, "{name} surface");
}

fn assert_subject_contract_kind(
    contracts: &BTreeMap<String, ExecutionContract>,
    name: &str,
    subject_contract_kind: &str,
) {
    let contract = contracts
        .get(name)
        .unwrap_or_else(|| panic!("missing descriptor {name}"));
    assert_eq!(
        contract.subject_contract_kind, subject_contract_kind,
        "{name} subject_contract_kind"
    );
}

fn collect_descriptors(directory: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("read directory {}: {error}", directory.display()),
    };
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "read directory entry under {}: {error}",
                directory.display()
            )
        });
        let path = entry.path();
        if path.is_dir() {
            collect_descriptors(&path, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".ability.toml"))
        {
            out.push(path);
        }
    }
}

fn required_string(value: &toml::Value, field: &str, path: &Path) -> String {
    value
        .get(field)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("{} missing string field {field:?}", path.display()))
}
