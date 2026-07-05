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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
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

    /// Optional language action-adapter report to validate against required cases.
    #[arg(long)]
    adapter_report: Option<PathBuf>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ConformanceStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Deserialize)]
struct AdapterReport {
    schema_version: u64,
    language: String,
    adapter_kind: String,
    records: Vec<AdapterResultRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct AdapterResultRecord {
    case_id: String,
    profile: String,
    status: ConformanceStatus,
    evidence: Vec<AdapterEvidence>,
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AdapterEvidence {
    kind: String,
    ref_path: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let records = run_manifest(&cli.root, &cli.language, cli.adapter_report.as_deref())?;
    emit_records(&records, cli.format)?;

    if records
        .iter()
        .any(|record| record.status == ConformanceStatus::Failed)
    {
        std::process::exit(1);
    }
    Ok(())
}

fn run_manifest(
    root: &Path,
    language: &str,
    adapter_report_path: Option<&Path>,
) -> Result<Vec<ConformanceResultRecord>> {
    let cases = load_cases(root)?;
    let adapter_report = adapter_report_path
        .map(|path| load_adapter_report(root, language, path))
        .transpose()?;
    let adapter_report = adapter_report.as_ref().map(AdapterReportIndex::new);
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
            match adapter_report.as_ref() {
                Some(report) => record_from_adapter(root, language, &case, report),
                None => ConformanceResultRecord {
                    case_id: case.id,
                    language: language.to_string(),
                    profile: case.profile,
                    status: ConformanceStatus::Passed,
                    error_code: None,
                    message: None,
                },
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

fn load_adapter_report(root: &Path, language: &str, path: &Path) -> Result<AdapterReport> {
    let path = resolve_repo_path(root, path);
    ensure_path_inside_root(root, &path).with_context(|| {
        format!(
            "adapter report {} must stay under repository root {}",
            path.display(),
            root.display()
        )
    })?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read adapter report {}", path.display()))?;
    let report: AdapterReport = serde_json::from_str(&raw)
        .with_context(|| format!("decode adapter report {}", path.display()))?;
    if report.schema_version != 1 {
        anyhow::bail!("adapter report schema_version must be 1");
    }
    if report.language != language {
        anyhow::bail!(
            "adapter report language `{}` does not match requested language `{}`",
            report.language,
            language
        );
    }
    if report.adapter_kind.trim().is_empty() {
        anyhow::bail!("adapter report adapter_kind must not be empty");
    }
    let mut seen = BTreeSet::new();
    for record in &report.records {
        if record.case_id.trim().is_empty() {
            anyhow::bail!("adapter record case_id must not be empty");
        }
        if record.profile.trim().is_empty() {
            anyhow::bail!("adapter record profile must not be empty");
        }
        if !seen.insert(record.case_id.clone()) {
            anyhow::bail!("duplicate adapter record case_id `{}`", record.case_id);
        }
        if record.evidence.is_empty() {
            anyhow::bail!("adapter record `{}` must include evidence", record.case_id);
        }
        for evidence in &record.evidence {
            validate_adapter_evidence(root, &record.case_id, evidence)?;
        }
    }
    Ok(report)
}

#[derive(Debug)]
struct AdapterReportIndex<'a> {
    records: BTreeMap<&'a str, &'a AdapterResultRecord>,
}

impl<'a> AdapterReportIndex<'a> {
    fn new(report: &'a AdapterReport) -> Self {
        Self {
            records: report
                .records
                .iter()
                .map(|record| (record.case_id.as_str(), record))
                .collect(),
        }
    }

    fn find(&self, case_id: &str) -> Option<&'a AdapterResultRecord> {
        self.records.get(case_id).copied()
    }
}

fn validate_adapter_evidence(root: &Path, case_id: &str, evidence: &AdapterEvidence) -> Result<()> {
    match evidence.kind.as_str() {
        "go_test" | "python_test" | "rust_test" | "c_abi_test" | "runner_test" => {}
        other => anyhow::bail!("adapter record `{case_id}` has unknown evidence kind `{other}`"),
    }
    let path = resolve_repo_path(root, Path::new(&evidence.ref_path));
    ensure_path_inside_root(root, &path)
        .with_context(|| format!("validate adapter evidence {}", path.display()))
}

fn resolve_repo_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn ensure_path_inside_root(root: &Path, path: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize root {}", root.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("missing {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        anyhow::bail!("path escapes repository root");
    }
    Ok(())
}

fn record_from_adapter(
    root: &Path,
    language: &str,
    case: &ConformanceCase,
    report: &AdapterReportIndex<'_>,
) -> ConformanceResultRecord {
    let Some(adapter) = report.find(&case.id) else {
        return ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            status: ConformanceStatus::Failed,
            error_code: Some("ACTION_ADAPTER_MISSING"),
            message: Some(format!(
                "adapter report missing required case `{}`",
                case.id
            )),
        };
    };
    let mut errors = Vec::new();
    if adapter.profile != case.profile {
        errors.push(format!(
            "adapter profile `{}` does not match case profile `{}`",
            adapter.profile, case.profile
        ));
    }
    if adapter.status != ConformanceStatus::Passed {
        errors.push(format!(
            "adapter status is {:?}: {}",
            adapter.status,
            adapter.message.clone().unwrap_or_default()
        ));
    }
    for evidence in &adapter.evidence {
        if let Err(err) = validate_adapter_evidence(root, &adapter.case_id, evidence) {
            errors.push(err.to_string());
        }
    }
    if errors.is_empty() {
        ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            status: ConformanceStatus::Passed,
            error_code: None,
            message: None,
        }
    } else {
        ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            status: ConformanceStatus::Failed,
            error_code: Some("ACTION_ADAPTER_FAILED"),
            message: Some(errors.join("; ")),
        }
    }
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
        let records = run_manifest(root, "rust", None).expect("runner manifest");

        assert!(!records.is_empty());
        assert!(records
            .iter()
            .all(|record| record.status != ConformanceStatus::Failed));
        assert!(records.iter().any(|record| {
            record.status == ConformanceStatus::Skipped
                && record.error_code == Some("PROFILE_UNDECLARED")
        }));
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

        let records = run_manifest(root.path(), "rust", None).expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].error_code, Some("CONFORMANCE_MANIFEST_INVALID"));
        assert!(records[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("missing.v4.json"));
    }

    #[test]
    fn runner_validates_language_adapter_report() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let report = root.join("sdk/conformance/runner/go-action-adapter-report.json");
        let records =
            run_manifest(root, "go", Some(&report)).expect("runner validates adapter report");

        let required: Vec<_> = records
            .iter()
            .filter(|record| record.language == "go" && record.status != ConformanceStatus::Skipped)
            .collect();
        assert!(!required.is_empty());
        assert!(required
            .iter()
            .all(|record| record.status == ConformanceStatus::Passed));
    }

    #[test]
    fn runner_reports_missing_required_adapter_record() {
        let root = tempfile::tempdir().expect("tempdir");
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("adapter.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 1,
  "language": "go",
  "adapter_kind": "unit_test",
  "records": []
}"#,
        )
        .unwrap();

        let records = run_manifest(root.path(), "go", Some(&report)).expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].error_code, Some("ACTION_ADAPTER_MISSING"));
    }

    #[test]
    fn runner_reports_failed_adapter_record() {
        let root = tempfile::tempdir().expect("tempdir");
        create_minimal_case_root(root.path(), "python");
        let report = root.path().join("adapter.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 1,
  "language": "python",
  "adapter_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "status": "failed",
      "evidence": [{"kind": "runner_test", "ref_path": "sdk/conformance/runner/README.md"}],
      "message": "forced failure"
    }
  ]
}"#,
        )
        .unwrap();

        let records = run_manifest(root.path(), "python", Some(&report)).expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].error_code, Some("ACTION_ADAPTER_FAILED"));
        assert!(records[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("forced failure"));
    }

    fn create_minimal_case_root(root: &Path, language: &str) {
        fs::create_dir_all(root.join("sdk/conformance/cases")).unwrap();
        fs::create_dir_all(root.join("sdk/conformance/fixtures")).unwrap();
        fs::create_dir_all(root.join("sdk/conformance/runner")).unwrap();
        fs::create_dir_all(root.join("sdk/schemas")).unwrap();
        fs::write(root.join("sdk/conformance/runner/README.md"), "# runner\n").unwrap();
        fs::write(
            root.join("sdk/conformance/cases/minimal.yaml"),
            format!(
                r#"
id: test/minimal
profile: runtime_core
required_for:
  - {language}
steps:
  - action: noop
expect:
  result: ok
"#
            ),
        )
        .unwrap();
    }
}
