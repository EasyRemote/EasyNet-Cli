// EasyNet CLI — ability TOML regenerator
// =======================================
//
// File: src/bin/gen-ability-tomls.rs
// Description: Regenerates daemon ability descriptors. Built-in daemon
//              descriptors come from the live system registry; plugin-owned
//              descriptors come from the plugin package index, independent of
//              this boot's runtime load plan.
//
// Usage
// -----
//   cargo run --bin gen-ability-tomls
//   cargo run --no-default-features --bin gen-ability-tomls
//
// Behaviour:
//   1. Walk `published_system_abilities()` for system abilities only
//      (`<agent>.chat` is excluded by the same filter the rest of
//      the discovery surface applies).
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
    ability_toml, descriptor_path_for, published_system_abilities, system_ability_descriptor_root,
    SYSTEM_ABILITY_DESCRIPTOR_ROOT,
};
use easynet_cli::daemon::plugins::{
    PluginDescriptorProjector, PluginPackageIndex, PluginWireRegistry,
};

fn main() -> anyhow::Result<()> {
    let target_dir = system_ability_descriptor_root();
    if !target_dir.exists() {
        anyhow::bail!(
            "{SYSTEM_ABILITY_DESCRIPTOR_ROOT} directory not found. \
             Run this binary from the crate root."
        );
    }

    let package_index = PluginPackageIndex::builtin()?;
    let plugin_wire = PluginWireRegistry::new(&package_index);

    let all_system_descriptors = published_system_abilities();
    let collisions: Vec<_> = all_system_descriptors
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

    let descriptors: Vec<_> = all_system_descriptors.into_iter().collect();
    let live_system_names: BTreeSet<String> = descriptors
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .collect();

    let mut written: Vec<String> = Vec::new();
    let mut unchanged: Vec<String> = Vec::new();
    for descriptor in &descriptors {
        let body = ability_toml::render_ability_toml(
            &descriptor.name,
            &descriptor.description,
            descriptor.input_schema(),
        );
        let path = PathBuf::from(descriptor_path_for(&descriptor.name));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let prior = std::fs::read_to_string(&path).ok();
        if prior.as_deref() == Some(body.as_str()) {
            unchanged.push(descriptor.name.clone());
            continue;
        }
        std::fs::write(&path, body)?;
        written.push(descriptor.name.clone());
    }

    let mut deleted: Vec<String> = Vec::new();
    delete_stale_descriptors(&target_dir, &live_system_names, &mut deleted)?;

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
            let body = ability_toml::render_ability_toml(
                &meta.name,
                &meta.description,
                &meta.input_schema,
            );
            let path = plugin_wire
                .ability_descriptor_path(&meta.name)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(descriptor_path_for(&meta.name)));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let prior = std::fs::read_to_string(&path).ok();
            if prior.as_deref() == Some(body.as_str()) {
                unchanged.push(meta.name.clone());
            } else {
                std::fs::write(&path, body)?;
                written.push(meta.name.clone());
            }
        }
        delete_stale_descriptors(
            Path::new(plugin.manifest().descriptor_dir()),
            &live_plugin_names,
            &mut deleted,
        )?;
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

fn delete_stale_descriptors(
    dir: &Path,
    live_names: &BTreeSet<String>,
    deleted: &mut Vec<String>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    // Stale-file removal. Any `<name>.ability.toml` whose name is NOT in
    // `live_names` AND whose body parses as TOML gets deleted. Files that
    // don't match the strict descriptor suffix are left alone.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            delete_stale_descriptors(&path, live_names, deleted)?;
            if std::fs::read_dir(&path)?.next().is_none() {
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
        if !live_names.contains(stripped) {
            // Confirm it parses as TOML before deleting, so a
            // human-edited unrelated file isn't silently nuked.
            if let Ok(body) = std::fs::read_to_string(&path) {
                if toml::from_str::<toml::Value>(&body).is_ok() {
                    std::fs::remove_file(&path)?;
                    deleted.push(path.display().to_string());
                }
            }
        }
    }
    Ok(())
}
