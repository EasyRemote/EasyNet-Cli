// EasyNet CLI — SDK conformance manifest runner
// =============================================
//
// File: tools/sdk-conformance-runner/src/main.rs
// Description: Loads language-neutral SDK conformance cases, validates their
//              fixture/schema references, and emits machine-readable runner
//              records.
//
// Protocol Responsibility
// -----------------------
// Own the executable runner contract for the repository's SDK conformance
// manifest. This runner validates the shared case/fixture/schema graph that
// every language facade must consume. When given a language runtime-conformance
// report, it also proves that the report is closed over required manifest cases
// and backed by repository-local evidence.
//
// Implementation Approach
// -----------------------
// Treat YAML cases as declarative manifests, bind every referenced fixture to a
// repository schema, validate the fixture payload before conformance execution, and
// emit one stable result record per case/language pair. This moves the
// conformance runner root from README-only scaffold to a CI-usable integrity
// gate without introducing a second daemon or Axon semantic path.
//
// Usage
// -----
//   cargo run -p sdk-conformance-runner -- --language rust \
//     --conformance-report sdk/conformance/runner/rust-runtime-conformance-report.json
//   cargo run -p sdk-conformance-runner -- --language c_abi \
//     --conformance-report sdk/conformance/runner/c-abi-runtime-conformance-report.json
//   cargo run -p sdk-conformance-runner -- --language go \
//     --conformance-report sdk/conformance/runner/go-runtime-conformance-report.json
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

    /// Optional language runtime conformance report. Report mode always executes
    /// runner-owned case selectors.
    #[arg(long)]
    conformance_report: Option<PathBuf>,

    /// Emit a runner-issued nonce for a multi-language gate invocation.
    #[arg(long)]
    issue_run_nonce: bool,
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
    sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ConformanceResultRecord {
    case_id: String,
    language: String,
    profile: String,
    case_sha256: String,
    selector: Option<String>,
    evidence: Vec<ConformanceEvidence>,
    collected_tests: Vec<String>,
    attestation_sha256: Option<String>,
    status: ConformanceStatus,
    error_code: Option<&'static str>,
    message: Option<String>,
    executions: Vec<ConformanceExecutionProof>,
    run_nonce: String,
    tree_sha256: String,
    toolchain_sha256: String,
    toolchain_version: String,
    axon_revision: String,
}

#[derive(Debug, Clone, Serialize)]
struct RunAttestationContext {
    run_nonce: String,
    tree_sha256: String,
    toolchain_sha256: String,
    toolchain_version: String,
    axon_revision: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ConformanceStatus {
    Passed,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceReport {
    schema_version: u64,
    language: String,
    report_kind: String,
    records: Vec<ConformanceReportRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceReportRecord {
    case_id: String,
    profile: String,
    evidence: Vec<ConformanceEvidence>,
    coverage: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConformanceEvidence {
    kind: String,
    ref_path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ConformanceExecutionProof {
    phase: &'static str,
    command: Vec<String>,
    working_directory: String,
    exit_code: i32,
    output_sha256: String,
}

#[derive(Debug, Clone)]
struct ConformanceExecution {
    proofs: Vec<ConformanceExecutionProof>,
    collected_tests: Vec<String>,
    failure: Option<String>,
}

trait ConformanceExecutor {
    fn execute(&self, root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution>;

    fn execute_many(
        &self,
        root: &Path,
        bindings: &[ExecutionBinding],
    ) -> BTreeMap<(String, String), ConformanceExecution> {
        execute_cases_individually(self, root, bindings)
    }
}

#[derive(Debug, Default)]
struct ProcessConformanceExecutor;

impl ConformanceExecutor for ProcessConformanceExecutor {
    fn execute(&self, root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
        execute_conformance_case(root, binding)
    }

    fn execute_many(
        &self,
        root: &Path,
        bindings: &[ExecutionBinding],
    ) -> BTreeMap<(String, String), ConformanceExecution> {
        if bindings.is_empty() {
            return BTreeMap::new();
        }
        let language = bindings[0].language.as_str();
        let result = match language {
            "go" => execute_go_cases(root, bindings),
            "rust" | "c_abi" => execute_rust_cases(root, bindings),
            "java" => execute_java_cases(root, bindings),
            "swift" => execute_swift_cases(root, bindings),
            _ => return execute_cases_individually(self, root, bindings),
        };
        result.unwrap_or_else(|error| {
            bindings
                .iter()
                .map(|binding| {
                    (
                        execution_key(binding),
                        ConformanceExecution {
                            proofs: Vec::new(),
                            collected_tests: Vec::new(),
                            failure: Some(format!(
                                "conformance batch execution could not start: {error:#}"
                            )),
                        },
                    )
                })
                .collect()
        })
    }
}

fn execute_cases_individually<T: ConformanceExecutor + ?Sized>(
    executor: &T,
    root: &Path,
    bindings: &[ExecutionBinding],
) -> BTreeMap<(String, String), ConformanceExecution> {
    bindings
        .iter()
        .map(|binding| {
            let execution =
                executor
                    .execute(root, binding)
                    .unwrap_or_else(|error| ConformanceExecution {
                        proofs: Vec::new(),
                        collected_tests: Vec::new(),
                        failure: Some(format!("conformance execution could not start: {error:#}")),
                    });
            (execution_key(binding), execution)
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionManifest {
    schema_version: u64,
    bindings: Vec<ExecutionBinding>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionBinding {
    language: String,
    case_id: String,
    selector: String,
    evidence: Vec<String>,
}

#[derive(Debug)]
struct ExecutionManifestIndex {
    bindings: BTreeMap<(String, String), ExecutionBinding>,
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
    if cli.issue_run_nonce {
        println!("{}", issue_run_nonce(&cli.root)?);
        return Ok(());
    }
    let mut records = if cli.conformance_report.is_some() {
        run_manifest_with_executor(
            &cli.root,
            &cli.language,
            cli.conformance_report.as_deref(),
            &ProcessConformanceExecutor,
        )?
    } else {
        run_manifest(&cli.root, &cli.language, cli.conformance_report.as_deref())?
    };
    bind_run_context(&cli.root, &cli.language, &mut records)?;
    let failed = records
        .iter()
        .any(|record| record.status == ConformanceStatus::Failed);
    emit_records(&records, cli.format)?;

    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn run_manifest(
    root: &Path,
    language: &str,
    conformance_report_path: Option<&Path>,
) -> Result<Vec<ConformanceResultRecord>> {
    if conformance_report_path.is_some() {
        anyhow::bail!("conformance report execution is mandatory; no executor was provided");
    }
    run_manifest_with_optional_executor(root, language, None, None)
}

fn run_manifest_with_executor(
    root: &Path,
    language: &str,
    conformance_report_path: Option<&Path>,
    executor: &dyn ConformanceExecutor,
) -> Result<Vec<ConformanceResultRecord>> {
    run_manifest_with_optional_executor(root, language, conformance_report_path, Some(executor))
}

fn run_manifest_with_optional_executor(
    root: &Path,
    language: &str,
    conformance_report_path: Option<&Path>,
    executor: Option<&dyn ConformanceExecutor>,
) -> Result<Vec<ConformanceResultRecord>> {
    let cases = load_cases(root)?;
    let fixture_schemas = FixtureSchemaBindings::load(root)?;
    let conformance_report = {
        let case_index = ManifestCaseIndex::new(&cases);
        conformance_report_path
            .map(|path| load_conformance_report(root, language, path, &case_index))
            .transpose()?
    };
    if conformance_report.is_some() && executor.is_none() {
        anyhow::bail!("conformance report execution is mandatory; no executor was provided");
    }
    let execution_manifest = conformance_report
        .as_ref()
        .map(|_| ExecutionManifestIndex::load(root, &cases))
        .transpose()?;
    let execution_results = match (&execution_manifest, executor) {
        (Some(manifest), Some(executor)) => {
            let bindings = cases
                .iter()
                .filter(|case| case.required_for.contains(language))
                .filter_map(|case| manifest.find(language, &case.id).cloned())
                .collect::<Vec<_>>();
            executor.execute_many(root, &bindings)
        }
        _ => BTreeMap::new(),
    };
    let conformance_report = conformance_report.as_ref().map(ConformanceReportIndex::new);
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
                status: ConformanceStatus::Unsupported,
                error_code: Some("CAPABILITY_UNSUPPORTED"),
                message: Some(format!("case is unsupported for language `{language}`")),
                case_sha256: case.sha256,
                selector: None,
                evidence: Vec::new(),
                collected_tests: Vec::new(),
                attestation_sha256: None,
                executions: Vec::new(),
                run_nonce: String::new(),
                tree_sha256: String::new(),
                toolchain_sha256: String::new(),
                toolchain_version: String::new(),
                axon_revision: String::new(),
            }
        } else if errors.is_empty() {
            match conformance_report.as_ref() {
                Some(report) => {
                    let binding = execution_manifest
                        .as_ref()
                        .and_then(|manifest| manifest.find(language, &case.id));
                    let execution =
                        binding.and_then(|binding| execution_results.get(&execution_key(binding)));
                    record_from_conformance_report(
                        root, language, &case, report, binding, execution,
                    )
                }
                None => ConformanceResultRecord {
                    case_id: case.id,
                    language: language.to_string(),
                    profile: case.profile,
                    case_sha256: case.sha256,
                    selector: None,
                    evidence: Vec::new(),
                    collected_tests: Vec::new(),
                    attestation_sha256: None,
                    status: ConformanceStatus::Unsupported,
                    error_code: Some("MANIFEST_ONLY"),
                    message: Some(
                        "manifest integrity validated without conformance execution".to_string(),
                    ),
                    executions: Vec::new(),
                    run_nonce: String::new(),
                    tree_sha256: String::new(),
                    toolchain_sha256: String::new(),
                    toolchain_version: String::new(),
                    axon_revision: String::new(),
                },
            }
        } else {
            ConformanceResultRecord {
                case_id: case.id,
                language: language.to_string(),
                profile: case.profile,
                case_sha256: case.sha256,
                selector: None,
                evidence: Vec::new(),
                collected_tests: Vec::new(),
                attestation_sha256: None,
                status: ConformanceStatus::Failed,
                error_code: Some("CONFORMANCE_MANIFEST_INVALID"),
                message: Some(errors.join("; ")),
                executions: Vec::new(),
                run_nonce: String::new(),
                tree_sha256: String::new(),
                toolchain_sha256: String::new(),
                toolchain_version: String::new(),
                axon_revision: String::new(),
            }
        };
        records.push(record);
    }

    Ok(records)
}

fn load_conformance_report(
    root: &Path,
    language: &str,
    path: &Path,
    case_index: &ManifestCaseIndex<'_>,
) -> Result<ConformanceReport> {
    let path = resolve_repo_path(root, path);
    ensure_path_inside_root(root, &path).with_context(|| {
        format!(
            "conformance report {} must stay under repository root {}",
            path.display(),
            root.display()
        )
    })?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read conformance report {}", path.display()))?;
    let report: ConformanceReport = serde_json::from_str(&raw)
        .with_context(|| format!("decode conformance report {}", path.display()))?;
    if report.schema_version != 2 {
        anyhow::bail!("conformance report schema_version must be 2");
    }
    if report.language != language {
        anyhow::bail!(
            "conformance report language `{}` does not match requested language `{}`",
            report.language,
            language
        );
    }
    if report.report_kind.trim().is_empty() {
        anyhow::bail!("conformance report_kind must not be empty");
    }
    let mut seen = BTreeSet::new();
    for record in &report.records {
        if record.case_id.trim().is_empty() {
            anyhow::bail!("conformance record case_id must not be empty");
        }
        if record.profile.trim().is_empty() {
            anyhow::bail!("conformance record profile must not be empty");
        }
        if record
            .coverage
            .as_deref()
            .is_some_and(|coverage| coverage.trim().is_empty())
        {
            anyhow::bail!("conformance record coverage must not be empty when present");
        }
        if !seen.insert(record.case_id.clone()) {
            anyhow::bail!("duplicate conformance record case_id `{}`", record.case_id);
        }
        let Some(case) = case_index.find(&record.case_id) else {
            anyhow::bail!(
                "conformance record `{}` does not match any manifest case",
                record.case_id
            );
        };
        if !case.required_for.contains(language) {
            anyhow::bail!(
                "conformance record `{}` is not declared for language `{}`",
                record.case_id,
                language
            );
        }
        if record.evidence.is_empty() {
            anyhow::bail!(
                "conformance record `{}` must include evidence",
                record.case_id
            );
        }
        for evidence in &record.evidence {
            validate_conformance_evidence(root, language, &record.case_id, evidence)?;
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
struct ConformanceReportIndex<'a> {
    records: BTreeMap<&'a str, &'a ConformanceReportRecord>,
}

impl<'a> ConformanceReportIndex<'a> {
    fn new(report: &'a ConformanceReport) -> Self {
        Self {
            records: report
                .records
                .iter()
                .map(|record| (record.case_id.as_str(), record))
                .collect(),
        }
    }

    fn find(&self, case_id: &str) -> Option<&'a ConformanceReportRecord> {
        self.records.get(case_id).copied()
    }
}

impl ExecutionManifestIndex {
    fn load(root: &Path, cases: &[ConformanceCase]) -> Result<Self> {
        let path = root.join("sdk/conformance/runner/execution-manifest.json");
        let manifest: ExecutionManifest = read_json_file(&path)
            .with_context(|| format!("load execution manifest {}", path.display()))?;
        if manifest.schema_version != 1 {
            anyhow::bail!("execution manifest schema_version must be 1");
        }

        let case_index = ManifestCaseIndex::new(cases);
        let mut bindings = BTreeMap::new();
        let mut selectors = BTreeSet::new();
        for binding in manifest.bindings {
            let Some(case) = case_index.find(&binding.case_id) else {
                anyhow::bail!(
                    "execution binding `{}/{}` does not match any manifest case",
                    binding.language,
                    binding.case_id
                );
            };
            if !case.required_for.contains(&binding.language) {
                anyhow::bail!(
                    "execution binding `{}/{}` is not declared by the case",
                    binding.language,
                    binding.case_id
                );
            }
            if binding.selector.trim().is_empty() {
                anyhow::bail!(
                    "execution binding `{}/{}` selector must not be empty",
                    binding.language,
                    binding.case_id
                );
            }
            if !selectors.insert((binding.language.clone(), binding.selector.clone())) {
                anyhow::bail!(
                    "execution selector `{}/{}` proves more than one case",
                    binding.language,
                    binding.selector
                );
            }
            if binding.evidence.is_empty() {
                anyhow::bail!(
                    "execution binding `{}/{}` evidence must not be empty",
                    binding.language,
                    binding.case_id
                );
            }
            for evidence in &binding.evidence {
                validate_evidence_scope(&binding.language, evidence).with_context(|| {
                    format!(
                        "execution binding `{}/{}` has invalid evidence",
                        binding.language, binding.case_id
                    )
                })?;
            }
            validate_selector_declaration(root, &binding)?;
            let key = (binding.language.clone(), binding.case_id.clone());
            if bindings.insert(key, binding).is_some() {
                anyhow::bail!("duplicate execution binding");
            }
        }
        Ok(Self { bindings })
    }

    fn find(&self, language: &str, case_id: &str) -> Option<&ExecutionBinding> {
        self.bindings
            .get(&(language.to_string(), case_id.to_string()))
    }
}

fn validate_selector_declaration(root: &Path, binding: &ExecutionBinding) -> Result<()> {
    let escaped = regex::escape(&binding.selector);
    let pattern = match binding.language.as_str() {
        "go" => format!(r"(?m)^func\s+{escaped}\s*\([^)]*\*testing\.T[^)]*\)"),
        "python" => format!(r"(?m)^\s*def\s+{escaped}\s*\("),
        "rust" | "c_abi" => format!(r"(?m)^\s*fn\s+{escaped}\s*\("),
        "node" => format!(r#"(?m)^\s*(?:test|it)\(\s*[\"']{escaped}[\"']"#),
        "java" => format!(r"(?m)^\s*private\s+static\s+void\s+{escaped}\s*\("),
        "swift" => format!(r"(?m)^\s*func\s+{escaped}\s*\("),
        language => {
            anyhow::bail!("no selector declaration grammar is registered for language `{language}`")
        }
    };
    let declaration = Regex::new(&pattern)?;
    let mut matches = 0;
    for evidence in &binding.evidence {
        let path = root.join(evidence);
        ensure_path_inside_root(root, &path)?;
        let source = fs::read_to_string(&path)
            .with_context(|| format!("read selector evidence {}", path.display()))?;
        matches += declaration
            .find_iter(&source)
            .filter(|matched| {
                !matches!(binding.language.as_str(), "rust" | "c_abi")
                    || rust_test_attribute_precedes(&source, matched.start())
            })
            .count();
    }
    if matches != 1 {
        anyhow::bail!(
            "execution binding `{}/{}` selector `{}` must be declared exactly once in its evidence; found {matches}",
            binding.language,
            binding.case_id,
            binding.selector
        );
    }
    Ok(())
}

fn rust_test_attribute_precedes(source: &str, function_start: usize) -> bool {
    let target = function_start.saturating_sub(512);
    let prefix_start = source[..function_start]
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| *index >= target)
        .unwrap_or(0);
    let prefix = &source[prefix_start..function_start];
    let attribute = prefix
        .rfind("#[test]")
        .or_else(|| prefix.rfind("#[tokio::test"));
    let previous_function = prefix.rfind("\n    fn ").or_else(|| prefix.rfind("\nfn "));
    attribute.is_some_and(|attribute| previous_function.is_none_or(|function| attribute > function))
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
            validate_fixture_file_name(&binding.fixture)
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

fn validate_fixture_file_name(raw: &str) -> Result<()> {
    validate_manifest_file_name(raw, ".json")?;
    if !is_versioned_fixture_file_name(raw) {
        anyhow::bail!("expected versioned fixture name ending with `.v<major>.json`");
    }
    Ok(())
}

fn is_versioned_fixture_file_name(raw: &str) -> bool {
    let Some((stem, version_suffix)) = raw.rsplit_once(".v") else {
        return false;
    };
    let Some(version) = version_suffix.strip_suffix(".json") else {
        return false;
    };
    !stem.is_empty()
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit())
        && version.parse::<u64>().is_ok_and(|version| version > 0)
}

fn validate_conformance_evidence(
    root: &Path,
    language: &str,
    case_id: &str,
    evidence: &ConformanceEvidence,
) -> Result<()> {
    let expected_kind = conformance_evidence_kind(language);
    if evidence.kind != expected_kind {
        anyhow::bail!(
            "conformance record `{case_id}` evidence kind `{}` does not match language `{language}`; expected `{expected_kind}`",
            evidence.kind
        );
    }
    validate_evidence_scope(language, &evidence.ref_path)
        .with_context(|| format!("conformance record `{case_id}` has invalid evidence scope"))?;
    let path = resolve_repo_path(root, Path::new(&evidence.ref_path));
    ensure_path_inside_root(root, &path)
        .with_context(|| format!("validate conformance evidence {}", path.display()))?;
    let contents =
        fs::read(&path).with_context(|| format!("read conformance evidence {}", path.display()))?;
    let actual = sha256_hex(&contents);
    if evidence.sha256 != actual {
        anyhow::bail!(
            "conformance record `{case_id}` evidence hash mismatch for `{}`: expected `{}`, actual `{actual}`",
            evidence.ref_path,
            evidence.sha256
        );
    }
    Ok(())
}

fn validate_evidence_scope(language: &str, ref_path: &str) -> Result<()> {
    let path = Path::new(ref_path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("evidence path must be repository-relative");
    }
    let normalized = ref_path.replace('\\', "/");
    let allowed = match language {
        "rust" => normalized.starts_with("src/") && normalized.ends_with(".rs"),
        "c_abi" => normalized.starts_with("src/ffi/") && normalized.ends_with(".rs"),
        "go" => normalized.starts_with("sdk/go/") && normalized.ends_with("_test.go"),
        "python" => normalized.starts_with("sdk/python/tests/test_") && normalized.ends_with(".py"),
        "node" => {
            normalized.starts_with("sdk/node/test/")
                && (normalized.ends_with(".mjs") || normalized.ends_with(".js"))
        }
        "java" => normalized.starts_with("sdk/java/src/test/") && normalized.ends_with(".java"),
        "swift" => normalized.starts_with("sdk/swift/Tests/") && normalized.ends_with(".swift"),
        _ => false,
    };
    if !allowed {
        anyhow::bail!(
            "evidence path `{ref_path}` is not covered by the `{language}` conformance suite"
        );
    }
    Ok(())
}

fn conformance_evidence_kind(language: &str) -> String {
    format!("{}_test", language.replace('-', "_"))
}

#[derive(Debug, Clone)]
struct CommandResult {
    proof: ConformanceExecutionProof,
    output: String,
    success: bool,
}

fn execute_conformance_case(
    root: &Path,
    binding: &ExecutionBinding,
) -> Result<ConformanceExecution> {
    match binding.language.as_str() {
        "go" => execute_go_case(root, binding),
        "python" => execute_python_case(root, binding),
        "rust" | "c_abi" => execute_rust_case(root, binding),
        "node" => execute_node_case(root, binding),
        "java" => execute_java_case(root, binding),
        "swift" => execute_swift_case(root, binding),
        language => {
            anyhow::bail!("no executable case executor is registered for language `{language}`")
        }
    }
}

fn execution_key(binding: &ExecutionBinding) -> (String, String) {
    (binding.language.clone(), binding.case_id.clone())
}

const GO_CONFORMANCE_BUILD_TAGS: &str = "runtime_direct,runtime_cabi";

fn execute_go_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let pattern = format!("^{}$", regex::escape(&binding.selector));
    let collect = run_conformance_command(
        root,
        "go",
        "collection",
        "sdk/go",
        &[
            "go",
            "test",
            &format!("-tags={GO_CONFORMANCE_BUILD_TAGS}"),
            "-list",
            &pattern,
            "./...",
        ],
    )?;
    let collected = collect
        .output
        .lines()
        .filter(|line| line.trim() == binding.selector)
        .map(|_| binding.selector.clone())
        .collect::<Vec<_>>();
    let execute = run_conformance_command(
        root,
        "go",
        "execution",
        "sdk/go",
        &[
            "go",
            "test",
            &format!("-tags={GO_CONFORMANCE_BUILD_TAGS}"),
            "-json",
            "-run",
            &pattern,
            "-count=1",
            "./...",
        ],
    )?;
    let passed = execute
        .output
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|event| {
                event.get("Action").and_then(Value::as_str) == Some("pass")
                    && event.get("Test").and_then(Value::as_str) == Some(binding.selector.as_str())
            })
        })
        .count()
        == 1;
    finish_case_execution(binding, collected, vec![collect, execute], passed)
}

fn execute_go_cases(
    root: &Path,
    bindings: &[ExecutionBinding],
) -> Result<BTreeMap<(String, String), ConformanceExecution>> {
    let collect = run_conformance_command(
        root,
        "go",
        "collection",
        "sdk/go",
        &[
            "go",
            "test",
            &format!("-tags={GO_CONFORMANCE_BUILD_TAGS}"),
            "-list",
            ".",
            "./...",
        ],
    )?;
    let listed = collect
        .output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();
    let mut executions = BTreeMap::new();
    for binding in bindings {
        let pattern = format!("^{}$", regex::escape(&binding.selector));
        let collected = listed
            .contains(binding.selector.as_str())
            .then(|| binding.selector.clone())
            .into_iter()
            .collect::<Vec<_>>();
        let execute = run_conformance_command(
            root,
            "go",
            "execution",
            "sdk/go",
            &[
                "go",
                "test",
                &format!("-tags={GO_CONFORMANCE_BUILD_TAGS}"),
                "-json",
                "-run",
                &pattern,
                "-count=1",
                "./...",
            ],
        )?;
        let passed = execute
            .output
            .lines()
            .filter(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|event| {
                    event.get("Action").and_then(Value::as_str) == Some("pass")
                        && event.get("Test").and_then(Value::as_str)
                            == Some(binding.selector.as_str())
                })
            })
            .count()
            == 1;
        executions.insert(
            execution_key(binding),
            finish_case_execution(binding, collected, vec![collect.clone(), execute], passed)?,
        );
    }
    Ok(executions)
}

fn execute_python_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let evidence = binding.evidence.first().expect("validated evidence");
    let evidence = Path::new(evidence)
        .strip_prefix("sdk/python")
        .with_context(|| format!("python evidence must be rooted under sdk/python: {evidence}"))?
        .to_string_lossy()
        .into_owned();
    let python = std::env::var("SDK_CONFORMANCE_PYTHON").unwrap_or_else(|_| "python".to_string());
    let collect = run_conformance_command(
        root,
        "python",
        "collection",
        "sdk/python",
        &[
            &python,
            "-m",
            "pytest",
            "--collect-only",
            "-q",
            &evidence,
            "-k",
            &binding.selector,
        ],
    )?;
    let nodeids = collect
        .output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            (line.rsplit("::").next() == Some(binding.selector.as_str())).then(|| line.to_string())
        })
        .collect::<Vec<_>>();
    let execute = if nodeids.len() == 1 {
        run_conformance_command(
            root,
            "python",
            "execution",
            "sdk/python",
            &[&python, "-m", "pytest", "-q", &nodeids[0]],
        )?
    } else {
        synthetic_failed_command(
            "execution",
            "sdk/python",
            "pytest selector was not collected",
        )
    };
    let passed = execute.success && execute.output.contains("1 passed");
    let collected = nodeids.iter().map(|_| binding.selector.clone()).collect();
    finish_case_execution(binding, collected, vec![collect, execute], passed)
}

fn execute_rust_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let mut executions = execute_rust_cases(root, std::slice::from_ref(binding))?;
    executions
        .remove(&execution_key(binding))
        .ok_or_else(|| anyhow::anyhow!("rust conformance runner did not return case execution"))
}

fn execute_rust_cases(
    root: &Path,
    bindings: &[ExecutionBinding],
) -> Result<BTreeMap<(String, String), ConformanceExecution>> {
    let language = bindings
        .first()
        .map(|binding| binding.language.as_str())
        .unwrap_or("rust");
    let build = run_conformance_command(
        root,
        language,
        "build",
        ".",
        &[
            "cargo",
            "test",
            "--features",
            "axon-pb",
            "--lib",
            "--no-run",
            "--message-format=json",
        ],
    )?;
    let executable = cargo_test_executable(&build.output);
    let collect = executable
        .as_deref()
        .map(|executable| {
            run_conformance_command(root, language, "collection", ".", &[executable, "--list"])
        })
        .transpose()?
        .unwrap_or_else(|| {
            synthetic_failed_command("collection", ".", "cargo test binary was not produced")
        });
    let mut by_selector: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in collect.output.lines() {
        let Some(id) = line.trim().strip_suffix(": test") else {
            continue;
        };
        if let Some(selector) = id.rsplit("::").next() {
            by_selector
                .entry(selector.to_string())
                .or_default()
                .push(id.to_string());
        }
    }
    let mut executions = BTreeMap::new();
    for binding in bindings {
        let test_ids = by_selector
            .get(&binding.selector)
            .cloned()
            .unwrap_or_default();
        let execute = if !build.success {
            synthetic_failed_command("execution", ".", "cargo test binary build failed")
        } else if executable.is_none() {
            synthetic_failed_command("execution", ".", "cargo test binary was not produced")
        } else if test_ids.len() == 1 {
            let executable = executable.as_deref().expect("checked executable");
            run_conformance_command(
                root,
                &binding.language,
                "execution",
                ".",
                &[executable, &test_ids[0], "--exact"],
            )?
        } else {
            synthetic_failed_command("execution", ".", "cargo selector was not collected")
        };
        let passed = execute.success && execute.output.contains("1 passed; 0 failed");
        let collected = test_ids.iter().map(|_| binding.selector.clone()).collect();
        executions.insert(
            execution_key(binding),
            finish_case_execution(
                binding,
                collected,
                vec![build.clone(), collect.clone(), execute],
                passed,
            )?,
        );
    }
    Ok(executions)
}

fn cargo_test_executable(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let event = serde_json::from_str::<Value>(line).ok()?;
        if event.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            return None;
        }
        if !event
            .get("profile")
            .and_then(|profile| profile.get("test"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return None;
        }
        event
            .get("executable")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn execute_node_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let evidence = binding.evidence.first().expect("validated evidence");
    let pattern = format!("^{}$", regex::escape(&binding.selector));
    let collect = run_conformance_command(
        root,
        "node",
        "collection",
        ".",
        &[
            "node",
            "--test",
            "--test-reporter=tap",
            "--test-name-pattern",
            &pattern,
            evidence,
        ],
    )?;
    let count = node_tap_selector_pass_count(&collect.output, &binding.selector);
    let execute = run_conformance_command(
        root,
        "node",
        "execution",
        ".",
        &[
            "node",
            "--test",
            "--test-reporter=tap",
            "--test-name-pattern",
            &pattern,
            evidence,
        ],
    )?;
    let passed =
        execute.success && node_tap_selector_pass_count(&execute.output, &binding.selector) == 1;
    finish_case_execution(
        binding,
        std::iter::repeat_n(binding.selector.clone(), count).collect(),
        vec![collect, execute],
        passed,
    )
}

fn node_tap_selector_pass_count(output: &str, selector: &str) -> usize {
    output
        .lines()
        .filter(|line| {
            line.trim_start()
                .strip_prefix("ok ")
                .and_then(|result| result.split_once(" - "))
                .is_some_and(|(_, name)| name == selector)
        })
        .count()
}

fn execute_java_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let build = run_conformance_command(
        root,
        "java",
        "build",
        "sdk/java",
        &["mvn", "test", "--batch-mode", "--quiet"],
    )?;
    let classpath = "target/test-classes:target/classes";
    let collect = run_conformance_command(
        root,
        "java",
        "collection",
        "sdk/java",
        &[
            "java",
            "-cp",
            classpath,
            "run.runtime.sdk.RuntimeCoreSeamTest",
            "--list",
        ],
    )?;
    let collected = collect
        .output
        .lines()
        .filter(|line| line.trim() == binding.selector)
        .map(|_| binding.selector.clone())
        .collect::<Vec<_>>();
    let execute = run_conformance_command(
        root,
        "java",
        "execution",
        "sdk/java",
        &[
            "java",
            "-cp",
            classpath,
            "run.runtime.sdk.RuntimeCoreSeamTest",
            &binding.selector,
        ],
    )?;
    let passed = execute.success;
    finish_case_execution(binding, collected, vec![build, collect, execute], passed)
}

fn execute_java_cases(
    root: &Path,
    bindings: &[ExecutionBinding],
) -> Result<BTreeMap<(String, String), ConformanceExecution>> {
    let build = run_conformance_command(
        root,
        "java",
        "build",
        "sdk/java",
        &["mvn", "test", "--batch-mode", "--quiet"],
    )?;
    let classpath = "target/test-classes:target/classes";
    let collect = run_conformance_command(
        root,
        "java",
        "collection",
        "sdk/java",
        &[
            "java",
            "-cp",
            classpath,
            "run.runtime.sdk.RuntimeCoreSeamTest",
            "--list",
        ],
    )?;
    let listed = collect
        .output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();
    let mut executions = BTreeMap::new();
    for binding in bindings {
        let collected = listed
            .contains(binding.selector.as_str())
            .then(|| binding.selector.clone())
            .into_iter()
            .collect::<Vec<_>>();
        let execute = run_conformance_command(
            root,
            "java",
            "execution",
            "sdk/java",
            &[
                "java",
                "-cp",
                classpath,
                "run.runtime.sdk.RuntimeCoreSeamTest",
                &binding.selector,
            ],
        )?;
        let passed = execute.success;
        executions.insert(
            execution_key(binding),
            finish_case_execution(
                binding,
                collected,
                vec![build.clone(), collect.clone(), execute],
                passed,
            )?,
        );
    }
    Ok(executions)
}

fn execute_swift_case(root: &Path, binding: &ExecutionBinding) -> Result<ConformanceExecution> {
    let collect = run_conformance_command(
        root,
        "swift",
        "collection",
        "sdk/swift",
        &["swift", "test", "list"],
    )?;
    let test_ids = collect
        .output
        .lines()
        .filter(|line| line.trim().split(['.', '/']).next_back() == Some(binding.selector.as_str()))
        .map(|line| line.trim().to_string())
        .collect::<Vec<_>>();
    let execute = if test_ids.len() == 1 {
        run_conformance_command(
            root,
            "swift",
            "execution",
            "sdk/swift",
            &["swift", "test", "--filter", &test_ids[0]],
        )?
    } else {
        synthetic_failed_command("execution", "sdk/swift", "swift selector was not collected")
    };
    let passed = execute.success && !execute.output.contains("failed");
    let collected = test_ids.iter().map(|_| binding.selector.clone()).collect();
    finish_case_execution(binding, collected, vec![collect, execute], passed)
}

fn execute_swift_cases(
    root: &Path,
    bindings: &[ExecutionBinding],
) -> Result<BTreeMap<(String, String), ConformanceExecution>> {
    let collect = run_conformance_command(
        root,
        "swift",
        "collection",
        "sdk/swift",
        &["swift", "test", "list"],
    )?;
    let mut by_selector: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in collect.output.lines() {
        let test_id = line.trim();
        let Some(selector) = test_id.split(['.', '/']).next_back() else {
            continue;
        };
        by_selector
            .entry(selector.to_string())
            .or_default()
            .push(test_id.to_string());
    }
    let mut executions = BTreeMap::new();
    for binding in bindings {
        let test_ids = by_selector
            .get(&binding.selector)
            .cloned()
            .unwrap_or_default();
        let execute = if test_ids.len() == 1 {
            run_conformance_command(
                root,
                "swift",
                "execution",
                "sdk/swift",
                &["swift", "test", "--filter", &test_ids[0]],
            )?
        } else {
            synthetic_failed_command("execution", "sdk/swift", "swift selector was not collected")
        };
        let passed = execute.success && !execute.output.contains("failed");
        let collected = test_ids.iter().map(|_| binding.selector.clone()).collect();
        executions.insert(
            execution_key(binding),
            finish_case_execution(binding, collected, vec![collect.clone(), execute], passed)?,
        );
    }
    Ok(executions)
}

fn run_conformance_command(
    root: &Path,
    language: &str,
    phase: &'static str,
    working_directory: &str,
    argv: &[&str],
) -> Result<CommandResult> {
    let cwd = root.join(working_directory);
    ensure_path_inside_root(root, &cwd)?;
    let mut command = Command::new(argv[0]);
    command.args(&argv[1..]).current_dir(&cwd);
    if matches!(language, "rust" | "c_abi") {
        let target_dir = std::env::var("SDK_CONFORMANCE_RUST_TARGET_DIR").unwrap_or_else(|_| {
            root.join("target/sdk-conformance-rust-runner")
                .to_string_lossy()
                .into_owned()
        });
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    if language == "python" {
        let mut python_path = format!(
            "{}:{}",
            root.join("sdk/python").display(),
            root.join("../EasyNet-Axon/sdk/python").display()
        );
        if let Some(existing) = std::env::var_os("PYTHONPATH") {
            python_path.push(':');
            python_path.push_str(&existing.to_string_lossy());
        }
        command.env("PYTHONPATH", python_path);
    }
    let output = command
        .output()
        .with_context(|| format!("execute `{}`", argv.join(" ")))?;
    let mut combined = output.stdout;
    combined.extend_from_slice(b"\n--- stderr ---\n");
    combined.extend_from_slice(&output.stderr);
    let decoded = String::from_utf8_lossy(&combined).into_owned();
    Ok(CommandResult {
        proof: ConformanceExecutionProof {
            phase,
            command: argv
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            working_directory: working_directory.to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            output_sha256: sha256_hex(&combined),
        },
        output: decoded,
        success: output.status.success(),
    })
}

fn synthetic_failed_command(phase: &'static str, cwd: &str, message: &str) -> CommandResult {
    CommandResult {
        proof: ConformanceExecutionProof {
            phase,
            command: Vec::new(),
            working_directory: cwd.to_string(),
            exit_code: -1,
            output_sha256: sha256_hex(message.as_bytes()),
        },
        output: message.to_string(),
        success: false,
    }
}

fn finish_case_execution(
    binding: &ExecutionBinding,
    collected_tests: Vec<String>,
    commands: Vec<CommandResult>,
    selector_passed: bool,
) -> Result<ConformanceExecution> {
    let failure = if let Some(command) = commands.iter().find(|command| !command.success) {
        Some(format!(
            "conformance command failed with exit {}: {}",
            command.proof.exit_code,
            output_tail(command.output.as_bytes(), 40)
        ))
    } else if collected_tests != [binding.selector.clone()] {
        Some(format!(
            "selector `{}` must be collected exactly once; collected {:?}",
            binding.selector, collected_tests
        ))
    } else if !selector_passed {
        Some(format!(
            "selector `{}` did not produce one passing result",
            binding.selector
        ))
    } else {
        None
    };
    Ok(ConformanceExecution {
        proofs: commands.into_iter().map(|command| command.proof).collect(),
        collected_tests,
        failure,
    })
}

fn output_tail(output: &[u8], max_lines: usize) -> String {
    let decoded = String::from_utf8_lossy(output);
    let lines = decoded.lines().collect::<Vec<_>>();
    lines[lines.len().saturating_sub(max_lines)..].join("\n")
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

fn record_from_conformance_report(
    root: &Path,
    language: &str,
    case: &ConformanceCase,
    report: &ConformanceReportIndex<'_>,
    binding: Option<&ExecutionBinding>,
    execution: Option<&ConformanceExecution>,
) -> ConformanceResultRecord {
    let Some(report_record) = report.find(&case.id) else {
        return ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            case_sha256: case.sha256.clone(),
            selector: binding.map(|binding| binding.selector.clone()),
            evidence: Vec::new(),
            collected_tests: execution
                .map(|execution| execution.collected_tests.clone())
                .unwrap_or_default(),
            attestation_sha256: None,
            status: ConformanceStatus::Failed,
            error_code: Some("CONFORMANCE_REPORT_MISSING"),
            message: Some(format!(
                "conformance report missing required case `{}`",
                case.id
            )),
            executions: execution
                .map(|execution| execution.proofs.clone())
                .unwrap_or_default(),
            run_nonce: String::new(),
            tree_sha256: String::new(),
            toolchain_sha256: String::new(),
            toolchain_version: String::new(),
            axon_revision: String::new(),
        };
    };
    let mut errors = Vec::new();
    if report_record.profile != case.profile {
        errors.push(format!(
            "conformance profile `{}` does not match case profile `{}`",
            report_record.profile, case.profile
        ));
    }
    for evidence in &report_record.evidence {
        if let Err(err) =
            validate_conformance_evidence(root, language, &report_record.case_id, evidence)
        {
            errors.push(err.to_string());
        }
    }
    let Some(binding) = binding else {
        errors.push(format!(
            "conformance record `{}` has no runner-owned execution binding",
            report_record.case_id
        ));
        return ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            case_sha256: case.sha256.clone(),
            selector: None,
            evidence: report_record.evidence.clone(),
            collected_tests: Vec::new(),
            attestation_sha256: None,
            status: ConformanceStatus::Failed,
            error_code: Some("CONFORMANCE_REPORT_EXECUTION_BINDING_MISSING"),
            message: Some(errors.join("; ")),
            executions: Vec::new(),
            run_nonce: String::new(),
            tree_sha256: String::new(),
            toolchain_sha256: String::new(),
            toolchain_version: String::new(),
            axon_revision: String::new(),
        };
    };
    let report_evidence = report_record
        .evidence
        .iter()
        .map(|evidence| evidence.ref_path.as_str())
        .collect::<BTreeSet<_>>();
    let binding_evidence = binding
        .evidence
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if report_evidence != binding_evidence {
        errors.push(format!(
            "report evidence {:?} does not match runner-owned binding evidence {:?}",
            report_evidence, binding_evidence
        ));
    }
    if execution.is_none() {
        errors.push("required case has no execution result".to_string());
    }
    if execution.is_some_and(|execution| execution.proofs.is_empty()) {
        errors.push("required case execution proof is empty".to_string());
    }
    if execution.is_some_and(|execution| execution.collected_tests != [binding.selector.clone()]) {
        errors.push(format!(
            "selector `{}` was not collected exactly once",
            binding.selector
        ));
    }
    if execution.is_some_and(|execution| {
        !execution
            .proofs
            .iter()
            .any(|proof| proof.phase == "execution")
    }) {
        errors.push("required case has no execution-phase command result".to_string());
    }
    if let Some(failure) = execution.and_then(|execution| execution.failure.as_ref()) {
        errors.push(failure.clone());
    }
    let attestation_sha256 = execution.map(|execution| {
        conformance_attestation_sha256(case, language, binding, report_record, execution)
    });
    if errors.is_empty() {
        ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            case_sha256: case.sha256.clone(),
            selector: Some(binding.selector.clone()),
            evidence: report_record.evidence.clone(),
            collected_tests: execution
                .map(|execution| execution.collected_tests.clone())
                .unwrap_or_default(),
            attestation_sha256: attestation_sha256.clone(),
            status: ConformanceStatus::Passed,
            error_code: None,
            message: None,
            executions: execution
                .map(|execution| execution.proofs.clone())
                .unwrap_or_default(),
            run_nonce: String::new(),
            tree_sha256: String::new(),
            toolchain_sha256: String::new(),
            toolchain_version: String::new(),
            axon_revision: String::new(),
        }
    } else {
        ConformanceResultRecord {
            case_id: case.id.clone(),
            language: language.to_string(),
            profile: case.profile.clone(),
            case_sha256: case.sha256.clone(),
            selector: Some(binding.selector.clone()),
            evidence: report_record.evidence.clone(),
            collected_tests: execution
                .map(|execution| execution.collected_tests.clone())
                .unwrap_or_default(),
            attestation_sha256,
            status: ConformanceStatus::Failed,
            error_code: Some(
                if execution.is_none()
                    || execution.is_some_and(|execution| execution.proofs.is_empty())
                {
                    "CONFORMANCE_REPORT_EXECUTION_MISSING"
                } else if execution
                    .and_then(|execution| execution.failure.as_ref())
                    .is_some()
                {
                    "CONFORMANCE_REPORT_EXECUTION_FAILED"
                } else {
                    "CONFORMANCE_REPORT_FAILED"
                },
            ),
            message: Some(errors.join("; ")),
            executions: execution
                .map(|execution| execution.proofs.clone())
                .unwrap_or_default(),
            run_nonce: String::new(),
            tree_sha256: String::new(),
            toolchain_sha256: String::new(),
            toolchain_version: String::new(),
            axon_revision: String::new(),
        }
    }
}

fn conformance_attestation_sha256(
    case: &ConformanceCase,
    language: &str,
    binding: &ExecutionBinding,
    report_record: &ConformanceReportRecord,
    execution: &ConformanceExecution,
) -> String {
    let payload = serde_json::json!({
        "case_id": case.id,
        "case_sha256": case.sha256,
        "language": language,
        "selector": binding.selector,
        "evidence": report_record.evidence,
        "collected_tests": execution.collected_tests,
        "executions": execution.proofs,
        "execution_failure": execution.failure,
    });
    sha256_hex(&serde_json::to_vec(&payload).expect("attestation payload is serializable"))
}

fn issue_run_nonce(root: &Path) -> Result<String> {
    let mut material = vec![0_u8; 64];
    fs::File::open("/dev/urandom")
        .context("open OS random source")?
        .read_exact(&mut material)
        .context("read OS random source")?;
    material.extend_from_slice(
        &SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_nanos()
            .to_le_bytes(),
    );
    material.extend_from_slice(&std::process::id().to_le_bytes());
    material.extend_from_slice(tree_sha256(root)?.as_bytes());
    Ok(sha256_hex(&material))
}

fn bind_run_context(
    root: &Path,
    language: &str,
    records: &mut [ConformanceResultRecord],
) -> Result<()> {
    if !records
        .iter()
        .any(|record| record.status != ConformanceStatus::Failed)
    {
        return Ok(());
    }
    let run_nonce = std::env::var("SDK_CONFORMANCE_RUN_NONCE")
        .context("SDK_CONFORMANCE_RUN_NONCE must be issued by --issue-run-nonce")?;
    if run_nonce.len() != 64 || !run_nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("SDK_CONFORMANCE_RUN_NONCE must be a 64-character hex digest");
    }
    let (toolchain_sha256, toolchain_version) = toolchain_sha256(root, language)?;
    let context = RunAttestationContext {
        run_nonce,
        tree_sha256: tree_sha256(root)?,
        toolchain_sha256,
        toolchain_version,
        axon_revision: axon_revision(root)?,
    };
    for record in records {
        if record.status == ConformanceStatus::Failed {
            continue;
        }
        record.run_nonce.clone_from(&context.run_nonce);
        record.tree_sha256.clone_from(&context.tree_sha256);
        record
            .toolchain_sha256
            .clone_from(&context.toolchain_sha256);
        record
            .toolchain_version
            .clone_from(&context.toolchain_version);
        record.axon_revision.clone_from(&context.axon_revision);
        if let Some(command_attestation) = record.attestation_sha256.take() {
            let payload = serde_json::json!({
                "command_attestation_sha256": command_attestation,
                "run_context": &context,
            });
            record.attestation_sha256 = Some(sha256_hex(
                &serde_json::to_vec(&payload).expect("run attestation is serializable"),
            ));
        }
    }
    Ok(())
}

fn tree_sha256(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()
        .context("inventory current git tree")?;
    if !output.status.success() {
        anyhow::bail!("git tree inventory failed");
    }
    let mut material = Vec::new();
    for raw in git_inventory_paths(&output.stdout) {
        let path = std::str::from_utf8(raw).context("git path is not UTF-8")?;
        material.extend_from_slice(raw);
        material.push(0);
        let absolute = root.join(path);
        if absolute.is_file() {
            material.extend_from_slice(
                &fs::read(&absolute).with_context(|| format!("hash git tree path {path}"))?,
            );
        } else {
            material.extend_from_slice(b"<deleted>");
        }
        material.push(0);
    }
    Ok(sha256_hex(&material))
}

fn git_inventory_paths(output: &[u8]) -> Vec<&[u8]> {
    let mut paths = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

fn toolchain_sha256(root: &Path, language: &str) -> Result<(String, String)> {
    let python = std::env::var("SDK_CONFORMANCE_PYTHON").unwrap_or_else(|_| "python".to_string());
    let (program, arguments): (&str, &[&str]) = match language {
        "rust" | "c_abi" => ("rustc", &["--version"]),
        "go" => ("go", &["version"]),
        "python" => (python.as_str(), &["--version"]),
        "node" => ("node", &["--version"]),
        "java" => ("java", &["-version"]),
        "swift" => ("swift", &["--version"]),
        _ => anyhow::bail!("unknown SDK language `{language}`"),
    };
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("read {language} toolchain version"))?;
    if !output.status.success() {
        anyhow::bail!("{language} toolchain version command failed");
    }
    let version = String::from_utf8_lossy(if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    })
    .trim()
    .to_string();
    let contract = fs::read(root.join("sdk/conformance/toolchains.json"))?;
    let payload = serde_json::json!({
        "contract_sha256": sha256_hex(&contract),
        "language": language,
        "version": version,
    });
    Ok((
        sha256_hex(&serde_json::to_vec(&payload).expect("toolchain payload is serializable")),
        version,
    ))
}

fn axon_revision(root: &Path) -> Result<String> {
    let repository = std::env::var_os("EASYNET_AXON_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("../EasyNet-Axon"));
    let python = std::env::var("SDK_CONFORMANCE_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let output = Command::new(python)
        .arg(root.join("sdk/conformance/source_revision.py"))
        .arg("--repository")
        .arg(repository)
        .args([
            "--root",
            "sdk",
            "--root",
            "core/proto",
            "--root",
            "core/runtime-rs/dendrite-bridge/include",
        ])
        .output()
        .context("read canonical Axon source revision")?;
    if !output.status.success() {
        anyhow::bail!("canonical Axon source revision command failed");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
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

    if object
        .get("steps")
        .and_then(Value::as_array)
        .is_none_or(|steps| steps.is_empty())
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
        sha256: sha256_hex(raw.as_bytes()),
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
            Value::String(raw) if is_versioned_fixture_file_name(raw) => {
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
    let errors = FixtureJsonSchemaValidator.validate(&fixture_json, &schema_json);
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
    let schema_root = value.clone();
    inline_internal_schema_refs(&mut value, &schema_root)?;
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

fn inline_internal_schema_refs(value: &mut Value, schema_root: &Value) -> Result<()> {
    match value {
        Value::Object(object) => {
            if let Some(raw_ref) = object.get("$ref").and_then(Value::as_str) {
                if let Some(pointer) = raw_ref.strip_prefix('#') {
                    let mut referenced =
                        schema_root.pointer(pointer).cloned().ok_or_else(|| {
                            anyhow::anyhow!("unresolved schema reference `{raw_ref}`")
                        })?;
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
                    inline_internal_schema_refs(value, schema_root)?;
                    return Ok(());
                }
            }
            for nested in object.values_mut() {
                inline_internal_schema_refs(nested, schema_root)?;
            }
        }
        Value::Array(values) => {
            for nested in values {
                inline_internal_schema_refs(nested, schema_root)?;
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

fn sha256_hex(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let sigma0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let sigma1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(ROUND[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (state, value) in hash.iter_mut().zip([a, b, c, d, e, f, g, h].into_iter()) {
            *state = state.wrapping_add(value);
        }
    }

    hash.iter().map(|word| format!("{word:08x}")).collect()
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
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sdk-conformance-runner-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug)]
    struct StaticConformanceExecutor {
        failure: Option<String>,
        include_proof: bool,
        collected_override: Option<Vec<String>>,
    }

    impl StaticConformanceExecutor {
        fn passing() -> Self {
            Self {
                failure: None,
                include_proof: true,
                collected_override: None,
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                failure: Some(message.to_string()),
                include_proof: true,
                collected_override: None,
            }
        }

        fn without_proof() -> Self {
            Self {
                failure: None,
                include_proof: false,
                collected_override: None,
            }
        }

        fn with_collected(collected: &[&str]) -> Self {
            Self {
                failure: None,
                include_proof: true,
                collected_override: Some(
                    collected.iter().map(|value| (*value).to_string()).collect(),
                ),
            }
        }
    }

    impl ConformanceExecutor for StaticConformanceExecutor {
        fn execute(
            &self,
            _root: &Path,
            binding: &ExecutionBinding,
        ) -> Result<ConformanceExecution> {
            Ok(ConformanceExecution {
                proofs: self
                    .include_proof
                    .then(|| ConformanceExecutionProof {
                        phase: "execution",
                        command: vec![binding.language.clone(), binding.selector.clone()],
                        working_directory: ".".to_string(),
                        exit_code: if self.failure.is_some() { 1 } else { 0 },
                        output_sha256: sha256_hex(b"test command output"),
                    })
                    .into_iter()
                    .collect(),
                collected_tests: self
                    .collected_override
                    .clone()
                    .unwrap_or_else(|| vec![binding.selector.clone()]),
                failure: self.failure.clone(),
            })
        }
    }

    #[test]
    fn runner_accepts_repo_manifest_for_rust() {
        let root = repository_root();
        let records = run_manifest(&root, "rust", None).expect("runner manifest");

        assert!(!records.is_empty());
        assert!(records
            .iter()
            .all(|record| record.status != ConformanceStatus::Failed));
        assert!(records.iter().any(|record| {
            record.status == ConformanceStatus::Unsupported
                && record.error_code == Some("CAPABILITY_UNSUPPORTED")
        }));
    }

    #[test]
    fn runner_reports_missing_fixture_reference() {
        let root = TestDir::new();
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
        let root = TestDir::new();
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
    fn runner_validates_repository_conformance_reports() {
        let root = repository_root();
        for (language, report) in [
            (
                "rust",
                "sdk/conformance/runner/rust-runtime-conformance-report.json",
            ),
            (
                "c_abi",
                "sdk/conformance/runner/c-abi-runtime-conformance-report.json",
            ),
            (
                "go",
                "sdk/conformance/runner/go-runtime-conformance-report.json",
            ),
            (
                "python",
                "sdk/conformance/runner/python-runtime-conformance-report.json",
            ),
            (
                "node",
                "sdk/conformance/runner/node-runtime-conformance-report.json",
            ),
        ] {
            let report = root.join(report);
            let records = run_manifest_with_executor(
                &root,
                language,
                Some(&report),
                &StaticConformanceExecutor::passing(),
            )
            .expect("runner validates conformance report");

            let required: Vec<_> = records
                .iter()
                .filter(|record| {
                    record.language == language && record.status != ConformanceStatus::Unsupported
                })
                .collect();
            assert!(!required.is_empty(), "{language} must have required cases");
            assert!(
                required
                    .iter()
                    .all(|record| record.status == ConformanceStatus::Passed),
                "{language} conformance report must pass every required case"
            );
        }
    }

    #[test]
    fn runner_reports_missing_required_conformance_record() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("report.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": []
}"#,
        )
        .unwrap();
        fs::create_dir_all(root.path().join("sdk/go")).unwrap();
        fs::write(
            root.path().join("sdk/go/minimal_test.go"),
            "package minimal\n\nimport \"testing\"\n\nfunc TestMinimal(t *testing.T) {}\n",
        )
        .unwrap();
        write_execution_manifest(
            root.path(),
            "go",
            "test/minimal",
            "TestMinimal",
            &["sdk/go/minimal_test.go"],
        );

        let records = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].error_code, Some("CONFORMANCE_REPORT_MISSING"));
    }

    #[test]
    fn runner_reports_failed_conformance_execution() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "python");
        let report = root.path().join("report.json");
        let evidence_hash = sha256_hex(b"def test_minimal():\n    assert True\n");
        fs::write(
            &report,
            format!(r#"{{
  "schema_version": 2,
  "language": "python",
  "report_kind": "unit_test",
  "records": [
    {{
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "evidence": [{{"kind": "python_test", "ref_path": "sdk/python/tests/test_minimal.py", "sha256": "{evidence_hash}"}}]
    }}
  ]
}}"#),
        )
        .unwrap();
        fs::create_dir_all(root.path().join("sdk/python/tests")).unwrap();
        fs::write(
            root.path().join("sdk/python/tests/test_minimal.py"),
            "def test_minimal():\n    assert True\n",
        )
        .unwrap();
        write_execution_manifest(
            root.path(),
            "python",
            "test/minimal",
            "test_minimal",
            &["sdk/python/tests/test_minimal.py"],
        );

        let records = run_manifest_with_executor(
            root.path(),
            "python",
            Some(&report),
            &StaticConformanceExecutor::failing("forced command failure"),
        )
        .expect("runner manifest");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(
            records[0].error_code,
            Some("CONFORMANCE_REPORT_EXECUTION_FAILED")
        );
        assert!(records[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("forced command failure"));
    }

    #[test]
    fn runner_rejects_cross_language_conformance_evidence() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("report.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "evidence": [{"kind": "rust_test", "ref_path": "sdk/conformance/runner/README.md", "sha256": "unused"}]
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("cross-language conformance evidence must fail");

        assert!(err.to_string().contains("expected `go_test`"));
    }

    #[test]
    fn runner_rejects_unknown_conformance_record_case() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        let report = root.path().join("report.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [
    {
      "case_id": "test/unknown",
      "profile": "runtime_core",
      "evidence": [{"kind": "go_test", "ref_path": "sdk/go/unknown_test.go", "sha256": "unused"}]
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("unknown conformance case must fail");

        assert!(err.to_string().contains("does not match any manifest case"));
    }

    #[test]
    fn runner_rejects_language_undeclared_conformance_record_case() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "rust");
        let report = root.path().join("report.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [
    {
      "case_id": "test/minimal",
      "profile": "runtime_core",
      "evidence": [{"kind": "go_test", "ref_path": "sdk/go/minimal_test.go", "sha256": "unused"}]
    }
  ]
}"#,
        )
        .unwrap();

        let err = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("language-undeclared conformance case must fail");

        assert!(err
            .to_string()
            .contains("is not declared for language `go`"));
    }

    #[test]
    fn runner_rejects_committed_status_attestation() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        fs::create_dir_all(root.path().join("sdk/go")).unwrap();
        let evidence = root.path().join("sdk/go/minimal_test.go");
        fs::write(&evidence, "package minimal\n").unwrap();
        let evidence_hash = sha256_hex(b"package minimal\n");
        let report = root.path().join("report.json");
        fs::write(
            &report,
            format!(
                r#"{{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [{{
    "case_id": "test/minimal",
    "profile": "runtime_core",
    "status": "passed",
    "evidence": [{{"kind": "go_test", "ref_path": "sdk/go/minimal_test.go", "sha256": "{evidence_hash}"}}]
  }}]
}}"#
            ),
        )
        .unwrap();

        let error = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("committed status must be rejected");

        assert!(format!("{error:#}").contains("unknown field `status`"));
    }

    #[test]
    fn runner_rejects_report_supplied_argv() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());
        let mut document: Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        document["records"][0]["argv"] = serde_json::json!(["true"]);
        fs::write(&report, serde_json::to_vec(&document).unwrap()).unwrap();

        let error = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("reports must not provide execution commands");

        assert!(format!("{error:#}").contains("unknown field `argv`"));
    }

    #[test]
    fn runner_rejects_selector_reused_for_distinct_cases() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        fs::write(
            root.path().join("sdk/conformance/cases/second.yaml"),
            "id: test/second\nprofile: runtime_core\nrequired_for:\n  - go\nsteps:\n  - action: noop\nexpect:\n  result: ok\n",
        )
        .unwrap();
        fs::create_dir_all(root.path().join("sdk/go")).unwrap();
        fs::write(
            root.path().join("sdk/go/minimal_test.go"),
            "package minimal\nimport \"testing\"\nfunc TestMinimal(t *testing.T) {}\n",
        )
        .unwrap();
        fs::write(
            root.path().join("sdk/conformance/runner/execution-manifest.json"),
            r#"{"schema_version":1,"bindings":[
              {"language":"go","case_id":"test/minimal","selector":"TestMinimal","evidence":["sdk/go/minimal_test.go"]},
              {"language":"go","case_id":"test/second","selector":"TestMinimal","evidence":["sdk/go/minimal_test.go"]}
            ]}"#,
        )
        .unwrap();

        let error = ExecutionManifestIndex::load(
            root.path(),
            &load_cases(root.path()).expect("load cases"),
        )
        .expect_err("one selector must not prove two cases");
        assert!(error.to_string().contains("proves more than one case"));
    }

    #[test]
    fn c_abi_header_is_not_executable_test_evidence() {
        let error = validate_evidence_scope("c_abi", "include/easynet_cli.h")
            .expect_err("a production header cannot be cited as a collected test");

        assert!(error.to_string().contains("not covered"));
    }

    #[test]
    fn rust_selector_requires_test_attribute() {
        let helper = "fn helper() {}\n";
        let test = "#[test]\nfn actual_test() {}\n";

        assert!(!rust_test_attribute_precedes(
            helper,
            helper.find("fn helper").unwrap()
        ));
        assert!(rust_test_attribute_precedes(
            test,
            test.find("fn actual_test").unwrap()
        ));
    }

    #[test]
    fn runner_rejects_evidence_hash_mismatch_before_execution() {
        let root = TestDir::new();
        create_minimal_case_root(root.path(), "go");
        fs::create_dir_all(root.path().join("sdk/go")).unwrap();
        fs::write(
            root.path().join("sdk/go/minimal_test.go"),
            "package minimal\n",
        )
        .unwrap();
        let report = root.path().join("report.json");
        fs::write(
            &report,
            r#"{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [{
    "case_id": "test/minimal",
    "profile": "runtime_core",
    "evidence": [{"kind": "go_test", "ref_path": "sdk/go/minimal_test.go", "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"}]
  }]
}"#,
        )
        .unwrap();

        let error = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("mutated evidence must fail hash validation");

        assert!(error.to_string().contains("evidence hash mismatch"));
    }

    #[test]
    fn runner_rejects_conformance_report_without_executor() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());

        let error = run_manifest(root.path(), "go", Some(&report))
            .expect_err("report mode must require an executor");

        assert!(error.to_string().contains("execution is mandatory"));
    }

    #[test]
    fn runner_rejects_empty_execution_proof() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());

        let records = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::without_proof(),
        )
        .expect("runner result");

        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(
            records[0].error_code,
            Some("CONFORMANCE_REPORT_EXECUTION_MISSING")
        );
        assert!(records[0].executions.is_empty());
    }

    #[test]
    fn runner_rejects_uncollected_selector() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());

        let records = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::with_collected(&[]),
        )
        .expect("runner result");

        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert!(records[0]
            .message
            .as_deref()
            .unwrap()
            .contains("not collected exactly once"));
    }

    #[test]
    fn node_tap_selector_parser_requires_one_exact_non_skipped_pass() {
        let selector = "conformance version compatible accepts exact ABI";
        let output = format!(
            "TAP version 13\n\
             # Subtest: {selector}\n\
             ok 1 - {selector}\n\
             ok 2 - {selector} extended # SKIP test name does not match pattern\n\
             ok 3 - unrelated # SKIP test name does not match pattern\n\
             1..3\n"
        );
        assert_eq!(node_tap_selector_pass_count(&output, selector), 1);
        assert_eq!(
            node_tap_selector_pass_count(
                &format!("ok 1 - {selector} # SKIP disabled by policy\n"),
                selector
            ),
            0
        );
    }

    #[test]
    fn runner_rejects_unrelated_collected_test() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());

        let records = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::with_collected(&["TestUnrelated"]),
        )
        .expect("runner result");

        assert_eq!(records[0].status, ConformanceStatus::Failed);
        assert_eq!(records[0].collected_tests, ["TestUnrelated"]);
    }

    #[test]
    fn runner_rejects_selector_absent_from_bound_evidence() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());
        write_execution_manifest(
            root.path(),
            "go",
            "test/minimal",
            "TestUnrelated",
            &["sdk/go/minimal_test.go"],
        );

        let error = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect_err("unrelated selector must fail before execution");

        assert!(error.to_string().contains("must be declared exactly once"));
    }

    #[test]
    fn runner_binds_case_digest_evidence_and_execution_result() {
        let root = TestDir::new();
        let report = create_executable_go_case(root.path());

        let records = run_manifest_with_executor(
            root.path(),
            "go",
            Some(&report),
            &StaticConformanceExecutor::passing(),
        )
        .expect("runner result");

        let record = &records[0];
        assert_eq!(record.status, ConformanceStatus::Passed);
        assert_eq!(record.selector.as_deref(), Some("TestMinimal"));
        assert_eq!(record.collected_tests, ["TestMinimal"]);
        assert_eq!(record.case_sha256.len(), 64);
        assert_eq!(record.evidence.len(), 1);
        assert_eq!(record.evidence[0].sha256.len(), 64);
        assert_eq!(record.attestation_sha256.as_deref().map(str::len), Some(64));
        assert!(!record.executions.is_empty());
        assert!(record
            .executions
            .iter()
            .any(|proof| proof.phase == "execution"));
    }

    #[test]
    fn attestation_changes_with_case_evidence_or_command_result() {
        let case = ConformanceCase {
            id: "test/minimal".to_string(),
            profile: "runtime_core".to_string(),
            required_for: BTreeSet::from(["go".to_string()]),
            document: Value::Null,
            sha256: "a".repeat(64),
        };
        let binding = ExecutionBinding {
            language: "go".to_string(),
            case_id: case.id.clone(),
            selector: "TestMinimal".to_string(),
            evidence: vec!["sdk/go/minimal_test.go".to_string()],
        };
        let report_record = ConformanceReportRecord {
            case_id: case.id.clone(),
            profile: case.profile.clone(),
            evidence: vec![ConformanceEvidence {
                kind: "go_test".to_string(),
                ref_path: binding.evidence[0].clone(),
                sha256: "b".repeat(64),
            }],
            coverage: None,
        };
        let execution = StaticConformanceExecutor::passing()
            .execute(Path::new("."), &binding)
            .unwrap();
        let baseline =
            conformance_attestation_sha256(&case, "go", &binding, &report_record, &execution);

        let mut changed_case = case.clone();
        changed_case.sha256 = "c".repeat(64);
        assert_ne!(
            baseline,
            conformance_attestation_sha256(
                &changed_case,
                "go",
                &binding,
                &report_record,
                &execution
            )
        );
        let mut changed_report_record = report_record.clone();
        changed_report_record.evidence[0].sha256 = "d".repeat(64);
        assert_ne!(
            baseline,
            conformance_attestation_sha256(
                &case,
                "go",
                &binding,
                &changed_report_record,
                &execution
            )
        );
        let mut changed_execution = execution.clone();
        changed_execution.proofs[0].output_sha256 = "e".repeat(64);
        assert_ne!(
            baseline,
            conformance_attestation_sha256(
                &case,
                "go",
                &binding,
                &report_record,
                &changed_execution
            )
        );
    }

    #[test]
    fn git_inventory_paths_are_sorted_for_stable_tree_attestation() {
        let first = git_inventory_paths(b"zeta\0alpha\0middle\0");
        let second = git_inventory_paths(b"middle\0zeta\0alpha\0");

        assert_eq!(first, second);
        assert_eq!(
            first
                .iter()
                .map(|path| std::str::from_utf8(path).unwrap())
                .collect::<Vec<_>>(),
            ["alpha", "middle", "zeta"]
        );
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("runner crate is nested under tools")
            .to_path_buf()
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

    fn create_executable_go_case(root: &Path) -> PathBuf {
        create_minimal_case_root(root, "go");
        fs::create_dir_all(root.join("sdk/go")).unwrap();
        let evidence_path = "sdk/go/minimal_test.go";
        let evidence =
            b"package minimal\n\nimport \"testing\"\n\nfunc TestMinimal(t *testing.T) {}\n";
        fs::write(root.join(evidence_path), evidence).unwrap();
        write_execution_manifest(root, "go", "test/minimal", "TestMinimal", &[evidence_path]);
        let report = root.join("report.json");
        fs::write(
            &report,
            format!(
                r#"{{
  "schema_version": 2,
  "language": "go",
  "report_kind": "unit_test",
  "records": [{{
    "case_id": "test/minimal",
    "profile": "runtime_core",
    "evidence": [{{
      "kind": "go_test",
      "ref_path": "{evidence_path}",
      "sha256": "{}"
    }}]
  }}]
}}"#,
                sha256_hex(evidence)
            ),
        )
        .unwrap();
        report
    }

    fn write_execution_manifest(
        root: &Path,
        language: &str,
        case_id: &str,
        selector: &str,
        evidence: &[&str],
    ) {
        let evidence = evidence
            .iter()
            .map(|path| format!(r#""{path}""#))
            .collect::<Vec<_>>()
            .join(", ");
        fs::write(
            root.join("sdk/conformance/runner/execution-manifest.json"),
            format!(
                r#"{{
  "schema_version": 1,
  "bindings": [{{
    "language": "{language}",
    "case_id": "{case_id}",
    "selector": "{selector}",
    "evidence": [{evidence}]
  }}]
}}"#
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
