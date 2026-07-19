// EasyNet CLI — Plugin template generation
// ========================================
//
// File: src/cli/commands/groups/plugin_template.rs
// Description: Product-facing plugin project scaffolds for `easynet plugin init`.
//
// Boundary:
// - This module generates developer files only. It does not install packages,
//   allocate authority, publish abilities, or construct invocations.
// - Install/reload/bind-time collision checks remain owned by the daemon plugin
//   installer, index, and runtime binder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};

#[derive(Debug, Clone)]
pub struct PluginTemplateInit {
    pub path: PathBuf,
    pub package_id: Option<String>,
    pub ability_name: Option<String>,
    pub package_version: String,
    pub descriptor_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedPluginProject {
    pub path: PathBuf,
    pub package_id: String,
    pub package_version: String,
    pub ability_name: String,
    pub descriptor_version: String,
}

struct HelloPluginTemplate {
    target: PathBuf,
    package_id: String,
    package_version: String,
    ability_name: String,
    descriptor_version: String,
}

pub fn init_hello_plugin(init: PluginTemplateInit) -> anyhow::Result<GeneratedPluginProject> {
    let template = HelloPluginTemplate::from_init(init)?;
    template.write()
}

impl HelloPluginTemplate {
    fn from_init(init: PluginTemplateInit) -> anyhow::Result<Self> {
        let target = init.path;
        let slug = slug_from_path(&target);
        let package_id = init.package_id.unwrap_or_else(|| format!("local.{slug}"));
        let ability_name = init.ability_name.unwrap_or_else(|| format!("{slug}.echo"));
        validate_dotted_identifier(&package_id, "plugin package id")?;
        validate_dotted_identifier(&ability_name, "plugin ability name")?;
        validate_numeric_version(&init.package_version, "plugin package version")?;
        validate_numeric_version(&init.descriptor_version, "ability descriptor version")?;
        Ok(Self {
            target,
            package_id,
            package_version: init.package_version,
            ability_name,
            descriptor_version: init.descriptor_version,
        })
    }

    fn write(self) -> anyhow::Result<GeneratedPluginProject> {
        ensure_empty_or_missing_dir(&self.target)?;
        let abilities_dir = self.target.join("abilities");
        let bin_dir = self.target.join("bin");
        fs::create_dir_all(&abilities_dir)
            .with_context(|| format!("create {}", abilities_dir.display()))?;
        fs::create_dir_all(&bin_dir).with_context(|| format!("create {}", bin_dir.display()))?;

        write_new_file(&self.target.join("plugin.toml"), &self.plugin_toml())?;
        write_new_file(
            &abilities_dir.join(format!("{}.ability.toml", self.ability_name)),
            &self.ability_toml(),
        )?;
        let exec_path = bin_dir.join("exec-plugin");
        write_new_file(&exec_path, EXEC_PLUGIN)?;
        make_executable(&exec_path)?;
        write_new_file(&self.target.join("README.md"), &self.readme())?;

        Ok(GeneratedPluginProject {
            path: self.target,
            package_id: self.package_id,
            package_version: self.package_version,
            ability_name: self.ability_name,
            descriptor_version: self.descriptor_version,
        })
    }

    fn plugin_toml(&self) -> String {
        format!(
            r#"schema_version = "1"
id = "{package_id}"
version = "{package_version}"
kind = "declarative"
entrypoint = "declarative.exec"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 8

[declarative]
kind = "exec"
argv = ["bin/exec-plugin"]

[[ability_metadata]]
name = "{ability_name}"
layer = "control"
call_mode = "rpc"
"#,
            package_id = self.package_id,
            package_version = self.package_version,
            ability_name = self.ability_name,
        )
    }

    fn ability_toml(&self) -> String {
        format!(
            r#"schema_version = "2"
name = "{ability_name}"
descriptor_version = "{descriptor_version}"
description = "Echo one message from a Hello World plugin."
call_mode = "rpc"
capability_state = "provider_backed"
admission_action = "invoke"
visibility = "SCOPED"
scope_subjects_kind = "any"
scope_subjects_uras = []
scope_agents_kind = "any"
scope_agents_uras = []
denied_agents = []
output_receipt_schema_json = "{{}}"
hints_json = "{{\"read_only\":false,\"destructive\":false,\"idempotent\":false,\"streaming_only\":false,\"bidi_only\":false}}"
receipt_semantics = "operational"

[input_schema]
type = "object"
additionalProperties = false
required = ["message"]

[input_schema.properties.message]
type = "string"
"#,
            ability_name = self.ability_name,
            descriptor_version = self.descriptor_version,
        )
    }

    fn readme(&self) -> String {
        format!(
            r#"# {package_id}

Hello World EasyNet plugin generated by `easynet plugin init`.

## Versions

- Plugin package version: `{package_version}`
- Ability descriptor version: `{descriptor_version}`

The package version controls install/update/remove lifecycle. The descriptor
version is the governed callable interface version that enters descriptor refs,
authority bindings, implementation bindings, and receipts.

## Install

```bash
easynet plugin install .
```

If the daemon is running, install asks it to reload plugin state. If the daemon
is offline, the plugin loads on next daemon boot.

## Invoke

Discover the canonical ability URA:

```bash
easynet ability list --format json
```

Then call the descriptor-bound ability:

```bash
easynet ability stream '<descriptor-ref>' \
  --subject 'easynet:///r/local/resource/plugin/hello' \
  --causal-root \
  --args '{{"message":"hello from plugin"}}' \
  --format json --raw
```

## Collision model

This template uses package id `{package_id}` and ability `{ability_name}`.
Templates reduce accidental conflicts, but daemon install/reload/bind checks are
the authority: duplicate package versions, duplicate ability ownership, and
conflicting descriptor facts fail closed before publication.
"#,
            package_id = self.package_id,
            package_version = self.package_version,
            descriptor_version = self.descriptor_version,
            ability_name = self.ability_name,
        )
    }
}

const EXEC_PLUGIN: &str = r#"#!/usr/bin/env python3
import json
import sys

frame = json.loads(sys.stdin.readline())
invocation = frame.get("invocation") or {}
args = invocation.get("args") or {}

value = {
    "ok": True,
    "source": "hello-plugin",
    "message": args.get("message"),
    "caller": invocation.get("caller"),
    "callee": invocation.get("callee"),
    "subject": invocation.get("subject"),
    "ability": invocation.get("ability"),
    "invocation_nonce_len": len(invocation.get("invocation_nonce") or []),
}

print(json.dumps({
    "type": "result",
    "call_id": frame.get("call_id"),
    "value": value,
}, separators=(",", ":")), flush=True)
"#;

fn ensure_empty_or_missing_dir(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        return Ok(());
    }
    if !path.is_dir() {
        return Err(anyhow!(
            "plugin init target exists and is not a directory: {}",
            path.display()
        ));
    }
    if fs::read_dir(path)
        .with_context(|| format!("read {}", path.display()))?
        .next()
        .is_some()
    {
        return Err(anyhow!(
            "plugin init target directory is not empty: {}",
            path.display()
        ));
    }
    Ok(())
}

fn write_new_file(path: &Path, body: &str) -> anyhow::Result<()> {
    if path.exists() {
        return Err(anyhow!("refusing to overwrite {}", path.display()));
    }
    fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).with_context(|| format!("chmod +x {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn slug_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(slugify)
        .filter(|slug| !slug.is_empty())
        .unwrap_or_else(|| "hello_plugin".to_string())
}

fn slugify(raw: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_separator = false;
        } else if matches!(ch, '-' | '_' | '.' | ' ') && !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

fn validate_dotted_identifier(value: &str, field: &str) -> anyhow::Result<()> {
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(anyhow!(
            "{field} must use non-empty ASCII segments separated by dots, underscores, or dashes"
        ))
    }
}

fn validate_numeric_version(value: &str, field: &str) -> anyhow::Result<()> {
    let mut parts = value.split('.');
    let valid = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(patch), None) => [major, minor, patch]
            .into_iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit())),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(anyhow!("{field} must use N.N.N numeric form"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_hello_plugin_generates_installable_project() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("hello-plugin");

        let project = init_hello_plugin(PluginTemplateInit {
            path: target.clone(),
            package_id: None,
            ability_name: None,
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
        })
        .expect("generate plugin");

        assert_eq!(project.package_id, "local.hello_plugin");
        assert_eq!(project.ability_name, "hello_plugin.echo");
        assert!(target.join("plugin.toml").is_file());
        assert!(target
            .join("abilities/hello_plugin.echo.ability.toml")
            .is_file());
        assert!(target.join("bin/exec-plugin").is_file());
        assert!(target.join("README.md").is_file());

        let package = crate::daemon::plugins::package::PluginPackage::from_installed(&target, None)
            .expect("generated project parses as plugin package");
        assert_eq!(package.manifest().id(), "local.hello_plugin");
        assert_eq!(package.manifest().version(), "0.1.0");
        assert_eq!(
            package.manifest().abilities()[0].name(),
            "hello_plugin.echo"
        );
    }

    #[test]
    fn generated_plugin_install_rejects_duplicate_package_version() {
        let source_root = tempfile::tempdir().expect("source tempdir");
        let plugin_root = tempfile::tempdir().expect("plugin root");
        let source = source_root.path().join("hello-plugin");
        init_hello_plugin(PluginTemplateInit {
            path: source.clone(),
            package_id: Some("local.hello_plugin".to_string()),
            ability_name: Some("hello_plugin.echo".to_string()),
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
        })
        .expect("generate plugin");

        let installer = crate::daemon::plugins::PluginInstaller::new(plugin_root.path());
        installer.install(&source).expect("first install");
        let err = installer.install(&source).unwrap_err();

        assert!(
            format!("{err}").contains("already installed"),
            "duplicate install should fail closed, got {err}"
        );
    }

    #[test]
    fn init_hello_plugin_refuses_non_empty_target() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("hello-plugin");
        fs::create_dir_all(&target).expect("target dir");
        fs::write(target.join("existing.txt"), "keep").expect("existing file");

        let err = init_hello_plugin(PluginTemplateInit {
            path: target,
            package_id: None,
            ability_name: None,
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
        })
        .unwrap_err();

        assert!(format!("{err}").contains("not empty"));
    }

    #[test]
    fn init_hello_plugin_rejects_identifier_injection() {
        let root = tempfile::tempdir().expect("tempdir");

        let err = init_hello_plugin(PluginTemplateInit {
            path: root.path().join("hello-plugin"),
            package_id: Some("bad\"id".to_string()),
            ability_name: None,
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
        })
        .unwrap_err();

        assert!(format!("{err}").contains("plugin package id"));
    }
}
