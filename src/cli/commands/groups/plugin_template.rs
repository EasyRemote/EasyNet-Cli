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
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginTemplateLanguage {
    Python,
    Go,
    Node,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSidecarHelperState {
    Unsupported,
    Seam,
    ProviderBacked,
    CutoverReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderSidecarCallMode {
    ExecInvoke,
    ExecStream,
    ExecBidi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSidecarHelperCapability {
    pub language: &'static str,
    pub call_mode: ProviderSidecarCallMode,
    pub state: ProviderSidecarHelperState,
    pub template_available: bool,
    pub helper_package: Option<&'static str>,
}

pub const PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX: &[ProviderSidecarHelperCapability] = &[
    ProviderSidecarHelperCapability {
        language: "python",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::CutoverReady,
        template_available: true,
        helper_package: Some("easynet_sdk.providers.easynet.plugin_exec"),
    },
    ProviderSidecarHelperCapability {
        language: "go",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::CutoverReady,
        template_available: true,
        helper_package: Some("easynet.run/cli/sdk/go/provider/easynet/pluginexec"),
    },
    ProviderSidecarHelperCapability {
        language: "rust",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "node",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::CutoverReady,
        template_available: true,
        helper_package: Some("@easynet/daemon-sdk/provider/easynet/pluginexec"),
    },
    ProviderSidecarHelperCapability {
        language: "java",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "c/c++",
        call_mode: ProviderSidecarCallMode::ExecInvoke,
        state: ProviderSidecarHelperState::Unsupported,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "python",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "go",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "rust",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "node",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "java",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "c/c++",
        call_mode: ProviderSidecarCallMode::ExecStream,
        state: ProviderSidecarHelperState::Unsupported,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "python",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "go",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "rust",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "node",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "java",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Seam,
        template_available: false,
        helper_package: None,
    },
    ProviderSidecarHelperCapability {
        language: "c/c++",
        call_mode: ProviderSidecarCallMode::ExecBidi,
        state: ProviderSidecarHelperState::Unsupported,
        template_available: false,
        helper_package: None,
    },
];

impl PluginTemplateLanguage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Go => "go",
            Self::Node => "node",
        }
    }

    pub fn sidecar_helper_capability(self) -> &'static ProviderSidecarHelperCapability {
        PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX
            .iter()
            .find(|capability| {
                capability.language == self.label()
                    && capability.call_mode == ProviderSidecarCallMode::ExecInvoke
            })
            .expect("plugin template language must have an exec-invoke provider helper matrix row")
    }
}

#[derive(Debug, Clone)]
pub struct PluginTemplateInit {
    pub path: PathBuf,
    pub package_id: Option<String>,
    pub ability_name: Option<String>,
    pub package_version: String,
    pub descriptor_version: String,
    pub language: PluginTemplateLanguage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedPluginProject {
    pub path: PathBuf,
    pub package_id: String,
    pub package_version: String,
    pub ability_name: String,
    pub descriptor_version: String,
    pub language: PluginTemplateLanguage,
}

struct HelloPluginTemplate {
    target: PathBuf,
    slug: String,
    package_id: String,
    package_version: String,
    ability_name: String,
    descriptor_version: String,
    language: PluginTemplateLanguage,
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
        let capability = init.language.sidecar_helper_capability();
        if !capability.template_available
            || !matches!(
                capability.state,
                ProviderSidecarHelperState::ProviderBacked
                    | ProviderSidecarHelperState::CutoverReady
            )
        {
            anyhow::bail!(
                "{} plugin templates require a provider-backed sidecar helper",
                init.language.label()
            );
        }
        validate_dotted_identifier(&package_id, "plugin package id")?;
        validate_dotted_identifier(&ability_name, "plugin ability name")?;
        validate_numeric_version(&init.package_version, "plugin package version")?;
        validate_numeric_version(&init.descriptor_version, "ability descriptor version")?;
        Ok(Self {
            target,
            slug,
            package_id,
            package_version: init.package_version,
            ability_name,
            descriptor_version: init.descriptor_version,
            language: init.language,
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
        self.write_language_files(&bin_dir)?;
        write_new_file(&self.target.join("README.md"), &self.readme())?;

        Ok(GeneratedPluginProject {
            path: self.target,
            package_id: self.package_id,
            package_version: self.package_version,
            ability_name: self.ability_name,
            descriptor_version: self.descriptor_version,
            language: self.language,
        })
    }

    fn write_language_files(&self, bin_dir: &Path) -> anyhow::Result<()> {
        match self.language {
            PluginTemplateLanguage::Python => {
                let exec_path = bin_dir.join("exec-plugin");
                write_new_file(&exec_path, PYTHON_EXEC_PLUGIN)?;
                make_executable(&exec_path)?;
            }
            PluginTemplateLanguage::Go => {
                let cmd_dir = self.target.join("cmd/exec-plugin");
                fs::create_dir_all(&cmd_dir)
                    .with_context(|| format!("create {}", cmd_dir.display()))?;
                write_new_file(&self.target.join("go.mod"), &self.go_mod())?;
                write_new_file(&self.target.join("Makefile"), GO_MAKEFILE)?;
                write_new_file(&cmd_dir.join("main.go"), GO_EXEC_PLUGIN)?;
                write_new_file(&bin_dir.join(".gitkeep"), "")?;
            }
            PluginTemplateLanguage::Node => {
                write_new_file(&self.target.join("package.json"), &self.node_package_json())?;
                let exec_path = bin_dir.join("exec-plugin");
                write_new_file(&exec_path, NODE_EXEC_WRAPPER)?;
                make_executable(&exec_path)?;
                write_new_file(&bin_dir.join("exec-plugin.mjs"), NODE_EXEC_PLUGIN)?;
            }
        }
        Ok(())
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
        let language_note = match self.language {
            PluginTemplateLanguage::Python => {
                r#"This template uses the Python CLI SDK provider helper:

```python
from easynet_sdk.providers.easynet.plugin_exec import SidecarInvocation, serve_exec_plugin
```

It is installable immediately when the runtime environment can import
`easynet_sdk`.
"#
            }
            PluginTemplateLanguage::Go => {
                r#"This template uses the Go CLI SDK provider helper:

```go
import "easynet.run/cli/sdk/go/provider/easynet/pluginexec"
```

Build the executable before install:

```bash
make build
easynet plugin install .
```

The daemon runs `bin/exec-plugin`; it does not run `go run` at invocation time.
"#
            }
            PluginTemplateLanguage::Node => {
                r#"This template uses the Node CLI SDK provider helper:

```js
import { serveExecPlugin } from "@easynet/daemon-sdk/provider/easynet/pluginexec.js";
```

Install dependencies before install:

```bash
npm install
easynet plugin install .
```

The daemon runs `bin/exec-plugin`, which executes the checked-in Node module.
"#
            }
        };
        format!(
            r#"# {package_id}

Hello World EasyNet plugin generated by `easynet plugin init`.

Language: `{language}`

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

## SDK sidecar helper

{language_note}

Do not hand-write daemon sidecar JSON frames in plugin code. The SDK helper owns
the `SidecarInvocation` projection and result/error frame emission.

## Invoke

Discover the canonical ability URA:

```bash
easynet ability list --format json
```

Then call the descriptor-bound ability:

```bash
easynet ability invoke '<descriptor-ref>' \
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
            language = self.language.label(),
            language_note = language_note,
        )
    }

    fn go_mod(&self) -> String {
        format!(
            r#"module example.com/easynet-plugin/{slug}

go 1.22

require easynet.run/cli/sdk/go v0.0.0

replace easynet.run/cli/sdk/go => {sdk_path}
"#,
            slug = self.slug,
            sdk_path = default_go_sdk_replace_path().display(),
        )
    }

    fn node_package_json(&self) -> String {
        format!(
            r#"{{
  "name": "{package_name}",
  "version": "{package_version}",
  "private": true,
  "type": "module",
  "dependencies": {{
    "@easynet/daemon-sdk": "file:{sdk_path}"
  }}
}}
"#,
            package_name = self.package_id.replace('.', "-"),
            package_version = self.package_version,
            sdk_path = default_node_sdk_file_path().display(),
        )
    }
}

const PYTHON_EXEC_PLUGIN: &str = r#"#!/usr/bin/env python3
from easynet_sdk.providers.easynet.plugin_exec import SidecarInvocation, serve_exec_plugin


def handle(invocation: SidecarInvocation) -> dict[str, object]:
    return {
        "ok": True,
        "source": "hello-plugin",
        "message": invocation.args.get("message"),
        "caller": invocation.caller,
        "callee": invocation.callee,
        "subject": invocation.subject,
        "ability": invocation.ability,
        "invocation_nonce_len": len(invocation.invocation_nonce),
    }


if __name__ == "__main__":
    serve_exec_plugin(handle)
"#;

const GO_EXEC_PLUGIN: &str = r#"package main

import (
	"context"

	"easynet.run/cli/sdk/go/provider/easynet/pluginexec"
)

func main() {
	pluginexec.MustServe(context.Background(), func(_ context.Context, invocation pluginexec.SidecarInvocation) (any, error) {
		return map[string]any{
			"ok":                   true,
			"source":               "hello-plugin",
			"message":              invocation.Args["message"],
			"caller":               invocation.Caller,
			"callee":               invocation.Callee,
			"subject":              invocation.Subject,
			"ability":              invocation.Ability,
			"invocation_nonce_len": len(invocation.InvocationNonce),
		}, nil
	})
}
"#;

const GO_MAKEFILE: &str = r#".PHONY: build

build:
	go build -o bin/exec-plugin ./cmd/exec-plugin
"#;

const NODE_EXEC_WRAPPER: &str = r#"#!/usr/bin/env sh
set -eu
exec node "$(dirname "$0")/exec-plugin.mjs"
"#;

const NODE_EXEC_PLUGIN: &str = r#"import { serveExecPlugin } from "@easynet/daemon-sdk/provider/easynet/pluginexec.js";

await serveExecPlugin((invocation) => ({
  ok: true,
  source: "hello-plugin",
  message: invocation.args.message,
  caller: invocation.caller,
  callee: invocation.callee,
  subject: invocation.subject,
  ability: invocation.ability,
  invocation_nonce_len: invocation.invocationNonce.length,
}));
"#;

fn default_go_sdk_replace_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdk/go")
}

fn default_node_sdk_file_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdk/node")
}

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
            language: PluginTemplateLanguage::Python,
        })
        .expect("generate plugin");

        assert_eq!(project.package_id, "local.hello_plugin");
        assert_eq!(project.ability_name, "hello_plugin.echo");
        assert_eq!(project.language, PluginTemplateLanguage::Python);
        assert!(target.join("plugin.toml").is_file());
        assert!(target
            .join("abilities/hello_plugin.echo.ability.toml")
            .is_file());
        assert!(target.join("bin/exec-plugin").is_file());
        assert!(!target.join("go.mod").exists());
        assert!(target.join("README.md").is_file());
        let exec_body = fs::read_to_string(target.join("bin/exec-plugin")).expect("exec body");
        assert!(exec_body.contains("serve_exec_plugin(handle)"));
        assert!(exec_body.contains("SidecarInvocation"));
        assert!(
            !exec_body.contains("json.loads"),
            "template must use the SDK plugin exec helper instead of hand-written frames"
        );

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
    fn sidecar_helper_matrix_keeps_templates_provider_backed() {
        let rows: std::collections::BTreeMap<_, _> = PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX
            .iter()
            .map(|row| ((row.language, row.call_mode), row))
            .collect();

        assert_eq!(rows.len(), PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX.len());
        for language in ["python", "go", "rust", "node", "java", "c/c++"] {
            for call_mode in [
                ProviderSidecarCallMode::ExecInvoke,
                ProviderSidecarCallMode::ExecStream,
                ProviderSidecarCallMode::ExecBidi,
            ] {
                assert!(
                    rows.contains_key(&(language, call_mode)),
                    "missing sidecar helper matrix row for {language}/{call_mode:?}"
                );
            }
        }
        for language in [
            PluginTemplateLanguage::Python,
            PluginTemplateLanguage::Go,
            PluginTemplateLanguage::Node,
        ] {
            let capability = language.sidecar_helper_capability();
            assert!(capability.template_available);
            assert!(matches!(
                capability.state,
                ProviderSidecarHelperState::ProviderBacked
                    | ProviderSidecarHelperState::CutoverReady
            ));
            assert!(
                capability.helper_package.is_some(),
                "{} template must point at a provider helper",
                language.label()
            );
        }
        for language in ["rust", "java", "c/c++"] {
            let capability = rows
                .get(&(language, ProviderSidecarCallMode::ExecInvoke))
                .unwrap_or_else(|| panic!("missing sidecar helper matrix row for {language}"));
            assert!(
                !capability.template_available,
                "{language} template must stay closed until its provider helper is backed"
            );
            assert!(matches!(
                capability.state,
                ProviderSidecarHelperState::Unsupported | ProviderSidecarHelperState::Seam
            ));
        }
        for language in ["python", "go", "rust", "node", "java", "c/c++"] {
            for call_mode in [
                ProviderSidecarCallMode::ExecStream,
                ProviderSidecarCallMode::ExecBidi,
            ] {
                let capability = rows[&(language, call_mode)];
                assert!(
                    !capability.template_available,
                    "{language}/{call_mode:?} template must stay closed until its provider helper owns streaming frames"
                );
                assert!(matches!(
                    capability.state,
                    ProviderSidecarHelperState::Unsupported | ProviderSidecarHelperState::Seam
                ));
                assert!(
                    capability.helper_package.is_none(),
                    "{language}/{call_mode:?} must not claim unary exec helper coverage"
                );
            }
        }
    }

    #[test]
    fn init_hello_plugin_generates_go_compiled_project() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("hello-go-plugin");

        let project = init_hello_plugin(PluginTemplateInit {
            path: target.clone(),
            package_id: Some("local.hello_go_plugin".to_string()),
            ability_name: Some("hello_go_plugin.echo".to_string()),
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
            language: PluginTemplateLanguage::Go,
        })
        .expect("generate go plugin");

        assert_eq!(project.language, PluginTemplateLanguage::Go);
        assert!(target.join("plugin.toml").is_file());
        assert!(target.join("go.mod").is_file());
        assert!(target.join("Makefile").is_file());
        assert!(target.join("cmd/exec-plugin/main.go").is_file());
        assert!(target.join("bin/.gitkeep").is_file());
        assert!(
            !target.join("bin/exec-plugin").exists(),
            "compiled template must not fake a binary before build"
        );
        let main_body = fs::read_to_string(target.join("cmd/exec-plugin/main.go")).expect("main");
        assert!(main_body.contains("pluginexec.MustServe"));
        assert!(!main_body.contains("json.NewDecoder"));
        let go_mod = fs::read_to_string(target.join("go.mod")).expect("go mod");
        assert!(go_mod.contains("replace easynet.run/cli/sdk/go => "));
        assert!(go_mod.contains("/sdk/go"));
        let readme = fs::read_to_string(target.join("README.md")).expect("readme");
        assert!(readme.contains("Build the executable before install"));
        assert!(readme.contains("make build"));
    }

    #[test]
    fn init_hello_plugin_generates_node_project() {
        let root = tempfile::tempdir().expect("tempdir");
        let target = root.path().join("hello-node-plugin");

        let project = init_hello_plugin(PluginTemplateInit {
            path: target.clone(),
            package_id: Some("local.hello_node_plugin".to_string()),
            ability_name: Some("hello_node_plugin.echo".to_string()),
            package_version: "0.1.0".to_string(),
            descriptor_version: "1.0.0".to_string(),
            language: PluginTemplateLanguage::Node,
        })
        .expect("generate node plugin");

        assert_eq!(project.language, PluginTemplateLanguage::Node);
        assert!(target.join("plugin.toml").is_file());
        assert!(target.join("package.json").is_file());
        assert!(target.join("bin/exec-plugin").is_file());
        assert!(target.join("bin/exec-plugin.mjs").is_file());
        let main_body = fs::read_to_string(target.join("bin/exec-plugin.mjs")).expect("node main");
        assert!(main_body.contains("serveExecPlugin"));
        assert!(!main_body.contains("JSON.parse"));
        let package_json = fs::read_to_string(target.join("package.json")).expect("package json");
        assert!(package_json.contains("\"@easynet/daemon-sdk\": \"file:"));
        assert!(package_json.contains("/sdk/node"));
        let readme = fs::read_to_string(target.join("README.md")).expect("readme");
        assert!(readme.contains("npm install"));
        assert!(readme.contains("@easynet/daemon-sdk/provider/easynet/pluginexec.js"));
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
            language: PluginTemplateLanguage::Python,
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
            language: PluginTemplateLanguage::Python,
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
            language: PluginTemplateLanguage::Python,
        })
        .unwrap_err();

        assert!(format!("{err}").contains("plugin package id"));
    }
}
