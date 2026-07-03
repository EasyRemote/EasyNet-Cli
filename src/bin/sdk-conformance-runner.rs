// EasyNet CLI — SDK conformance manifest runner
// =============================================
//
// File: src/bin/sdk-conformance-runner.rs
// Description: Loads language-neutral SDK conformance cases, validates their
//              fixture/schema references, and emits machine-readable runner
//              records.
//
// Protocol Responsibility
// -----------------------
// Own the executable runner contract for the repository's SDK conformance
// manifest. This runner validates the shared case/fixture/schema graph that
// every language facade must consume. It does not claim that a language profile
// passed behavioral API execution; language adapters remain responsible for
// action execution over the same cases.
//
// Implementation Approach
// -----------------------
// Treat YAML cases as declarative manifests, parse JSON fixtures/schemas through
// serde_json, and emit one stable result record per case/language pair. This
// moves the conformance runner root from README-only scaffold to a CI-usable
// integrity gate without introducing a second daemon or Axon semantic path.
//
// Usage
// -----
//   cargo run --bin sdk-conformance-runner -- --language rust
//   cargo run --bin sdk-conformance-runner -- --language c_abi --format json
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(
    name = "sdk-conformance-runner",
    about = "Validate EasyNet SDK conformance case manifests and fixtures."
)]
struct Cli {
    /// Repository root containing sdk/conformance and sdk/schemas.
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Language target to report against, for example rust, c_abi, go, python.
    #[arg(long, default_value = "rust")]
    language: String,

    /// Output format for result records.
    #[arg(long, value_enum, default_value_t = OutputFormat::Jsonl)]
    format: OutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Jsonl,
    Json,
}

#[derive(Debug, Clone)]
struct ConformanceCase {
    id: String,
    profile: String,
    required_for: BTreeSet<String>,
    document: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ConformanceResultRecord {
    case_id: String,
    language: String,
    profile: String,
    status: ConformanceStatus,
    error_code: Option<&'static str>,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ConformanceStatus {
    Passed,
    Failed,
    Skipped,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let records = run_manifest(&cli.root, &cli.language)?;
    emit_records(&records, cli.format)?;

    if records
        .iter()
        .any(|record| record.status == ConformanceStatus::Failed)
    {
        std::process::exit(1);
    }
    Ok(())
}

fn run_manifest(root: &Path, language: &str) -> Result<Vec<ConformanceResultRecord>> {
    let cases = load_cases(root)?;
    let duplicate_errors = duplicate_case_errors(&cases);
    let mut records = Vec::with_capacity(cases.len());

    for case in cases {
        let mut errors = Vec::new();
        errors.extend(duplicate_errors.iter().filter_map(|(case_id, error)| {
            if case_id == &case.id {
                Some(error.clone())
            } else {
                None
            }
        }));
        errors.extend(validate_case_references(root, &case));

        let record = if !case.required_for.contains(language) {
            ConformanceResultRecord {
                case_id: case.id,
                language: language.to_string(),
                profile: case.profile,
                status: ConformanceStatus::Skipped,
                error_code: Some("PROFILE_UNDECLARED"),
                message: Some(format!("case is not declared for language `{language}`")),
            }
        } else if errors.is_empty() {
            ConformanceResultRecord {
                case_id: case.id,
                language: language.to_string(),
                profile: case.profile,
                status: ConformanceStatus::Passed,
                error_code: None,
                message: None,
            }
        } else {
            ConformanceResultRecord {
                case_id: case.id,
                language: language.to_string(),
                profile: case.profile,
                status: ConformanceStatus::Failed,
                error_code: Some("CONFORMANCE_MANIFEST_INVALID"),
                message: Some(errors.join("; ")),
            }
        };
        records.push(record);
    }

    Ok(records)
}

fn load_cases(root: &Path) -> Result<Vec<ConformanceCase>> {
    let case_dir = root.join("sdk/conformance/cases");
    let mut paths = fs::read_dir(&case_dir)
        .with_context(|| format!("read conformance case directory {}", case_dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("read conformance case entries {}", case_dir.display()))?;
    paths.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("yaml"));
    paths.sort();
    if paths.is_empty() {
        anyhow::bail!(
            "no conformance case YAML files found in {}",
            case_dir.display()
        );
    }

    paths
        .iter()
        .map(|path| load_case(path).with_context(|| format!("load case {}", path.display())))
        .collect()
}

fn load_case(path: &Path) -> Result<ConformanceCase> {
    let raw = fs::read_to_string(path)?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&raw)?;
    let document = serde_json::to_value(yaml)?;
    let object = document
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("case root must be an object"))?;

    let id = required_string(object.get("id"), "id")?;
    let profile = required_string(object.get("profile"), "profile")?;
    let required_for = required_string_set(object.get("required_for"), "required_for")?;

    if !object
        .get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| !steps.is_empty())
    {
        anyhow::bail!("steps must be a non-empty array");
    }
    if !object.get("expect").is_some_and(Value::is_object) {
        anyhow::bail!("expect must be an object");
    }
    validate_step_actions(object.get("steps"))?;

    Ok(ConformanceCase {
        id,
        profile,
        required_for,
        document,
    })
}

fn required_string(value: Option<&Value>, field: &'static str) -> Result<String> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    Ok(trimmed.to_string())
}

fn required_string_set(value: Option<&Value>, field: &'static str) -> Result<BTreeSet<String>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("{field} must be an array"))?;
    if values.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }

    let mut result = BTreeSet::new();
    for value in values {
        result.insert(required_string(Some(value), field)?);
    }
    Ok(result)
}

fn validate_step_actions(steps: Option<&Value>) -> Result<()> {
    let Some(steps) = steps.and_then(Value::as_array) else {
        anyhow::bail!("steps must be an array");
    };
    for (index, step) in steps.iter().enumerate() {
        let Some(step_object) = step.as_object() else {
            anyhow::bail!("steps[{index}] must be an object");
        };
        required_string(step_object.get("action"), "action")
            .with_context(|| format!("steps[{index}].action"))?;
    }
    Ok(())
}

fn duplicate_case_errors(cases: &[ConformanceCase]) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut duplicates = Vec::new();
    for case in cases {
        if !seen.insert(case.id.clone()) {
            duplicates.push((case.id.clone(), format!("duplicate case id `{}`", case.id)));
        }
    }
    duplicates
}

fn validate_case_references(root: &Path, case: &ConformanceCase) -> Vec<String> {
    let mut refs = CaseReferences::default();
    refs.collect(&case.document);

    let mut errors = Vec::new();
    for fixture in refs.fixtures {
        let path = root.join("sdk/conformance/fixtures").join(&fixture);
        if let Err(err) = validate_json_file(&path) {
            errors.push(format!("fixture `{fixture}`: {err}"));
        }
    }
    for schema in refs.schemas {
        let path = root.join("sdk/schemas").join(&schema);
        if let Err(err) = validate_schema_file(&path) {
            errors.push(format!("schema `{schema}`: {err}"));
        }
    }
    errors
}

#[derive(Debug, Default)]
struct CaseReferences {
    fixtures: BTreeSet<String>,
    schemas: BTreeSet<String>,
}

impl CaseReferences {
    fn collect(&mut self, value: &Value) {
        match value {
            Value::String(raw) if raw.ends_with(".schema.json") => {
                self.schemas.insert(raw.clone());
            }
            Value::String(raw) if raw.ends_with(".v4.json") => {
                self.fixtures.insert(raw.clone());
            }
            Value::Array(values) => {
                for value in values {
                    self.collect(value);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    self.collect(value);
                }
            }
            _ => {}
        }
    }
}

fn validate_json_file(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path).with_context(|| format!("missing {}", path.display()))?;
    serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("invalid json {}", path.display()))?;
    Ok(())
}

fn validate_schema_file(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path).with_context(|| format!("missing {}", path.display()))?;
    let json: Value =
        serde_json::from_str(&raw).with_context(|| format!("invalid json {}", path.display()))?;
    let object = json
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("schema root must be an object"))?;
    required_string(object.get("$schema"), "$schema")
        .with_context(|| format!("schema {} missing $schema", path.display()))?;
    required_string(object.get("title"), "title")
        .with_context(|| format!("schema {} missing title", path.display()))?;
    Ok(())
}

fn emit_records(records: &[ConformanceResultRecord], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Jsonl => {
            for record in records {
                println!("{}", serde_json::to_string(record)?);
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(records)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_accepts_repo_manifest_for_rust() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let records = run_manifest(root, "rust").expect("runner manifest");

        assert!(!records.is_empty());
        assert!(records
            .iter()
            .all(|record| record.status == ConformanceStatus::Passed));
    }

    #[test]
    fn runner_reports_missing_fixture_reference() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("sdk/conformance/cases")).unwrap();
        fs::create_dir_all(root.path().join("sdk/conformance/fixtures")).unwrap();
        fs::create_dir_all(root.path().join("sdk/schemas")).unwrap();
        fs::write(
            root.path()
                .join("sdk/conformance/cases/missing-fixture.yaml"),
            r#"
id: broken/missing_fixture
profile: runtime_core
required_for:
  - rust
steps:
  - action: load_fixture
    fixture: missing.v4.json
expect:
  result: error
"#,
        )
        .unwrap();

        let records = run_manifest(root.path(), "rust").expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].error_code, Some("CONFORMANCE_MANIFEST_INVALID"));
        assert!(records[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("missing.v4.json"));
    }
}
