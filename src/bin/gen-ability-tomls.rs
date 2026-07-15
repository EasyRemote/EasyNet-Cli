// EasyNet CLI — ability TOML regenerator
// =======================================
//
// File: src/bin/gen-ability-tomls.rs
// Description: Regenerates daemon ability descriptors. Built-in daemon
//              operational descriptors come from the live system registry;
//              Seam/Unsupported contracts come from capability-state
//              evidence. Plugin-owned descriptors come from the plugin package
//              index, independent of this boot's runtime load plan.
//
// Usage
// -----
//   cargo run --bin gen-ability-tomls
//   cargo run --no-default-features --bin gen-ability-tomls
//   cargo run --bin gen-ability-tomls -- --check
//
// Behaviour:
//   1. Walk `system_ability_contract_inventory()` for system contracts.
//      Operational rows originate from the live registry; non-operational
//      rows originate from capability-state evidence and never become live
//      handlers merely because their TOML is generated.
//   2. Walk the builtin `PluginPackageIndex` for repo-owned plugin abilities,
//      without consulting env flags, platform gates, the runtime load plan, or
//      user-local installed plugins. Builtin plugin bindings remain compile-
//      feature gated for specialist `--no-default-features` builds; ordinary
//      daemon/product builds include the remote desktop package by default.
//   3. For each, render the canonical TOML via
//      `daemon::ability::catalog::ability_toml::render_ability_toml`.
//   4. Write to the canonical descriptor path, overwriting any prior content.
//      Files no longer present in their owning system/package index are
//      deleted from that owner directory.
//
// Why a separate binary (and not part of cargo build)
// ---------------------------------------------------
// build.rs runs *before* the crate's own modules compile, so it
// cannot call `daemon::ability::catalog::published_system_abilities()` (which
// depends on every ability module being compiled). Putting the
// generator in `src/bin/` lets it link the full crate and run
// after any code change with a single command.
//
// The drift test in `src/daemon/ability/catalog/catalog_metadata.rs` is the safety
// net: if a contributor edits `description_for` and forgets to
// regenerate, `cargo test` fails with a message naming every
// drifted ability and telling them to run this binary.
//
// What this binary deliberately does NOT do
// -----------------------------------------
// * Delete handwritten TOMLs that aren't owned by the system registry or a
//   plugin package manifest.
//   `chat.ability.toml` is lazily seeded at runtime to the
//   per-install path (see runtime/agent_ability_specs.rs); a future
//   non-system descriptor (e.g. for a third-party ability shipped
//   alongside) might also live in this directory. We delete only
//   files matching the strict `<published_name>.ability.toml`
//   pattern, leaving anything else alone.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use easynet_cli::daemon::ability::catalog::{
    ability_toml, descriptor_path_for, system_ability_contract_inventory,
    system_ability_descriptor_root, SYSTEM_ABILITY_DESCRIPTOR_ROOT,
};
use easynet_cli::daemon::plugins::{
    PluginDescriptorProjector, PluginPackageIndex, PluginWireRegistry,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationMode {
    Write,
    Check,
}

fn main() -> anyhow::Result<()> {
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => GenerationMode::Write,
        [flag] if flag == "--check" => GenerationMode::Check,
        args => anyhow::bail!("usage: gen-ability-tomls [--check], got {args:?}"),
    };
    let target_dir = system_ability_descriptor_root();
    if !target_dir.exists() {
        anyhow::bail!(
            "{SYSTEM_ABILITY_DESCRIPTOR_ROOT} directory not found. \
             Run this binary from the crate root."
        );
    }

    let package_index = PluginPackageIndex::builtin()?;
    let plugin_wire = PluginWireRegistry::new(&package_index);

    let system_contracts = system_ability_contract_inventory();
    let collisions: Vec<_> = system_contracts
        .iter()
        .filter(|m| plugin_wire.ability_descriptor_path(&m.name).is_some())
        .map(|m| m.name.clone())
        .collect();
    if !collisions.is_empty() {
        anyhow::bail!(
            "plugin ability names collide with daemon system abilities: {:?}",
            collisions
        );
    }

    let contract_system_names: BTreeSet<String> = system_contracts
        .iter()
        .map(|contract| contract.name.clone())
        .collect();

    let mut written: Vec<String> = Vec::new();
    let mut unchanged: Vec<String> = Vec::new();
    for descriptor in &system_contracts {
        let body = ability_toml::render_ability_contract_toml(descriptor);
        let path = PathBuf::from(descriptor_path_for(&descriptor.name));
        if mode == GenerationMode::Write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let prior = std::fs::read_to_string(&path).ok();
        if prior.as_deref() == Some(body.as_str()) {
            unchanged.push(descriptor.name.clone());
            continue;
        }
        if mode == GenerationMode::Write {
            std::fs::write(&path, body)?;
        }
        written.push(descriptor.name.clone());
    }

    let mut deleted: Vec<String> = Vec::new();
    delete_stale_descriptors(&target_dir, &contract_system_names, mode, &mut deleted)?;

    let plugin_metas = PluginDescriptorProjector::project(&package_index)?;
    for plugin in package_index.packages() {
        let live_plugin_names: BTreeSet<String> = plugin
            .manifest()
            .abilities()
            .iter()
            .map(|ability| ability.name().to_string())
            .collect();
        for meta in plugin_metas
            .iter()
            .filter(|meta| live_plugin_names.contains(&meta.name))
        {
            let contract = plugin_contract(meta);
            let body = ability_toml::render_ability_contract_toml(&contract);
            let path = plugin_wire
                .ability_descriptor_path(&meta.name)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(descriptor_path_for(&meta.name)));
            if mode == GenerationMode::Write {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let prior = std::fs::read_to_string(&path).ok();
            if prior.as_deref() == Some(body.as_str()) {
                unchanged.push(meta.name.clone());
            } else {
                if mode == GenerationMode::Write {
                    std::fs::write(&path, body)?;
                }
                written.push(meta.name.clone());
            }
        }
        delete_stale_descriptors(
            Path::new(plugin.manifest().descriptor_dir()),
            &live_plugin_names,
            mode,
            &mut deleted,
        )?;
    }

    if mode == GenerationMode::Check && (!written.is_empty() || !deleted.is_empty()) {
        anyhow::bail!(
            "descriptor contract drift: would update {:?}; would delete {:?}",
            written,
            deleted
        );
    }

    println!(
        "gen-ability-tomls: wrote {} updated, {} unchanged, {} deleted",
        written.len(),
        unchanged.len(),
        deleted.len()
    );
    if !written.is_empty() {
        println!("  updated: {written:?}");
    }
    if !deleted.is_empty() {
        println!("  deleted: {deleted:?}");
    }
    Ok(())
}

fn plugin_contract(
    meta: &easynet_cli::daemon::plugins::PluginAbilityMetadata,
) -> easynet_cli::daemon::ability::catalog::SystemAbilityContract {
    use easynet_cli::daemon::ability::conformance::CapabilityState;
    use easynet_cli::daemon::ability::descriptors::{ReceiptSemantics, ScopeRule, Visibility};
    use easynet_cli::daemon::ability::CallMode;

    let call_mode = if meta.hints.bidi_only {
        CallMode::Bidi
    } else if meta.hints.streaming_only {
        CallMode::Stream
    } else {
        CallMode::Rpc
    };
    easynet_cli::daemon::ability::catalog::SystemAbilityContract {
        name: meta.name.clone(),
        descriptor_version: easynet_cli::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
            .to_string(),
        description: meta.description.clone(),
        input_schema: meta.input_schema.clone(),
        output_receipt_schema: meta
            .output_schema
            .clone()
            .unwrap_or_else(|| serde_json::json!({})),
        call_mode,
        admission_action: meta.admission_action,
        receipt_semantics: ReceiptSemantics::Operational,
        visibility: Visibility::Scoped,
        scope_subjects: ScopeRule::Any,
        scope_agents: ScopeRule::Any,
        denied_agents: Vec::new(),
        hints: meta.hints.clone(),
        capability_state: CapabilityState::ProviderBacked,
    }
}

fn delete_stale_descriptors(
    dir: &Path,
    contract_names: &BTreeSet<String>,
    mode: GenerationMode,
    deleted: &mut Vec<String>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    // Stale-file removal. Any `<name>.ability.toml` whose name is NOT in
    // `contract_names` AND whose body parses as TOML gets deleted. Files that
    // don't match the strict descriptor suffix are left alone.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            delete_stale_descriptors(&path, contract_names, mode, deleted)?;
            if mode == GenerationMode::Write && std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let stripped = match name.strip_suffix(".ability.toml") {
            Some(s) => s,
            None => continue,
        };
        if !contract_names.contains(stripped) {
            // Confirm it parses as TOML before deleting, so a
            // human-edited unrelated file isn't silently nuked.
            if let Ok(body) = std::fs::read_to_string(&path) {
                if toml::from_str::<toml::Value>(&body).is_ok() {
                    if mode == GenerationMode::Write {
                        std::fs::remove_file(&path)?;
                    }
                    deleted.push(path.display().to_string());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_mode_reports_stale_contract_without_deleting_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = dir.path().join("voice.stale.ability.toml");
        std::fs::write(&stale, "schema_version = \"1\"\nname = \"voice.stale\"\n")
            .expect("write stale descriptor");
        let mut deleted = Vec::new();

        delete_stale_descriptors(
            dir.path(),
            &BTreeSet::new(),
            GenerationMode::Check,
            &mut deleted,
        )
        .expect("dry-run stale scan");

        assert_eq!(deleted, vec![stale.display().to_string()]);
        assert!(stale.exists(), "check mode must never delete descriptors");
    }

    #[test]
    fn contract_inventory_name_is_not_considered_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let retained = dir.path().join("voice.join_call.ability.toml");
        std::fs::write(
            &retained,
            "schema_version = \"1\"\nname = \"voice.join_call\"\n",
        )
        .expect("write retained descriptor");
        let contract_names = BTreeSet::from(["voice.join_call".to_string()]);
        let mut deleted = Vec::new();

        delete_stale_descriptors(
            dir.path(),
            &contract_names,
            GenerationMode::Write,
            &mut deleted,
        )
        .expect("scan contract descriptor");

        assert!(deleted.is_empty());
        assert!(retained.exists(), "contract descriptor must be retained");
    }
}
