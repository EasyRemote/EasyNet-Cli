// EasyNet CLI
// ===========
//
// File: src/cli/ability_scaffold/mod.rs
// Description: `easynet ability new <name>` and `easynet ability validate <path>`.
//
// The scaffold emits a layout that is *simultaneously* a valid EasyNet
// ability, a valid MCP tool, and a valid Claude Code / Agent Skills skill.
// The CLI-owned scaffold schema is deliberately a superset of the common
// skill/tool manifests, so one directory is three things at once. Packaging
// policy remains in the product CLI rather than Axon's protocol SDK:
//
//   my-ability/
//   ├── ability.json        // manifest (required)
//   ├── SKILL.md            // Agent Skills frontmatter + docs
//   ├── scripts/
//   │   ├── invoke.sh       // unified entrypoint called by the runtime
//   │   └── handler.<ext>   // actual logic (language chosen via --lang)
//   └── README.md           // human-facing notes
//
// Validation re-reads the manifest and reports the common mistakes that
// trip deploy later (missing tool_name, invalid command, schema not an
// object, undeclared prerequisites referenced by the command, etc.).
//
// Module layout:
//   - this file          : CLI arg types + run_new/run_validate orchestration
//   - templates.rs       : string templates (manifest, SKILL.md, handler, etc.)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

mod templates;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use clap::{Args, ValueEnum};
use serde_json::Value;

use crate::support::platform::output;

#[derive(Debug, Clone, ValueEnum)]
pub enum AbilityLang {
    /// Shell script handler (portable, no runtime deps).
    Sh,
    /// Python 3 handler (stdin JSON → stdout JSON).
    Python,
    /// Rust handler source (compile to `target/{release,debug}/<name>`).
    Rust,
}

impl AbilityLang {
    pub(super) fn handler_filename(&self) -> &'static str {
        match self {
            AbilityLang::Sh => "handler.sh",
            AbilityLang::Python => "handler.py",
            AbilityLang::Rust => "handler.rs",
        }
    }
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Ability name (kebab-case recommended, e.g. "image-resize").
    pub name: String,
    /// Destination directory (default: current dir / <name>).
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Handler language.
    #[arg(long, value_enum, default_value_t = AbilityLang::Sh)]
    pub lang: AbilityLang,
    /// One-line description.
    #[arg(long, default_value = "Describe what this ability does.")]
    pub description: String,
    /// Overwrite if the target directory already exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Path to the ability directory (must contain ability.json).
    pub path: PathBuf,
    /// Return non-zero exit on warnings too (default: only errors fail).
    #[arg(long)]
    pub strict: bool,
}

pub fn run_new(args: NewArgs) -> anyhow::Result<()> {
    // Validate the ability name at the CLI boundary. This is the outer
    // layer of Bug #7's defense-in-depth: even though `render()` is now a
    // single-pass scanner that cannot be fooled by `{placeholder}` tokens
    // inside the name, we still reject such inputs up front because they
    // would produce confusing artifacts (e.g. a SKILL.md frontmatter with
    // `name: foo{desc}bar` is valid but jarring). The narrow allow-list
    // also protects against control characters that could break the
    // scaffolded filesystem layout.
    validate_ability_name(&args.name)?;

    let root: PathBuf = args
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from(&args.name));

    if root.exists() {
        if !args.force {
            bail!(
                "target already exists: {} (use --force to overwrite)",
                root.display()
            );
        }
    } else {
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    }
    fs::create_dir_all(root.join("scripts"))?;

    let ctx = templates::ScaffoldCtx {
        name: &args.name,
        description: &args.description,
        lang: &args.lang,
    };

    // ability.json — MCP tool spec + Agent Skills schema superset.
    let manifest = templates::ability_manifest(&ctx, &normalize_tool_name(&args.name));
    fs::write(
        root.join("ability.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;

    // SKILL.md — Agent Skills discovery entry point.
    fs::write(root.join("SKILL.md"), templates::skill_md(&ctx))?;

    // scripts/invoke.sh — universal entrypoint, never language-specific.
    let invoke_sh_path = root.join("scripts/invoke.sh");
    fs::write(&invoke_sh_path, templates::invoke_sh(&ctx))?;
    make_executable(&invoke_sh_path)?;

    // scripts/handler.<ext> — language-specific handler stub.
    let handler_path = root.join("scripts").join(args.lang.handler_filename());
    fs::write(&handler_path, templates::handler_source(&ctx))?;
    if !matches!(args.lang, AbilityLang::Rust) {
        make_executable(&handler_path)?;
    }

    fs::write(root.join("README.md"), templates::readme(&ctx))?;

    // Fail-fast: run the same structural checks `ability validate`
    // performs, so a broken scaffold is surfaced immediately instead
    // of waiting until deploy time. The rationale is pure ergonomics —
    // a user who just ran `ability new` and gets a success line should
    // be able to `ability deploy` without an intermediate validate
    // step. Any error here is a scaffolder bug, not user error, so
    // we print it so the operator can file a report.
    let report = validate_manifest_at(&root)?;
    if !report.errors.is_empty() {
        for e in &report.errors {
            output::error(e);
        }
        bail!(
            "scaffolded output failed self-validation ({} error(s)); \
             this is a bug in the template, please report",
            report.errors.len()
        );
    }

    output::success(&format!("scaffolded ability at {}", root.display()));
    if matches!(args.lang, AbilityLang::Rust) {
        output::step(
            "Rust mode: scripts/invoke.sh can auto-compile scripts/handler.rs via rustc; prebuild to target/release for deterministic deploys.",
        );
    }
    output::info(
        "next: 'easynet ability deploy <path> --node <node>' (or 'ability validate <path>' first)",
    );
    Ok(())
}

/// Outcome of a pure manifest check. Used by both `run_validate` (the
/// user-facing CLI verb that prints warnings / errors) and `run_new`
/// (which runs the same checks silently right after scaffolding so the
/// user cannot end up with a structurally-invalid skeleton that only
/// errors at deploy time).
struct ValidationReport {
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// Parse the manifest at `<dir>/ability.json` and run the structural
/// checks. Pure: no console output, no exit-on-error; the caller
/// decides how to surface the findings.
fn validate_manifest_at(dir: &std::path::Path) -> anyhow::Result<ValidationReport> {
    let manifest_path = dir.join("ability.json");
    if !manifest_path.exists() {
        bail!("ability.json not found at {}", manifest_path.display());
    }
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parse JSON: {}", manifest_path.display()))?;

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Required fields.
    for field in ["name", "description", "command"] {
        if manifest
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            errors.push(format!("missing or empty field: {field}"));
        }
    }

    // version: non-empty string if present; a default is applied at deploy time.
    match manifest.get("version") {
        Some(v) if !v.is_string() => errors.push("field 'version' must be a string".into()),
        None => warnings.push("field 'version' is absent; deploy will default to 1.0.0".into()),
        _ => {}
    }

    // tool_name: warn if missing, complain if divergent from normalized name.
    let name = manifest.get("name").and_then(Value::as_str).unwrap_or("");
    match manifest.get("tool_name").and_then(Value::as_str) {
        None | Some("") => warnings.push(format!(
            "tool_name absent — will default to '{}'",
            normalize_tool_name(name)
        )),
        Some(tn) if !is_valid_tool_name(tn) => errors.push(format!(
            "tool_name '{tn}' contains invalid characters (allowed: a-z 0-9 - _ .)"
        )),
        _ => {}
    }

    // Schemas must be objects (if present).
    for field in ["input_schema", "output_schema"] {
        if let Some(v) = manifest.get(field) {
            if !v.is_object() {
                errors.push(format!("field '{field}' must be a JSON object"));
            }
        }
    }

    // Command template sanity: if it references {ability_package_root}/...
    // files, check existence. `extract_referenced_files` only reports
    // paths under the ability package root, so joining against `dir` is
    // the correct way to resolve them without touching user $PWD.
    if let Some(cmd) = manifest.get("command").and_then(Value::as_str) {
        for referenced in extract_referenced_files(cmd) {
            let abs = dir.join(&referenced);
            if !abs.exists() {
                warnings.push(format!(
                    "command references '{}' but {} does not exist",
                    referenced,
                    abs.display()
                ));
            }
        }
    }

    Ok(ValidationReport { errors, warnings })
}

pub fn run_validate(args: ValidateArgs) -> anyhow::Result<()> {
    let report = validate_manifest_at(&args.path)?;

    for w in &report.warnings {
        output::warn(w);
    }
    for e in &report.errors {
        output::error(e);
    }

    if !report.errors.is_empty() {
        bail!(
            "{} error(s); fix them before 'ability deploy'",
            report.errors.len()
        );
    }
    if args.strict && !report.warnings.is_empty() {
        bail!(
            "{} warning(s) in strict mode; use without --strict to accept",
            report.warnings.len()
        );
    }
    output::success(&format!(
        "{} looks good ({} warning{})",
        args.path.display(),
        report.warnings.len(),
        if report.warnings.len() == 1 { "" } else { "s" }
    ));
    Ok(())
}

// ── helpers ─────────────────────────────────────────────────────────────

/// Validate an ability name supplied at `easynet ability new <name>`.
///
/// Allow-list rationale:
/// - Non-empty after trimming (empty names produce unusable artifacts).
/// - No `{` / `}`: avoids any possibility of confusing the template
///   renderer or producing artifacts whose frontmatter mentions
///   placeholder-looking tokens.
/// - No ASCII control characters (including newlines): these corrupt
///   YAML frontmatter in SKILL.md and make filesystem paths unusable.
/// - No path separators (`/`, `\`): the name is used as a directory
///   name when `--path` is omitted.
///
/// The allow-list is intentionally stricter than `normalize_tool_name`'s
/// alphabet. `normalize_tool_name` is a silent cleanup for generating the
/// `tool_name` manifest field; this validator is a loud guardrail at the
/// input boundary.
fn validate_ability_name(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        bail!("ability name must not be empty");
    }
    for c in name.chars() {
        match c {
            '{' | '}' => bail!(
                "ability name must not contain '{{' or '}}' (got '{name}'); \
                 these characters collide with template placeholders"
            ),
            '/' | '\\' => bail!("ability name must not contain path separators (got '{name}')"),
            c if c.is_control() => {
                bail!("ability name must not contain control characters (got '{name}')")
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_valid_tool_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.')
}

fn is_valid_tool_name(tn: &str) -> bool {
    tn.chars().all(is_valid_tool_name_char)
}

fn normalize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if is_valid_tool_name_char(c) {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Extracts path substrings of the form `{ability_package_root}/foo/bar`
/// from a command template. Stops at whitespace or shell metacharacters.
fn extract_referenced_files(cmd: &str) -> Vec<String> {
    const MARKER: &str = "{ability_package_root}/";
    let mut out = Vec::new();
    let mut rest = cmd;
    while let Some(idx) = rest.find(MARKER) {
        let after = &rest[idx + MARKER.len()..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '|' | ';'))
            .unwrap_or(after.len());
        let path = &after[..end];
        if !path.is_empty() {
            out.push(path.to_string());
        }
        rest = &after[end..];
    }
    out
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("easynet-scaffold-test-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        dir
    }

    // ── validate_ability_name ─────────────────────────────────────────

    #[test]
    fn validate_ability_name_accepts_kebab_and_snake_case() {
        assert!(validate_ability_name("image-resize").is_ok());
        assert!(validate_ability_name("vision.detect_v2").is_ok());
        assert!(validate_ability_name("OK_123").is_ok()); // uppercase is normalized later
    }

    #[test]
    fn validate_ability_name_rejects_empty_and_whitespace_only() {
        assert!(validate_ability_name("").is_err());
        assert!(validate_ability_name("   ").is_err());
    }

    #[test]
    fn validate_ability_name_rejects_template_placeholder_chars() {
        // Bug #7 outer defense: even though render() is now single-pass
        // and cannot be fooled, names with `{` / `}` still produce
        // confusing frontmatter. Reject at the boundary.
        let err = validate_ability_name("bad{desc}name")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains('{') || err.contains("placeholder"),
            "error must mention the forbidden character, got: {err}"
        );
        assert!(validate_ability_name("has}brace").is_err());
    }

    #[test]
    fn validate_ability_name_rejects_path_separators() {
        assert!(validate_ability_name("a/b").is_err());
        assert!(validate_ability_name("a\\b").is_err());
    }

    #[test]
    fn validate_ability_name_rejects_control_characters() {
        assert!(validate_ability_name("line1\nline2").is_err());
        assert!(validate_ability_name("tab\there").is_err());
        assert!(validate_ability_name("null\0char").is_err());
    }

    // ── normalize_tool_name ───────────────────────────────────────────

    #[test]
    fn normalize_tool_name_preserves_safe_characters() {
        assert_eq!(normalize_tool_name("image-resize"), "image-resize");
        assert_eq!(normalize_tool_name("vision.detect_v2"), "vision.detect_v2");
        assert_eq!(normalize_tool_name("ok_123"), "ok_123");
    }

    #[test]
    fn normalize_tool_name_lowercases_uppercase_ascii() {
        assert_eq!(normalize_tool_name("OK_123"), "ok_123");
    }

    #[test]
    fn normalize_tool_name_substitutes_unsafe_characters() {
        assert_eq!(normalize_tool_name("hello world"), "hello-world");
        assert_eq!(normalize_tool_name("ab/cd:ef"), "ab-cd-ef");
        // Per-codepoint mapping (not per-byte): two CJK chars → two dashes.
        assert_eq!(normalize_tool_name("中文"), "--");
    }

    #[test]
    fn extract_referenced_files_finds_simple_path() {
        let cmd = "python3 {ability_package_root}/scripts/run.py";
        assert_eq!(extract_referenced_files(cmd), vec!["scripts/run.py"]);
    }

    #[test]
    fn extract_referenced_files_finds_multiple_paths() {
        let cmd = "bash {ability_package_root}/scripts/a.sh && \
                   bash {ability_package_root}/scripts/b.sh";
        assert_eq!(
            extract_referenced_files(cmd),
            vec!["scripts/a.sh", "scripts/b.sh"]
        );
    }

    #[test]
    fn extract_referenced_files_stops_at_shell_metachars() {
        let cmd = "cat {ability_package_root}/data.json | jq .";
        assert_eq!(extract_referenced_files(cmd), vec!["data.json"]);
    }

    #[test]
    fn extract_referenced_files_returns_empty_when_no_marker() {
        let cmd = "echo hello";
        assert!(extract_referenced_files(cmd).is_empty());
    }

    #[test]
    fn run_validate_accepts_minimal_valid_manifest() {
        let dir = tmp_dir("valid");
        std::fs::write(
            dir.join("ability.json"),
            r#"{"name":"hi","description":"d","command":"echo {\"entries\":[]}"}"#,
        )
        .unwrap();
        let result = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: false,
        });
        assert!(
            result.is_ok(),
            "minimal manifest should validate: {result:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_validate_rejects_missing_required_fields() {
        let dir = tmp_dir("missing");
        std::fs::write(dir.join("ability.json"), r#"{"version":"1.0.0"}"#).unwrap();
        let err = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: false,
        })
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("error") || err.contains("missing"),
            "should fail with missing-field error, got: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_validate_rejects_invalid_tool_name_characters() {
        let dir = tmp_dir("bad-tool");
        std::fs::write(
            dir.join("ability.json"),
            r#"{"name":"hi","tool_name":"bad name!","description":"d","command":"echo"}"#,
        )
        .unwrap();
        let err = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: false,
        })
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("error"),
            "must reject illegal tool_name characters, got: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_validate_strict_mode_fails_on_warnings() {
        let dir = tmp_dir("strict");
        // No version field → produces a warning. Strict mode should escalate.
        std::fs::write(
            dir.join("ability.json"),
            r#"{"name":"hi","description":"d","command":"echo {\"entries\":[]}"}"#,
        )
        .unwrap();
        let lenient = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: false,
        });
        assert!(lenient.is_ok(), "lenient mode should accept warnings");
        let strict = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: true,
        });
        assert!(strict.is_err(), "strict mode should reject warnings");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_validate_errors_when_directory_lacks_ability_json() {
        let dir = tmp_dir("nofile");
        let err = run_validate(ValidateArgs {
            path: dir.clone(),
            strict: false,
        })
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("ability.json"),
            "should mention missing manifest, got: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
