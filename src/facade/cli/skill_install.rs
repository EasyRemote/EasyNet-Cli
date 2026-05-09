// EasyNet CLI — Skill Install
// ============================
//
// File: src/cli/skill_install.rs
// Description: `easynet skill-install` — install EasyNet skill templates into
//              Claude Code's ~/.claude/skills/ directory.
//
// Skills are bundled in the `skills/` directory of the EasyNet-Cli repo.
// This command copies them to the right location so Claude Code / Codex
// can discover and use them natively.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use console::style;

use crate::persistence::config;
use crate::support::output;

#[derive(Debug, Args)]
pub struct SkillInstallArgs {
    /// Skill name to install (or "all" for all bundled skills)
    #[arg(default_value = "all")]
    pub name: String,

    /// Install target: claude, codex, or both (default: both)
    #[arg(long, default_value = "both")]
    pub client: String,

    /// Override target directory
    #[arg(long)]
    pub target: Option<String>,

    /// List available bundled skills without installing
    #[arg(long)]
    pub list: bool,

    /// Overwrite existing skills
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: SkillInstallArgs) -> anyhow::Result<()> {
    let skills_src = find_bundled_skills_dir()?;

    if args.list {
        return list_skills(&skills_src);
    }

    let targets = resolve_targets(&args)?;

    for (label, target_dir) in &targets {
        eprintln!("{}", style(format!("Installing for {label}:")).bold());
        install_to(skills_src.clone(), target_dir, &args)?;
        eprintln!();
    }

    Ok(())
}

fn resolve_targets(args: &SkillInstallArgs) -> anyhow::Result<Vec<(&'static str, PathBuf)>> {
    if let Some(t) = &args.target {
        return Ok(vec![("custom", PathBuf::from(t))]);
    }
    let home = config::home_dir();
    match args.client.as_str() {
        "claude" => Ok(vec![("Claude Code", home.join(".claude").join("skills"))]),
        "codex" => Ok(vec![("Codex", home.join(".agents").join("skills"))]),
        "both" => Ok(vec![
            ("Claude Code", home.join(".claude").join("skills")),
            ("Codex", home.join(".agents").join("skills")),
        ]),
        other => anyhow::bail!("unknown client: {other} (expected: claude, codex, both)"),
    }
}

fn install_to(
    skills_src: PathBuf,
    target_dir: &std::path::Path,
    args: &SkillInstallArgs,
) -> anyhow::Result<()> {
    let target_dir = target_dir.to_path_buf();

    let skills_to_install = if args.name == "all" {
        list_skill_names(&skills_src)?
    } else {
        let name = &args.name;
        let skill_path = skills_src.join(name);
        anyhow::ensure!(
            skill_path.join("SKILL.md").exists(),
            "bundled skill '{}' not found. Run 'easynet skill-install --list' to see available skills.",
            name
        );
        vec![name.clone()]
    };

    if skills_to_install.is_empty() {
        eprintln!("No bundled skills found.");
        return Ok(());
    }

    fs::create_dir_all(&target_dir)?;

    let mut installed = 0;
    let mut skipped = 0;

    for name in &skills_to_install {
        let src = skills_src.join(name);
        let dst = target_dir.join(name);

        if dst.exists() && !args.force {
            eprintln!(
                "  {} {} (already exists, use --force to overwrite)",
                style("skip").yellow(),
                name
            );
            skipped += 1;
            continue;
        }

        copy_dir_recursive(&src, &dst)?;
        eprintln!("  {} {}", style("✓").green(), name);
        installed += 1;
    }

    eprintln!();
    if installed > 0 {
        output::success(&format!(
            "Installed {} skill(s) into {}",
            installed,
            target_dir.display()
        ));
    }
    if skipped > 0 {
        eprintln!(
            "  {} {} skill(s) skipped (already installed)",
            style("⊘").dim(),
            skipped
        );
    }

    Ok(())
}

fn find_bundled_skills_dir() -> anyhow::Result<PathBuf> {
    // 1. Check relative to the binary (installed mode)
    if let Ok(exe) = std::env::current_exe() {
        // exe might be at: <install>/bin/easynet
        // skills at: <install>/skills/ or <install>/share/easynet/skills/
        if let Some(parent) = exe.parent() {
            let candidates = [
                parent.join("../skills"),
                parent.join("../share/easynet/skills"),
            ];
            for c in &candidates {
                if c.is_dir() && has_any_skill(c) {
                    return Ok(c.canonicalize()?);
                }
            }
        }
    }

    // 2. Check relative to current working directory (dev mode)
    let cwd = std::env::current_dir()?;
    let candidates = [cwd.join("skills"), cwd.join("../EasyNet-Cli/skills")];
    for c in &candidates {
        if c.is_dir() && has_any_skill(c) {
            return Ok(c.canonicalize()?);
        }
    }

    // 3. Walk up from cwd looking for Cargo.toml + skills/
    let mut dir = cwd.as_path();
    for _ in 0..8 {
        let skills = dir.join("skills");
        if skills.is_dir() && has_any_skill(&skills) && dir.join("Cargo.toml").exists() {
            return Ok(skills.canonicalize()?);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }

    anyhow::bail!(
        "Cannot find bundled skills directory. \
         Run from the EasyNet-Cli repo root, or install skills alongside the binary."
    )
}

fn has_any_skill(dir: &Path) -> bool {
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.path().join("SKILL.md").exists())
        })
        .unwrap_or(false)
}

fn list_skill_names(dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("SKILL.md").exists() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn list_skills(dir: &Path) -> anyhow::Result<()> {
    let names = list_skill_names(dir)?;
    if names.is_empty() {
        eprintln!("No bundled skills found.");
        return Ok(());
    }

    eprintln!("{}", style("Bundled EasyNet skills:").bold());
    for name in &names {
        let skill_md = dir.join(name).join("SKILL.md");
        let desc = read_skill_description(&skill_md);
        eprintln!("  {} — {}", style(name).cyan(), desc);
    }
    eprintln!();
    eprintln!("Install with: easynet skill-install [name|all]");
    Ok(())
}

fn read_skill_description(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| {
            // Parse YAML frontmatter description field
            content
                .lines()
                .find(|l| l.starts_with("description:"))
                .map(|l| l.trim_start_matches("description:").trim().to_string())
        })
        .unwrap_or_else(|| "(no description)".to_string())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
