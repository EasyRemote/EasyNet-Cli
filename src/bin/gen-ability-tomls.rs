// EasyNet CLI — ability TOML regenerator
// =======================================
//
// File: src/bin/gen-ability-tomls.rs
// Description: Regenerates every `abilities/system/<name>.ability.toml`
//              file from the live dispatcher metadata. Run this
//              after any change to ability description /
//              input_schema; the corresponding `cargo test` drift
//              guard fails CI if you forget.
//
// Usage
// -----
//   cargo run --bin gen-ability-tomls
//
// Behaviour:
//   1. Walk `published_abilities()` (every system ability the
//      dispatcher publishes — `<agent>.chat` is excluded by the
//      same filter the rest of the discovery surface applies).
//   2. For each, render the canonical TOML via
//      `runtime::agents::ability_toml::render_ability_toml`.
//   3. Write to `abilities/system/<name>.ability.toml`,
//      overwriting any prior content. Files NOT in the live
//      registry are deleted from `abilities/system/` so a
//      removed ability cleans up after itself.
//
// Why a separate binary (and not part of cargo build)
// ---------------------------------------------------
// build.rs runs *before* the crate's own modules compile, so it
// cannot call `runtime::agents::published_abilities()` (which
// depends on every ability module being compiled). Putting the
// generator in `src/bin/` lets it link the full crate and run
// after any code change with a single command.
//
// The drift test in `src/runtime/agents/mod.rs` is the safety
// net: if a contributor edits `description_for` and forgets to
// regenerate, `cargo test` fails with a message naming every
// drifted ability and telling them to run this binary.
//
// What this binary deliberately does NOT do
// -----------------------------------------
// * Delete handwritten TOMLs that aren't in `published_abilities()`.
//   `chat.ability.toml` is lazily seeded at runtime to the
//   per-install path (see runtime/abilities.rs); a future
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

use easynet_cli::runtime::agents::{ability_toml, published_abilities};

const TARGET_DIR: &str = "abilities/system";

fn main() -> anyhow::Result<()> {
    let target_dir = PathBuf::from(TARGET_DIR);
    if !target_dir.exists() {
        anyhow::bail!(
            "{TARGET_DIR} directory not found. \
             Run this binary from the crate root."
        );
    }

    let metas = published_abilities();
    let live_names: BTreeSet<String> =
        metas.iter().map(|m| m.name.clone()).collect();

    let mut written: Vec<String> = Vec::new();
    let mut unchanged: Vec<String> = Vec::new();
    for meta in &metas {
        let body = ability_toml::render_ability_toml(
            &meta.name,
            meta.description,
            &meta.input_schema,
        );
        let path = target_dir.join(format!("{}.ability.toml", meta.name));
        let prior = std::fs::read_to_string(&path).ok();
        if prior.as_deref() == Some(body.as_str()) {
            unchanged.push(meta.name.clone());
            continue;
        }
        std::fs::write(&path, body)?;
        written.push(meta.name.clone());
    }

    // Stale-file removal. Any `<name>.ability.toml` whose name
    // is NOT in `live_names` AND which round-trips to a name
    // matching our shape gets deleted. Files that don't match
    // the pattern (e.g. handwritten READMEs, future
    // non-system descriptors) are left alone.
    let mut deleted: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&target_dir)? {
        let entry = entry?;
        let path = entry.path();
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
                    deleted.push(stripped.to_string());
                }
            }
        }
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
    let _ = Path::new(TARGET_DIR); // silence unused-import warning if cfg ever excludes the body
    Ok(())
}
