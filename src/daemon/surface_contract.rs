// EasyNet CLI — Surface shared contract
// ======================================
//
// File: src/daemon/surface_contract.rs
// Description: Shared daemon SDK contract for Surface page carriers and DTO
//              projections.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Surface profile semantics for daemon-governed page
// abilities. This module builds complete Invocation carriers for existing
// `pages.*` abilities and projects daemon page facts into stable SDK DTOs. It
// does not render HTML, own backend public routes, manage CDN policy, or
// replace daemon page state.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for `pages.list`,
// `pages.publish`, `pages.get`, and `pages.unpublish`. Keep page identity
// derivation explicit: record projection needs a page id plus either direct
// refs or enough daemon facts to derive canonical URAs through Axon helpers.
//
// Usage Contract
// --------------
// Carrier construction requires explicit Invocation tuple fields. Page create
// requests require an absolute folder path and bounded project id. Projection
// accepts object-shaped daemon facts and rejects missing identity anchors.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Surface profile. Runtime Core remains the submit path
// for returned carriers; EasyNet backend remains the renderer and public HTTP
// product surface owner.

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::core::ura;
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_string_field, required_string, validate_ura,
    SdkContractError,
};

const SURFACE_PROFILE: &str = "surface";
const ABILITY_PAGES_LIST: &str = "pages.list";
const ABILITY_PAGES_PUBLISH: &str = "pages.publish";
const ABILITY_PAGES_GET: &str = "pages.get";
const ABILITY_PAGES_UNPUBLISH: &str = "pages.unpublish";

pub(crate) const SURFACE_DEFAULT_PAGE_SIZE: usize = 50;
pub(crate) const SURFACE_MAX_PAGE_SIZE: usize = 500;

pub(crate) type SurfaceError = SdkContractError;

pub(crate) fn build_list_pages_invocation(request: &Value) -> Result<Value, SurfaceError> {
    let obj = object(request, "SurfaceListPagesRequest")?;
    reject_unsupported_fields(obj, SURFACE_LIST_REQUEST_FIELDS)?;
    let _ = PageControls::from_request(obj)?;
    build_system_invocation(obj, SURFACE_PROFILE, ABILITY_PAGES_LIST, json!({}))
}

pub(crate) fn build_create_page_invocation(request: &Value) -> Result<Value, SurfaceError> {
    let obj = object(request, "SurfaceCreatePageRequest")?;
    reject_unsupported_fields(obj, SURFACE_CREATE_REQUEST_FIELDS)?;
    let project_id = required_string(obj, "project_id")?;
    validate_project_id(project_id)?;
    let folder = required_string(obj, "folder")?;
    validate_absolute_folder(folder)?;
    let mut args = Map::new();
    args.insert(
        "project_id".to_string(),
        Value::String(project_id.to_string()),
    );
    args.insert("folder".to_string(), Value::String(folder.to_string()));
    if let Some(visibility) = optional_string_field(obj, "visibility")? {
        validate_visibility(&visibility)?;
        args.insert("visibility".to_string(), Value::String(visibility));
    }
    build_system_invocation(
        obj,
        SURFACE_PROFILE,
        ABILITY_PAGES_PUBLISH,
        Value::Object(args),
    )
}

pub(crate) fn build_delete_page_invocation(request: &Value) -> Result<Value, SurfaceError> {
    let obj = object(request, "SurfaceDeletePageRequest")?;
    reject_unsupported_fields(obj, SURFACE_PROJECT_REQUEST_FIELDS)?;
    let args = project_id_args(obj)?;
    build_system_invocation(obj, SURFACE_PROFILE, ABILITY_PAGES_UNPUBLISH, args)
}

pub(crate) fn build_manifest_invocation(request: &Value) -> Result<Value, SurfaceError> {
    let obj = object(request, "SurfaceManifestRequest")?;
    reject_unsupported_fields(obj, SURFACE_PROJECT_REQUEST_FIELDS)?;
    let args = project_id_args(obj)?;
    build_system_invocation(obj, SURFACE_PROFILE, ABILITY_PAGES_GET, args)
}

pub(crate) fn project_page_record(input: &Value) -> Result<Value, SurfaceError> {
    PageFacts::from_value(input, ProjectionHints::default())?.record_json()
}

pub(crate) fn project_page_page(input: &Value) -> Result<Value, SurfaceError> {
    let page_input = PageInput::parse(input)?;
    let rows = rows_from_value(page_input.result, "projects", "items", "SurfacePageRows")?;
    let page = page_input.controls.slice(rows)?;
    let mut items = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        items.push(PageFacts::from_value(row, page_input.hints.clone())?.record_json()?);
    }
    Ok(json!({
        "profile": SURFACE_PROFILE,
        "kind": "surface_page_page",
        "item_kind": "page_record",
        "items": items,
        "next_cursor": page.next_cursor,
        "limit": page_input.controls.limit,
        "source": "pages_read_model",
        "metadata": {
            "profile": SURFACE_PROFILE,
            "source_ability": ABILITY_PAGES_LIST,
            "page_size_default": SURFACE_DEFAULT_PAGE_SIZE,
            "page_size_max": SURFACE_MAX_PAGE_SIZE,
            "total_available": rows.len(),
        },
    }))
}

pub(crate) fn project_public_page_ref(input: &Value) -> Result<Value, SurfaceError> {
    PageFacts::from_value(input, ProjectionHints::default())?.public_ref_json()
}

pub(crate) fn project_surface_manifest(input: &Value) -> Result<Value, SurfaceError> {
    let facts = PageFacts::from_value(input, ProjectionHints::default())?;
    let page = facts.record_json()?;
    let public_ref = facts.public_ref_value()?;
    Ok(json!({
        "profile": SURFACE_PROFILE,
        "kind": "surface_manifest",
        "page_id": facts.page_id,
        "owner_ura": facts.owner_ura,
        "surface_ref": facts.surface_ref,
        "public_ref": public_ref,
        "page": page,
        "entrypoint": {
            "kind": "public_page_ref",
            "href": public_ref,
        },
        "metadata": {
            "profile": SURFACE_PROFILE,
            "source_ability": ABILITY_PAGES_GET,
            "raw_page": facts.raw,
        },
    }))
}

pub(crate) fn project_mutation_result(input: &Value) -> Result<Value, SurfaceError> {
    let obj = object(input, "SurfaceMutationResult")?;
    let project_id = required_string(obj, "project_id")?;
    validate_project_id(project_id)?;
    let removed = obj.get("removed").and_then(Value::as_bool).unwrap_or(false);
    Ok(json!({
        "profile": SURFACE_PROFILE,
        "kind": "surface_mutation_result",
        "operation": optional_string_field(obj, "operation")?.unwrap_or_else(|| "delete".to_string()),
        "page_id": project_id,
        "removed": removed,
        "state": if removed { "deleted" } else { "unknown" },
        "metadata": {
            "profile": SURFACE_PROFILE,
            "source_ability": ABILITY_PAGES_UNPUBLISH,
            "raw_result": input,
        },
    }))
}

const COMMON_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "limit",
    "cursor",
];
const SURFACE_LIST_REQUEST_FIELDS: &[&str] = COMMON_REQUEST_FIELDS;
const SURFACE_CREATE_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "project_id",
    "folder",
    "visibility",
];
const SURFACE_PROJECT_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "project_id",
];

fn reject_unsupported_fields(
    obj: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), SurfaceError> {
    for key in obj.keys() {
        if !allowed.iter().any(|allowed| allowed == key) {
            return Err(SurfaceError::InvalidField(
                "request",
                format!("unsupported field `{key}`"),
            ));
        }
    }
    Ok(())
}

fn project_id_args(obj: &Map<String, Value>) -> Result<Value, SurfaceError> {
    let project_id = required_string(obj, "project_id")?;
    validate_project_id(project_id)?;
    Ok(json!({ "project_id": project_id }))
}

fn validate_project_id(project_id: &str) -> Result<(), SurfaceError> {
    if project_id.is_empty() {
        return Err(SurfaceError::InvalidField(
            "project_id",
            "must not be empty".to_string(),
        ));
    }
    if project_id.len() > 64 {
        return Err(SurfaceError::InvalidField(
            "project_id",
            format!("must be at most 64 bytes, got {}", project_id.len()),
        ));
    }
    if !project_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(SurfaceError::InvalidField(
            "project_id",
            "may contain only ASCII letters, digits, `_`, or `-`".to_string(),
        ));
    }
    Ok(())
}

fn validate_absolute_folder(folder: &str) -> Result<(), SurfaceError> {
    let path = Path::new(folder);
    if !path.is_absolute() {
        return Err(SurfaceError::InvalidField(
            "folder",
            "must be an absolute path".to_string(),
        ));
    }
    Ok(())
}

fn validate_visibility(visibility: &str) -> Result<(), SurfaceError> {
    match visibility {
        "public" | "private" => Ok(()),
        other => Err(SurfaceError::InvalidField(
            "visibility",
            format!("unsupported page visibility {other:?}"),
        )),
    }
}

#[derive(Debug, Clone, Default)]
struct ProjectionHints {
    owner_ura: Option<String>,
    realm: Option<String>,
}

#[derive(Debug, Clone)]
struct PageFacts {
    page_id: String,
    owner_ura: String,
    surface_ref: String,
    public_ref: Option<String>,
    status: Option<String>,
    raw: Value,
    metadata: Map<String, Value>,
}

impl PageFacts {
    fn from_value(input: &Value, hints: ProjectionHints) -> Result<Self, SurfaceError> {
        let obj = object(input, "SurfacePageRecord")?;
        let page_id = optional_string_field(obj, "page_id")?
            .or_else(|| optional_string_field(obj, "project_id").ok().flatten())
            .or_else(|| optional_string_field(obj, "id").ok().flatten())
            .ok_or(SurfaceError::MissingField("page_id"))?;
        validate_project_id(&page_id)?;
        let user = optional_string_field(obj, "user")?;
        let surface_ref = surface_ref(obj, user.as_deref(), &page_id, hints.realm.as_deref())?;
        let owner_ura = owner_ura(
            obj,
            hints.owner_ura.as_deref(),
            user.as_deref(),
            &surface_ref,
        )?;
        let public_ref = optional_string_field(obj, "public_ref")?
            .or_else(|| optional_string_field(obj, "url_root").ok().flatten())
            .or_else(|| optional_string_field(obj, "public_url").ok().flatten());
        if let Some(public_ref) = public_ref.as_deref() {
            validate_public_ref(public_ref)?;
        }
        let status = optional_string_field(obj, "status")?.or_else(|| {
            optional_string_field(obj, "visibility")
                .ok()
                .flatten()
                .map(|visibility| {
                    if visibility == "public" || visibility == "private" {
                        "published".to_string()
                    } else {
                        visibility
                    }
                })
        });
        let mut metadata = obj
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for key in [
            "user",
            "project_id",
            "folder",
            "visibility",
            "started_at_ms",
            "dev_listener_url_root",
            "file_size_cap",
        ] {
            if let Some(value) = obj.get(key) {
                metadata.insert(key.to_string(), value.clone());
            }
        }
        metadata.insert(
            "profile".to_string(),
            Value::String(SURFACE_PROFILE.to_string()),
        );
        metadata.insert(
            "source_ability".to_string(),
            Value::String(ABILITY_PAGES_GET.to_string()),
        );
        metadata.insert("raw_page".to_string(), input.clone());
        Ok(Self {
            page_id,
            owner_ura,
            surface_ref,
            public_ref,
            status,
            raw: input.clone(),
            metadata,
        })
    }

    fn record_json(&self) -> Result<Value, SurfaceError> {
        validate_ura(&self.owner_ura, "owner_ura")?;
        validate_ura(&self.surface_ref, "surface_ref")?;
        Ok(json!({
            "profile": SURFACE_PROFILE,
            "kind": "page_record",
            "page_id": self.page_id,
            "owner_ura": self.owner_ura,
            "surface_ref": self.surface_ref,
            "public_ref": self.public_ref,
            "status": self.status,
            "metadata": self.metadata,
        }))
    }

    fn public_ref_value(&self) -> Result<&str, SurfaceError> {
        self.public_ref
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(SurfaceError::MissingField("public_ref"))
    }

    fn public_ref_json(self) -> Result<Value, SurfaceError> {
        let public_ref = self.public_ref_value()?.to_string();
        Ok(json!({
            "profile": SURFACE_PROFILE,
            "kind": "public_page_ref",
            "page_id": self.page_id,
            "owner_ura": self.owner_ura,
            "surface_ref": self.surface_ref,
            "public_ref": public_ref,
            "route_kind": "hub_web",
            "metadata": {
                "profile": SURFACE_PROFILE,
                "source_ability": ABILITY_PAGES_GET,
                "raw_page": self.raw,
            },
        }))
    }
}

fn surface_ref(
    obj: &Map<String, Value>,
    user: Option<&str>,
    page_id: &str,
    realm_hint: Option<&str>,
) -> Result<String, SurfaceError> {
    if let Some(surface_ref) = optional_string_field(obj, "surface_ref")?
        .or_else(|| optional_string_field(obj, "project_ura").ok().flatten())
        .or_else(|| optional_string_field(obj, "resource_ura").ok().flatten())
    {
        validate_ura(&surface_ref, "surface_ref")?;
        return Ok(surface_ref);
    }
    let Some(user) = user else {
        return Err(SurfaceError::MissingField("surface_ref"));
    };
    let Some(realm) =
        optional_string_field(obj, "realm")?.or_else(|| realm_hint.map(str::to_string))
    else {
        return Err(SurfaceError::MissingField("realm"));
    };
    validate_realm(&realm)?;
    Ok(ura::resource_dot_ura(
        &realm,
        &format!("{user}.{page_id}"),
        "/",
    ))
}

fn owner_ura(
    obj: &Map<String, Value>,
    owner_hint: Option<&str>,
    user: Option<&str>,
    surface_ref: &str,
) -> Result<String, SurfaceError> {
    if let Some(owner_ura) = optional_string_field(obj, "owner_ura")? {
        validate_ura(&owner_ura, "owner_ura")?;
        return Ok(owner_ura);
    }
    if let Some(owner_ura) = owner_hint {
        validate_ura(owner_ura, "owner_ura")?;
        return Ok(owner_ura.to_string());
    }
    let Some(user) = user else {
        return Err(SurfaceError::MissingField("owner_ura"));
    };
    let parsed = ura::parse_ura(surface_ref)
        .map_err(|err| SurfaceError::InvalidField("surface_ref", err.to_string()))?;
    Ok(ura::agent_ura(&parsed.realm, user, "pages"))
}

fn validate_public_ref(public_ref: &str) -> Result<(), SurfaceError> {
    if public_ref.starts_with("https://") || public_ref.starts_with("http://127.0.0.1") {
        return Ok(());
    }
    if public_ref.starts_with("http://") && public_ref.contains(".pages.localhost") {
        return Ok(());
    }
    Err(SurfaceError::InvalidField(
        "public_ref",
        "must be an https URL or daemon-local pages localhost URL".to_string(),
    ))
}

fn validate_realm(realm: &str) -> Result<(), SurfaceError> {
    if realm.trim().is_empty()
        || realm.contains('/')
        || realm.contains('\\')
        || realm.chars().any(char::is_whitespace)
    {
        return Err(SurfaceError::InvalidField(
            "realm",
            "must be a non-empty realm token".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct PageControls {
    limit: usize,
    offset: usize,
}

impl PageControls {
    fn from_request(obj: &Map<String, Value>) -> Result<Self, SurfaceError> {
        let limit = optional_usize(obj, "limit")?.unwrap_or(SURFACE_DEFAULT_PAGE_SIZE);
        validate_limit(limit)?;
        let offset = optional_cursor_offset(obj, "cursor")?.unwrap_or(0);
        Ok(Self { limit, offset })
    }

    fn slice<'a, T>(&self, rows: &'a [T]) -> Result<PageSlice<'a, T>, SurfaceError> {
        if self.offset > rows.len() {
            return Err(SurfaceError::InvalidField(
                "cursor",
                "must not point past the current page snapshot".to_string(),
            ));
        }
        let end = self.offset.saturating_add(self.limit).min(rows.len());
        let next_cursor = if end < rows.len() {
            Some(end.to_string())
        } else {
            None
        };
        Ok(PageSlice {
            rows: &rows[self.offset..end],
            next_cursor,
        })
    }
}

struct PageSlice<'a, T> {
    rows: &'a [T],
    next_cursor: Option<String>,
}

struct PageInput<'a> {
    result: &'a Value,
    controls: PageControls,
    hints: ProjectionHints,
}

impl<'a> PageInput<'a> {
    fn parse(input: &'a Value) -> Result<Self, SurfaceError> {
        let Some(obj) = input.as_object() else {
            return Ok(Self {
                result: input,
                controls: PageControls {
                    limit: SURFACE_DEFAULT_PAGE_SIZE,
                    offset: 0,
                },
                hints: ProjectionHints::default(),
            });
        };
        let hints = ProjectionHints {
            owner_ura: optional_string_field(obj, "owner_ura")?,
            realm: optional_string_field(obj, "realm")?,
        };
        if let Some(owner_ura) = hints.owner_ura.as_deref() {
            validate_ura(owner_ura, "owner_ura")?;
        }
        if let Some(realm) = hints.realm.as_deref() {
            validate_realm(realm)?;
        }
        if let Some(result) = obj.get("result").filter(|value| !value.is_null()) {
            return Ok(Self {
                result,
                controls: PageControls::from_request(obj)?,
                hints,
            });
        }
        Ok(Self {
            result: input,
            controls: PageControls::from_request(obj)?,
            hints,
        })
    }
}

fn rows_from_value<'a>(
    value: &'a Value,
    primary: &'static str,
    fallback: &'static str,
    name: &'static str,
) -> Result<&'a Vec<Value>, SurfaceError> {
    if let Some(rows) = value.as_array() {
        return Ok(rows);
    }
    let obj = object(value, name)?;
    obj.get(primary)
        .or_else(|| obj.get(fallback))
        .or_else(|| obj.get("pages"))
        .and_then(Value::as_array)
        .ok_or(SurfaceError::MissingField(primary))
}

fn validate_limit(limit: usize) -> Result<(), SurfaceError> {
    if limit == 0 || limit > SURFACE_MAX_PAGE_SIZE {
        return Err(SurfaceError::InvalidField(
            "limit",
            format!("must be between 1 and {SURFACE_MAX_PAGE_SIZE}"),
        ));
    }
    Ok(())
}

fn optional_usize(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, SurfaceError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| SurfaceError::InvalidField(field, "must be unsigned".to_string())),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| SurfaceError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(SurfaceError::InvalidField(
            field,
            "must be an integer or decimal string".to_string(),
        )),
    }
}

fn optional_cursor_offset(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, SurfaceError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.starts_with('-') || !trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(SurfaceError::InvalidField(
                    field,
                    "must be a non-negative decimal offset cursor".to_string(),
                ));
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| SurfaceError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(SurfaceError::InvalidField(
            field,
            "must be a cursor string".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/agent/alice.pages",
            "subject_ura": "easynet:///r/example/agent/alice.pages",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "surface-1"}
        });
        let Value::Object(extra) = extra else {
            return request;
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        request
    }

    fn page_fact() -> Value {
        json!({
            "user": "alice",
            "project_id": "docs",
            "project_ura": "easynet:///r/example/resource/alice.docs",
            "url_root": "https://example/web/alice/docs/",
            "visibility": "public",
            "folder": "/tmp/docs"
        })
    }

    #[test]
    fn build_create_page_invocation_targets_pages_publish() {
        let request = base_request(json!({
            "project_id": "docs",
            "folder": "/tmp/docs",
            "visibility": "public"
        }));

        let invocation = build_create_page_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_PAGES_PUBLISH
        );
        assert_eq!(invocation["args"]["project_id"], "docs");
        assert_eq!(invocation["args"]["folder"], "/tmp/docs");
        assert_eq!(
            invocation["descriptor_ref"],
            "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0"
        );
    }

    #[test]
    fn build_create_page_rejects_relative_folder() {
        let request = base_request(json!({
            "project_id": "docs",
            "folder": "docs",
        }));

        let err = build_create_page_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("folder"));
    }

    #[test]
    fn build_list_and_manifest_target_pages_abilities() {
        let list = build_list_pages_invocation(&base_request(json!({}))).unwrap();
        let manifest =
            build_manifest_invocation(&base_request(json!({"project_id": "docs"}))).unwrap();

        assert_eq!(list["metadata"]["system_ability"], ABILITY_PAGES_LIST);
        assert_eq!(manifest["metadata"]["system_ability"], ABILITY_PAGES_GET);
        assert_eq!(manifest["args"], json!({"project_id": "docs"}));
    }

    #[test]
    fn build_delete_targets_pages_unpublish() {
        let invocation =
            build_delete_page_invocation(&base_request(json!({"project_id": "docs"}))).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_PAGES_UNPUBLISH
        );
        assert_eq!(invocation["args"], json!({"project_id": "docs"}));
    }

    #[test]
    fn project_page_record_derives_owner_from_project_ura() {
        let record = project_page_record(&page_fact()).unwrap();

        assert_eq!(record["page_id"], "docs");
        assert_eq!(
            record["owner_ura"],
            "easynet:///r/example/agent/alice.pages"
        );
        assert_eq!(
            record["surface_ref"],
            "easynet:///r/example/resource/alice.docs"
        );
        assert_eq!(record["public_ref"], "https://example/web/alice/docs/");
        assert_eq!(record["status"], "published");
    }

    #[test]
    fn project_page_page_applies_cursor_pagination_with_hints() {
        let page = project_page_page(&json!({
            "owner_ura": "easynet:///r/example/agent/alice.pages",
            "realm": "example",
            "limit": 1,
            "result": {
                "projects": [
                    {"user": "alice", "project_id": "docs", "url_root": "https://example/web/alice/docs/"},
                    {"user": "alice", "project_id": "blog", "url_root": "https://example/web/alice/blog/"}
                ]
            }
        }))
        .unwrap();

        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            page["items"][0]["surface_ref"],
            "easynet:///r/example/resource/alice.docs"
        );
        assert_eq!(page["next_cursor"], "1");
    }

    #[test]
    fn public_page_ref_requires_public_ref() {
        let err = project_public_page_ref(&json!({
            "user": "alice",
            "project_id": "docs",
            "project_ura": "easynet:///r/example/resource/alice.docs"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("public_ref"));
    }

    #[test]
    fn project_manifest_wraps_page_and_entrypoint() {
        let manifest = project_surface_manifest(&page_fact()).unwrap();

        assert_eq!(manifest["kind"], "surface_manifest");
        assert_eq!(manifest["page"]["page_id"], "docs");
        assert_eq!(
            manifest["entrypoint"]["href"],
            "https://example/web/alice/docs/"
        );
    }

    #[test]
    fn project_mutation_result_projects_delete_state() {
        let result = project_mutation_result(&json!({
            "user": "alice",
            "project_id": "docs",
            "removed": true
        }))
        .unwrap();

        assert_eq!(result["page_id"], "docs");
        assert_eq!(result["state"], "deleted");
    }
}
