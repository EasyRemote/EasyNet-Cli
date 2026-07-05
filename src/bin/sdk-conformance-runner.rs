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
// every language facade must consume. When given a language action-adapter
// report, it also proves that the report is closed over required manifest cases
// and backed by repository-local evidence.
//
// Implementation Approach
// -----------------------
// Treat YAML cases as declarative manifests, bind every referenced fixture to a
// repository schema, validate the fixture payload before adapter execution, and
// emit one stable result record per case/language pair. This moves the
// conformance runner root from README-only scaffold to a CI-usable integrity
// gate without introducing a second daemon or Axon semantic path.
//
// Usage
// -----
//   cargo run --bin sdk-conformance-runner -- --language rust \
//     --adapter-report sdk/conformance/runner/rust-action-adapter-report.json
//   cargo run --bin sdk-conformance-runner -- --language c_abi \
//     --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json
//   cargo run --bin sdk-conformance-runner -- --language go \
//     --adapter-report sdk/conformance/runner/go-action-adapter-report.json
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use regex::Regex;
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

#[derive(Debug, Clone, Deserialize)]
struct FixtureSchemaBindingManifest {
    schema_version: u64,
    bindings: Vec<FixtureSchemaBindingRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureSchemaBindingRecord {
    fixture: String,
    schema: String,
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
    let fixture_schemas = FixtureSchemaBindings::load(root)?;
    let adapter_report = {
        let case_index = ManifestCaseIndex::new(&cases);
        adapter_report_path
            .map(|path| load_adapter_report(root, language, path, &case_index))
            .transpose()?
    };
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
        errors.extend(validate_case_references(root, &case, &fixture_schemas));

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

fn load_adapter_report(
    root: &Path,
    language: &str,
    path: &Path,
    case_index: &ManifestCaseIndex<'_>,
) -> Result<AdapterReport> {
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
        let Some(case) = case_index.find(&record.case_id) else {
            anyhow::bail!(
                "adapter record `{}` does not match any manifest case",
                record.case_id
            );
        };
        if !case.required_for.contains(language) {
            anyhow::bail!(
                "adapter record `{}` is not declared for language `{}`",
                record.case_id,
                language
            );
        }
        if record.evidence.is_empty() {
            anyhow::bail!("adapter record `{}` must include evidence", record.case_id);
        }
        for evidence in &record.evidence {
            validate_adapter_evidence(root, language, &record.case_id, evidence)?;
        }
    }
    Ok(report)
}

#[derive(Debug)]
struct ManifestCaseIndex<'a> {
    cases: BTreeMap<&'a str, &'a ConformanceCase>,
}

impl<'a> ManifestCaseIndex<'a> {
    fn new(cases: &'a [ConformanceCase]) -> Self {
        Self {
            cases: cases.iter().map(|case| (case.id.as_str(), case)).collect(),
        }
    }

    fn find(&self, case_id: &str) -> Option<&'a ConformanceCase> {
        self.cases.get(case_id).copied()
    }
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

#[derive(Debug)]
struct FixtureSchemaBindings {
    bindings: BTreeMap<String, String>,
}

impl FixtureSchemaBindings {
    fn load(root: &Path) -> Result<Self> {
        let path = root.join("sdk/conformance/fixture-schema-bindings.json");
        let manifest: FixtureSchemaBindingManifest = read_json_file(&path)
            .with_context(|| format!("load fixture schema bindings {}", path.display()))?;
        if manifest.schema_version != 1 {
            anyhow::bail!("fixture schema bindings schema_version must be 1");
        }
        let mut bindings = BTreeMap::new();
        for binding in manifest.bindings {
            validate_manifest_file_name(&binding.fixture, ".v4.json")
                .with_context(|| format!("invalid fixture binding `{}`", binding.fixture))?;
            validate_manifest_file_name(&binding.schema, ".schema.json")
                .with_context(|| format!("invalid schema binding `{}`", binding.schema))?;
            if bindings
                .insert(binding.fixture.clone(), binding.schema.clone())
                .is_some()
            {
                anyhow::bail!("duplicate fixture schema binding `{}`", binding.fixture);
            }
            let schema_path = root.join("sdk/schemas").join(&binding.schema);
            ensure_path_inside_root(root, &schema_path)
                .with_context(|| format!("validate schema binding {}", schema_path.display()))?;
        }
        if bindings.is_empty() {
            anyhow::bail!("fixture schema bindings must not be empty");
        }
        Ok(Self { bindings })
    }

    fn schema_for(&self, fixture: &str) -> Option<&str> {
        self.bindings.get(fixture).map(String::as_str)
    }
}

fn validate_manifest_file_name(raw: &str, suffix: &str) -> Result<()> {
    if raw.trim().is_empty() || !raw.ends_with(suffix) {
        anyhow::bail!("expected file name ending with `{suffix}`");
    }
    let path = Path::new(raw);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("file name must stay relative to the conformance directory");
    }
    Ok(())
}

fn validate_adapter_evidence(
    root: &Path,
    language: &str,
    case_id: &str,
    evidence: &AdapterEvidence,
) -> Result<()> {
    let expected_kind = adapter_evidence_kind(language);
    if evidence.kind != expected_kind {
        anyhow::bail!(
            "adapter record `{case_id}` evidence kind `{}` does not match language `{language}`; expected `{expected_kind}`",
            evidence.kind
        );
    }
    let path = resolve_repo_path(root, Path::new(&evidence.ref_path));
    ensure_path_inside_root(root, &path)
        .with_context(|| format!("validate adapter evidence {}", path.display()))
}

fn adapter_evidence_kind(language: &str) -> String {
    format!("{}_test", language.replace('-', "_"))
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
        if let Err(err) = validate_adapter_evidence(root, language, &adapter.case_id, evidence) {
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

fn validate_case_references(
    root: &Path,
    case: &ConformanceCase,
    fixture_schemas: &FixtureSchemaBindings,
) -> Vec<String> {
    let mut refs = CaseReferences::default();
    refs.collect(&case.document);

    let mut errors = Vec::new();
    for fixture in refs.fixtures {
        let Some(schema) = fixture_schemas.schema_for(&fixture) else {
            errors.push(format!("fixture `{fixture}`: missing schema binding"));
            continue;
        };
        if let Err(err) = validate_fixture_against_schema(root, &fixture, schema) {
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

fn read_json_file<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(path).with_context(|| format!("missing {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("invalid json {}", path.display()))
}

fn validate_schema_file(path: &Path) -> Result<()> {
    let json: Value = read_json_file(path)?;
    let object = json
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("schema root must be an object"))?;
    required_string(object.get("$schema"), "$schema")
        .with_context(|| format!("schema {} missing $schema", path.display()))?;
    required_string(object.get("title"), "title")
        .with_context(|| format!("schema {} missing title", path.display()))?;
    Ok(())
}

fn validate_fixture_against_schema(root: &Path, fixture: &str, schema: &str) -> Result<()> {
    let fixture_path = root.join("sdk/conformance/fixtures").join(fixture);
    let fixture_json: Value = read_json_file(&fixture_path)?;
    let schema_json = load_schema_with_local_refs(root, schema, &mut BTreeSet::new())?;
    let errors = FixtureJsonSchemaValidator::default().validate(&fixture_json, &schema_json);
    if !errors.is_empty() {
        anyhow::bail!(
            "schema validation against `{schema}` failed: {}",
            errors.join("; ")
        );
    }
    Ok(())
}

fn load_schema_with_local_refs(
    root: &Path,
    schema: &str,
    stack: &mut BTreeSet<String>,
) -> Result<Value> {
    validate_manifest_file_name(schema, ".schema.json")?;
    if !stack.insert(schema.to_string()) {
        anyhow::bail!("recursive schema reference `{schema}`");
    }
    let schema_path = root.join("sdk/schemas").join(schema);
    let mut value: Value = read_json_file(&schema_path)?;
    inline_local_schema_refs(root, &mut value, stack)?;
    stack.remove(schema);
    Ok(value)
}

fn inline_local_schema_refs(
    root: &Path,
    value: &mut Value,
    stack: &mut BTreeSet<String>,
) -> Result<()> {
    match value {
        Value::Object(object) => {
            if let Some(raw_ref) = object.get("$ref").and_then(Value::as_str) {
                if raw_ref.ends_with(".schema.json") {
                    let mut referenced = load_schema_with_local_refs(root, raw_ref, stack)?;
                    if object.len() > 1 {
                        let siblings = object
                            .iter()
                            .filter(|(key, _)| key.as_str() != "$ref")
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect::<Vec<_>>();
                        let Some(referenced_object) = referenced.as_object_mut() else {
                            anyhow::bail!("referenced schema `{raw_ref}` is not an object");
                        };
                        for (key, value) in siblings {
                            referenced_object.insert(key, value);
                        }
                    }
                    *value = referenced;
                    return Ok(());
                }
            }
            for nested in object.values_mut() {
                inline_local_schema_refs(root, nested, stack)?;
            }
        }
        Value::Array(values) => {
            for nested in values {
                inline_local_schema_refs(root, nested, stack)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Default)]
struct FixtureJsonSchemaValidator;

impl FixtureJsonSchemaValidator {
    fn validate(&self, instance: &Value, schema: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        self.validate_node(instance, schema, "$", &mut errors);
        errors.into_iter().take(5).collect()
    }

    fn validate_node(
        &self,
        instance: &Value,
        schema: &Value,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        if errors.len() >= 5 {
            return;
        }
        match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                errors.push(format!("{path}: boolean schema rejected value"));
                return;
            }
            Value::Object(_) => {}
            _ => {
                errors.push(format!("{path}: schema node must be an object or boolean"));
                return;
            }
        }
        let Some(schema_object) = schema.as_object() else {
            return;
        };

        if let Some(type_rule) = schema_object.get("type") {
            let allowed = schema_type_names(type_rule);
            if allowed.is_empty() {
                errors.push(format!(
                    "{path}: schema type rule must be a string or array"
                ));
            } else if !allowed
                .iter()
                .any(|expected| instance_matches_json_type(instance, expected))
            {
                errors.push(format!(
                    "{path}: expected type {}, got {}",
                    allowed.join("|"),
                    json_type_name(instance)
                ));
            }
        }

        if let Some(expected) = schema_object.get("const") {
            if instance != expected {
                errors.push(format!("{path}: expected const {}", compact_json(expected)));
            }
        }

        if let Some(values) = schema_object.get("enum") {
            match values.as_array() {
                Some(values) if values.iter().any(|candidate| candidate == instance) => {}
                Some(_) => errors.push(format!("{path}: value is not in enum")),
                None => errors.push(format!("{path}: schema enum must be an array")),
            }
        }

        if let Some(one_of) = schema_object.get("oneOf") {
            self.validate_one_of(instance, one_of, path, errors);
        }
        if let Some(any_of) = schema_object.get("anyOf") {
            self.validate_any_of(instance, any_of, path, errors);
        }

        if let Some(not_schema) = schema_object.get("not") {
            let mut nested_errors = Vec::new();
            self.validate_node(instance, not_schema, path, &mut nested_errors);
            if nested_errors.is_empty() {
                errors.push(format!("{path}: value matched forbidden schema"));
            }
        }

        self.validate_object_rules(instance, schema_object, path, errors);
        self.validate_array_rules(instance, schema_object, path, errors);
        self.validate_string_rules(instance, schema_object, path, errors);
        self.validate_number_rules(instance, schema_object, path, errors);
    }

    fn validate_one_of(
        &self,
        instance: &Value,
        one_of: &Value,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(candidates) = one_of.as_array() else {
            errors.push(format!("{path}: schema oneOf must be an array"));
            return;
        };
        let mut matches = 0usize;
        for candidate in candidates {
            let mut nested_errors = Vec::new();
            self.validate_node(instance, candidate, path, &mut nested_errors);
            if nested_errors.is_empty() {
                matches += 1;
            }
        }
        if matches != 1 {
            errors.push(format!("{path}: oneOf matched {matches} schemas"));
        }
    }

    fn validate_any_of(
        &self,
        instance: &Value,
        any_of: &Value,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(candidates) = any_of.as_array() else {
            errors.push(format!("{path}: schema anyOf must be an array"));
            return;
        };
        for candidate in candidates {
            let mut nested_errors = Vec::new();
            self.validate_node(instance, candidate, path, &mut nested_errors);
            if nested_errors.is_empty() {
                return;
            }
        }
        errors.push(format!("{path}: anyOf matched no schemas"));
    }

    fn validate_object_rules(
        &self,
        instance: &Value,
        schema_object: &serde_json::Map<String, Value>,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(instance_object) = instance.as_object() else {
            return;
        };

        let properties = schema_object.get("properties").and_then(Value::as_object);
        if let Some(required) = schema_object.get("required") {
            match required.as_array() {
                Some(required) => {
                    for field in required {
                        let Some(field) = field.as_str() else {
                            errors.push(format!("{path}: required field names must be strings"));
                            continue;
                        };
                        if !instance_object.contains_key(field) {
                            errors.push(format!("{path}: missing required property `{field}`"));
                        }
                    }
                }
                None => errors.push(format!("{path}: schema required must be an array")),
            }
        }

        if let Some(properties) = properties {
            for (name, property_schema) in properties {
                if let Some(value) = instance_object.get(name) {
                    let property_path = format!("{path}.{}", escape_json_path_segment(name));
                    self.validate_node(value, property_schema, &property_path, errors);
                }
            }
        }

        if let Some(additional) = schema_object.get("additionalProperties") {
            match additional {
                Value::Bool(false) => {
                    if let Some(properties) = properties {
                        for name in instance_object.keys() {
                            if !properties.contains_key(name) {
                                errors.push(format!("{path}: unexpected property `{name}`"));
                            }
                        }
                    }
                }
                Value::Bool(true) | Value::Null => {}
                Value::Object(_) => {
                    for (name, value) in instance_object {
                        if properties.is_some_and(|properties| properties.contains_key(name)) {
                            continue;
                        }
                        let property_path = format!("{path}.{}", escape_json_path_segment(name));
                        self.validate_node(value, additional, &property_path, errors);
                    }
                }
                _ => errors.push(format!(
                    "{path}: schema additionalProperties must be boolean or object"
                )),
            }
        }
    }

    fn validate_array_rules(
        &self,
        instance: &Value,
        schema_object: &serde_json::Map<String, Value>,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(values) = instance.as_array() else {
            return;
        };
        if let Some(items) = schema_object.get("items") {
            for (index, value) in values.iter().enumerate() {
                self.validate_node(value, items, &format!("{path}[{index}]"), errors);
            }
        }
        if let Some(min_items) = schema_object.get("minItems").and_then(Value::as_u64) {
            if values.len() < min_items as usize {
                errors.push(format!("{path}: array length is below {min_items}"));
            }
        }
    }

    fn validate_string_rules(
        &self,
        instance: &Value,
        schema_object: &serde_json::Map<String, Value>,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(value) = instance.as_str() else {
            return;
        };
        if let Some(min_length) = schema_object.get("minLength").and_then(Value::as_u64) {
            if value.chars().count() < min_length as usize {
                errors.push(format!("{path}: string length is below {min_length}"));
            }
        }
        if let Some(max_length) = schema_object.get("maxLength").and_then(Value::as_u64) {
            if value.chars().count() > max_length as usize {
                errors.push(format!("{path}: string length is above {max_length}"));
            }
        }
        if let Some(pattern) = schema_object.get("pattern").and_then(Value::as_str) {
            match Regex::new(pattern) {
                Ok(regex) if !regex.is_match(value) => {
                    errors.push(format!("{path}: string does not match pattern `{pattern}`"));
                }
                Ok(_) => {}
                Err(err) => {
                    errors.push(format!("{path}: invalid schema pattern `{pattern}`: {err}"))
                }
            }
        }
    }

    fn validate_number_rules(
        &self,
        instance: &Value,
        schema_object: &serde_json::Map<String, Value>,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        let Some(value) = instance.as_f64() else {
            return;
        };
        if let Some(minimum) = schema_object.get("minimum").and_then(Value::as_f64) {
            if value < minimum {
                errors.push(format!("{path}: number is below minimum {minimum}"));
            }
        }
        if let Some(maximum) = schema_object.get("maximum").and_then(Value::as_f64) {
            if value > maximum {
                errors.push(format!("{path}: number is above maximum {maximum}"));
            }
        }
    }
}

fn schema_type_names(type_rule: &Value) -> Vec<String> {
    match type_rule {
        Value::String(raw) => vec![raw.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn instance_matches_json_type(instance: &Value, expected: &str) -> bool {
    match expected {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "number" => instance.is_number(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string())
}

fn escape_json_path_segment(segment: &str) -> String {
    if segment
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        segment.to_string()
    } else {
        format!("[{}]", compact_json(&Value::String(segment.to_string())))
    }
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
        write_minimal_schema(root.path(), "minimal.schema.json");
        write_fixture_schema_bindings(root.path(), &[("missing.v4.json", "minimal.schema.json")]);
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
    fn runner_reports_fixture_schema_violation() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("sdk/conformance/cases")).unwrap();
        fs::create_dir_all(root.path().join("sdk/conformance/fixtures")).unwrap();
        fs::create_dir_all(root.path().join("sdk/schemas")).unwrap();
        write_minimal_schema(root.path(), "minimal.schema.json");
        write_fixture_schema_bindings(root.path(), &[("invalid.v4.json", "minimal.schema.json")]);
        fs::write(
            root.path().join("sdk/conformance/fixtures/invalid.v4.json"),
            r#"{"state":"bad"}"#,
        )
        .unwrap();
        fs::write(
            root.path()
                .join("sdk/conformance/cases/invalid-fixture.yaml"),
            r#"
id: broken/invalid_fixture
profile: runtime_core
required_for:
  - rust
steps:
  - action: load_fixture
    fixture: invalid.v4.json
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
            .contains("schema validation against `minimal.schema.json` failed"));
    }

    #[test]
    fn runner_validates_repository_adapter_reports() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (language, report) in [
            (
                "rust",
                "sdk/conformance/runner/rust-action-adapter-report.json",
            ),
            (
                "c_abi",
                "sdk/conformance/runner/c-abi-action-adapter-report.json",
            ),
            ("go", "sdk/conformance/runner/go-action-adapter-report.json"),
            (
                "python",
                "sdk/conformance/runner/python-action-adapter-report.json",
            ),
        ] {
            let report = root.join(report);
            let records = run_manifest(root, language, Some(&report))
                .expect("runner validates adapter report");

            let required: Vec<_> = records
                .iter()
                .filter(|record| {
                    record.language == language && record.status != ConformanceStatus::Skipped
                })
                .collect();
            assert!(!required.is_empty(), "{language} must have required cases");
            assert!(
                required
                    .iter()
                    .all(|record| record.status == ConformanceStatus::Passed),
                "{language} adapter report must pass every required case"
            );
        }
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
      "evidence": [{"kind": "python_test", "ref_path": "sdk/conformance/runner/README.md"}],
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

    #[test]
    fn runner_rejects_cross_language_adapter_evidence() {
        let root = tempfile::tempdir().expect("tempdir");
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("adapter.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 1,
  "language": "go",
  "adapter_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "status": "passed",
      "evidence": [{"kind": "rust_test", "ref_path": "sdk/conformance/runner/README.md"}],
      "message": null
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest(root.path(), "go", Some(&report))
            .expect_err("cross-language adapter evidence must fail");

        assert!(err.to_string().contains("expected `go_test`"));
    }

    #[test]
    fn runner_rejects_unknown_adapter_record_case() {
        let root = tempfile::tempdir().expect("tempdir");
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("adapter.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 1,
  "language": "go",
  "adapter_kind": "unit_test",
  "records": [
    {
      "case_id": "test/unknown",
      "profile": "runtime_core",
      "status": "passed",
      "evidence": [{"kind": "runner_test", "ref_path": "sdk/conformance/runner/README.md"}],
      "message": null
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest(root.path(), "go", Some(&report))
            .expect_err("unknown adapter case must fail");

        assert!(err.to_string().contains("does not match any manifest case"));
    }

    #[test]
    fn runner_rejects_language_undeclared_adapter_record_case() {
        let root = tempfile::tempdir().expect("tempdir");
        create_minimal_case_root(root.path(), "rust");
        let report = root.path().join("adapter.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 1,
  "language": "go",
  "adapter_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "status": "passed",
      "evidence": [{"kind": "runner_test", "ref_path": "sdk/conformance/runner/README.md"}],
      "message": null
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest(root.path(), "go", Some(&report))
            .expect_err("language-undeclared adapter case must fail");

        assert!(err
            .to_string()
            .contains("is not declared for language `go`"));
    }

    fn create_minimal_case_root(root: &Path, language: &str) {
        fs::create_dir_all(root.join("sdk/conformance/cases")).unwrap();
        fs::create_dir_all(root.join("sdk/conformance/fixtures")).unwrap();
        fs::create_dir_all(root.join("sdk/conformance/runner")).unwrap();
        fs::create_dir_all(root.join("sdk/schemas")).unwrap();
        fs::write(root.join("sdk/conformance/runner/README.md"), "# runner\n").unwrap();
        write_minimal_schema(root, "minimal.schema.json");
        fs::write(
            root.join("sdk/conformance/fixtures/minimal.v4.json"),
            r#"{"state":"ok"}"#,
        )
        .unwrap();
        write_fixture_schema_bindings(root, &[("minimal.v4.json", "minimal.schema.json")]);
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

    fn write_minimal_schema(root: &Path, name: &str) {
        fs::write(
            root.join("sdk/schemas").join(name),
            r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Minimal",
  "type": "object",
  "additionalProperties": false,
  "required": ["state"],
  "properties": {
    "state": {"const": "ok"}
  }
}"#,
        )
        .unwrap();
    }

    fn write_fixture_schema_bindings(root: &Path, bindings: &[(&str, &str)]) {
        let records = bindings
            .iter()
            .map(|(fixture, schema)| {
                format!(r#"{{"fixture":"{}","schema":"{}"}}"#, fixture, schema)
            })
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            root.join("sdk/conformance/fixture-schema-bindings.json"),
            format!(r#"{{"schema_version":1,"bindings":[{records}]}}"#),
        )
        .unwrap();
    }
}
