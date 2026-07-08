import { createHash } from "node:crypto";

const ERROR_CODES = new Set([
  "INVALID_ARGUMENT",
  "INVALID_HANDLE",
  "NULL_POINTER",
  "INVALID_UTF8",
  "NOT_INITIALIZED",
  "ALREADY_INIT",
  "DAEMON_OFFLINE",
  "PERMISSION_DENIED",
  "ADMISSION_DENIED",
  "ABILITY_NOT_FOUND",
  "ROUTE_UNAVAILABLE",
  "TIMEOUT",
  "CANCELLED",
  "INVALID_INVOCATION",
  "PROTOCOL_MISMATCH",
  "VERSION_MISMATCH",
  "VERSION_INCOMPATIBLE",
  "CONTROL_ONLY",
  "TRANSPORT",
  "PROTOCOL",
  "NOT_FOUND",
  "ABILITY_FAILED",
  "NOT_IMPLEMENTED",
  "GENERIC",
]);

export const ErrorCode = Object.freeze(
  Object.fromEntries([...ERROR_CODES].map((code) => [code, code])),
);

export const ErrorClass = Object.freeze({
  VALIDATION: "validation",
  HANDLE: "handle",
  LIFECYCLE: "lifecycle",
  AVAILABILITY: "availability",
  PERMISSION: "permission",
  ADMISSION: "admission",
  ROUTING: "routing",
  TIMEOUT: "timeout",
  CANCELLATION: "cancellation",
  PROTOCOL: "protocol",
  VERSION: "version",
  CONTROL: "control",
  UNSUPPORTED: "unsupported",
  GENERIC: "generic",
});

export const RetryHint = Object.freeze({
  NEVER: "never",
  SAFE: "safe",
  AFTER_BACKOFF: "after_backoff",
  UNKNOWN: "unknown",
});

export const DEFAULT_DIRECTORY_PAGE_SIZE = 50;
export const MAX_DIRECTORY_PAGE_SIZE = 500;
export const DIRECTORY_IDENTITY_PROFILE = "directory_identity";
export const RECEIPT_PROFILE = "receipt";
export const PUBLICATION_PROFILE = "publication";
export const DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE = 50;
export const MAX_PUBLISHED_ABILITY_PAGE_SIZE = 500;
export const HOST_BINDING_PROFILE = "host_binding";
export const HEALTH_PROFILE = "health";
export const AUTHORITY_PROFILE = "authority";
export const DELEGATION_METADATA_KEY = "x-easynet-delegation";
export const SESSION_AUTHORITY_METADATA_KEY = "x-easynet-session-authority";
export const EVENTS_PROFILE = "events";
export const MISSION_PROFILE = "mission";
export const ADMIN_GATEWAY_PROFILE = "admin_gateway";
export const WRAPPERS_PROFILE = "wrappers";
export const SURFACE_PROFILE = "surface";
export const COMPATIBILITY_PROFILE = "compatibility";
export const MAX_STREAM_BUFFERED_EVENTS = 1024;
export const MAX_BIDI_BUFFERED_FRAMES = 1024;
export const DEFAULT_EVENT_PAGE_SIZE = 50;
export const MAX_EVENT_PAGE_SIZE = 500;
export const MIN_EVENT_HEARTBEAT_INTERVAL_MS = 1000;
export const MAX_EVENT_HEARTBEAT_INTERVAL_MS = 300000;
export const DEFAULT_SURFACE_PAGE_SIZE = 50;
export const MAX_SURFACE_PAGE_SIZE = 500;
export const HOST_STREAM_FRAME_SCHEMA = "host-stream-frame.schema.json";
export const HOST_STREAM_HASH_ALGORITHM = "sha256(prev_hash || seq_be || canonical_json(value))";
export const HOST_STREAM_EMPTY_OUTPUT_HASH =
  "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

export class SDKError extends Error {
  constructor({
    code,
    stage,
    retry = RetryHint.NEVER,
    retryable = retry === RetryHint.SAFE || retry === RetryHint.AFTER_BACKOFF,
    message,
    source = "",
    invocationId = "",
    receiptURA = "",
    details = {},
    cause,
  }) {
    super(message || String(code), { cause });
    this.name = "SDKError";
    this.code = normalizeErrorCode(code);
    this.stage = requiredWireString(stage, "stage");
    this.retry = parseRetryHint(retry);
    this.retryable = Boolean(retryable);
    this.source = source || "";
    this.invocationId = invocationId || "";
    this.receiptURA = receiptURA || "";
    this.details = objectValue(details, "details");
  }

  static fromJSON(raw) {
    const text = decodeText(raw);
    if (text.trim() === "null") {
      return null;
    }
    const decoded = parseJSON(text, "daemon error");
    return new SDKError({
      code: requiredWireString(decoded.code, "code"),
      stage: requiredWireString(decoded.stage, "stage"),
      retry: requiredWireString(decoded.retry, "retry"),
      message: stringValue(decoded.message, "message", true),
      source: stringValue(decoded.source, "source", true),
      invocationId: stringValue(decoded.invocation_id, "invocation_id", true),
      receiptURA: stringValue(decoded.receipt_ura, "receipt_ura", true),
      details: decoded.details ?? {},
    });
  }

  errorClass() {
    return errorClassForCode(this.code);
  }

  profile() {
    return detailString(this.details, "profile");
  }

  sourceRef() {
    return detailString(this.details, "source_ref");
  }
}

export class Client {
  constructor(transport) {
    if (!transport || typeof transport.featureDiscovery !== "function") {
      throw invalidSDK("feature discovery transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async featureDiscovery() {
    const transport = this.requireOpen();
    const decoded = parseJSON(await transport.featureDiscovery(), "feature discovery");
    const abiVersion = nonNegativeInteger(decoded.abi_version, "abi_version");
    const sdkVersion = stringValue(decoded.sdk_version ?? "", "sdk_version", true);
    return {
      abiVersion,
      sdkVersion,
      profiles: stringMap(decoded.profiles ?? {}, "profiles"),
      symbols: boolMap(decoded.symbols ?? {}, "symbols"),
      axonPB: booleanValue(decoded.axon_pb ?? false, "axon_pb"),
      version() {
        return { abiVersion, sdkVersion };
      },
    };
  }

  async requireABI(expected) {
    if (!Number.isInteger(expected) || expected <= 0) {
      throw invalidSDK("expected ABI version must be positive");
    }
    const features = await this.featureDiscovery();
    if (features.abiVersion !== expected) {
      throw new SDKError({
        code: ErrorCode.VERSION_MISMATCH,
        stage: "sdk",
        retry: RetryHint.NEVER,
        message: `daemon ABI version ${features.abiVersion} does not match expected ${expected}`,
      });
    }
    return features;
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("client is closed");
    }
    return this.transport;
  }
}

export class RuntimeHealth {
  constructor(fields) {
    const value = objectValue(fields, "runtime health");
    this.apiReady = requiredHealthBoolean(value.api_ready, "api_ready");
    this.daemonReady = requiredHealthBoolean(value.daemon_ready, "daemon_ready");
    this.invocationReady = requiredHealthBoolean(value.invocation_ready, "invocation_ready");
    this.directoryReady = requiredHealthBoolean(value.directory_ready, "directory_ready");
    this.trustReady = requiredHealthBoolean(value.trust_ready, "trust_ready");
    this.runtimeReady = requiredHealthBoolean(value.runtime_ready, "runtime_ready");
    this.version = optionalHealthString(value.version, "version");
    this.abiVersion = optionalHealthNonNegativeInteger(value.abi_version, "abi_version");
    this.mismatch = optionalHealthObject(value.mismatch, "mismatch");
    this.diagnostics = healthDiagnostics(value.diagnostics ?? []);
  }

  static fromJSON(raw) {
    return new RuntimeHealth(parseJSON(raw, "runtime health"));
  }

  apiAlive() {
    return this.apiReady && this.daemonReady;
  }

  ready() {
    return this.runtimeReady;
  }

  toJSON() {
    return {
      api_ready: this.apiReady,
      daemon_ready: this.daemonReady,
      invocation_ready: this.invocationReady,
      directory_ready: this.directoryReady,
      trust_ready: this.trustReady,
      runtime_ready: this.runtimeReady,
      version: this.version,
      abi_version: this.abiVersion,
      mismatch: this.mismatch,
      diagnostics: this.diagnostics,
    };
  }
}

export class DiagnosticCheck {
  constructor(fields) {
    const value = objectValue(fields, "diagnostic check");
    this.name = requiredHealthString(value.name, "checks.name");
    this.ready = requiredHealthBoolean(value.ready, "checks.ready");
    this.message = optionalHealthString(value.message, "checks.message");
  }

  toJSON() {
    return {
      name: this.name,
      ready: this.ready,
      message: this.message,
    };
  }
}

export class DiagnosticsReport {
  constructor(fields) {
    const value = objectValue(fields, "diagnostics report");
    this.profile = requiredHealthString(value.profile, "profile");
    if (this.profile !== HEALTH_PROFILE) {
      throw invalidHealth("profile must be health");
    }
    this.kind = requiredHealthString(value.kind, "kind");
    if (this.kind !== "diagnostics_report") {
      throw invalidHealth("kind must be diagnostics_report");
    }
    this.state = requiredHealthString(value.state, "state");
    this.ready = requiredHealthBoolean(value.ready, "ready");
    this.version = requiredHealthString(value.version, "version");
    this.abiVersion = requiredHealthNonNegativeInteger(value.abi_version, "abi_version");
    this.controlEndpoint = requiredHealthString(value.control_endpoint, "control_endpoint");
    this.invocationEndpoint = optionalHealthString(value.invocation_endpoint, "invocation_endpoint");
    if (!Array.isArray(value.checks) || value.checks.length === 0) {
      throw invalidHealth("checks must be non-empty");
    }
    this.checks = value.checks.map((check) => new DiagnosticCheck(check));
    this.diagnostics = healthDiagnostics(value.diagnostics ?? []);
  }

  static fromJSON(raw) {
    return new DiagnosticsReport(parseJSON(raw, "diagnostics report"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      state: this.state,
      ready: this.ready,
      version: this.version,
      abi_version: this.abiVersion,
      control_endpoint: this.controlEndpoint,
      invocation_endpoint: this.invocationEndpoint,
      checks: this.checks.map((check) => check.toJSON()),
      diagnostics: this.diagnostics,
    };
  }
}

export class HealthClient {
  constructor(transport) {
    if (!transport || typeof transport.runtimeHealth !== "function") {
      throw invalidSDK("health transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async runtimeHealth() {
    const transport = this.requireOpen();
    try {
      return RuntimeHealth.fromJSON(await transport.runtimeHealth());
    } catch (error) {
      if (error instanceof SDKError) {
        throw error;
      }
      throw new SDKError({
        code: ErrorCode.ROUTE_UNAVAILABLE,
        stage: "transport",
        retry: RetryHint.SAFE,
        message: "runtime health transport failed",
        cause: error,
      });
    }
  }

  async diagnostics() {
    const transport = this.requireOpen();
    if (typeof transport.runtimeDiagnostics !== "function") {
      throw new SDKError({
        code: ErrorCode.NOT_IMPLEMENTED,
        stage: "transport",
        retry: RetryHint.NEVER,
        message: "health diagnostics transport is not available",
      });
    }
    try {
      return DiagnosticsReport.fromJSON(await transport.runtimeDiagnostics());
    } catch (error) {
      if (error instanceof SDKError) {
        throw error;
      }
      throw new SDKError({
        code: ErrorCode.ROUTE_UNAVAILABLE,
        stage: "transport",
        retry: RetryHint.SAFE,
        message: "runtime diagnostics transport failed",
        cause: error,
      });
    }
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("health client is closed");
    }
    return this.transport;
  }
}

export class IdentityClient {
  constructor(transport) {
    if (!transport || typeof transport.projectDescriptorRef !== "function") {
      throw invalidSDK("identity transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async projectDescriptorRef(request) {
    const payload = identityRequest(request, ["descriptor_ref", "metadata"]);
    cleanRequiredString(payload.descriptor_ref, "descriptor_ref");
    return callJSON(this.requireOpen(), "projectDescriptorRef", payload, "identity descriptor_ref projection");
  }

  async buildDescriptorRef(request) {
    const payload = identityRequest(request, ["ability_ura", "descriptor_version", "metadata"]);
    cleanRequiredString(payload.ability_ura, "ability_ura");
    cleanRequiredString(payload.descriptor_version, "descriptor_version");
    return callJSON(this.requireOpen(), "buildDescriptorRef", payload, "identity descriptor_ref build");
  }

  async canonicalAbilityDescriptorRef(value, descriptorVersion = "") {
    cleanRequiredString(value, descriptorVersion ? "ability_ura" : "descriptor_ref");
    if (descriptorVersion) {
      const projection = await this.buildDescriptorRef({
        ability_ura: value,
        descriptor_version: descriptorVersion,
      });
      return requiredWireString(projection.descriptor_ref, "descriptor_ref");
    }
    const projection = await this.projectDescriptorRef({ descriptor_ref: value });
    return requiredWireString(projection.descriptor_ref, "descriptor_ref");
  }

  async abilityURAFromDescriptorRef(descriptorRef) {
    const projection = await this.projectDescriptorRef({ descriptor_ref: descriptorRef });
    return requiredWireString(projection.ability_ura, "ability_ura");
  }

  async ownerAbilityURA(ownerURA, abilityName) {
    cleanRequiredString(ownerURA, "owner_ura");
    cleanRequiredString(abilityName, "ability_name");
    const projection = await callJSON(
      this.requireOpen(),
      "ownerAbilityURA",
      { owner_ura: ownerURA, ability_name: abilityName },
      "identity owner ability URA",
    );
    return requiredWireString(projection.ability_ura ?? projection.ura, "ability_ura");
  }

  async ownerAbilityDescriptorRef(ownerURA, abilityName, descriptorVersion) {
    const abilityURA = await this.ownerAbilityURA(ownerURA, abilityName);
    return this.canonicalAbilityDescriptorRef(abilityURA, descriptorVersion);
  }

  async resourceURA(ownerURA, path) {
    cleanRequiredString(ownerURA, "owner_ura");
    cleanRequiredString(path, "path");
    const projection = await callJSON(
      this.requireOpen(),
      "resourceURA",
      { owner_ura: ownerURA, path },
      "identity resource URA",
    );
    return requiredWireString(projection.resource_ura ?? projection.ura, "resource_ura");
  }

  async descriptorBoundResourceSubjectURA(ownerURA, path) {
    return this.resourceURA(ownerURA, path);
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("identity client is closed");
    }
    return this.transport;
  }
}

export class AuthorityMetadata {
  constructor(fields) {
    const value = objectValue(fields, "authority metadata");
    this.kind = requiredAuthorityString(value.kind, "kind");
    this.key = requiredAuthorityString(value.key, "key");
    this.value = requiredAuthorityString(value.value, "value");
    validateAuthorityMetadataEnvelope(this.kind, this.key);
    validateAuthorityMetadata(this.toMetadata());
  }

  toMetadata() {
    return { [this.key]: this.value };
  }

  mergeInto(metadata = {}) {
    const merged = { ...objectValue(metadata, "metadata"), [this.key]: this.value };
    validateAuthorityMetadata(merged);
    return merged;
  }
}

export class DelegationProof {
  constructor(fields) {
    const value = objectValue(fields, "delegation proof");
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.subjectURA = requiredAuthorityString(value.subject_ura, "subject_ura");
    this.callerURA = requiredAuthorityString(value.caller_ura, "caller_ura");
    this.audience = requiredAuthorityString(value.audience, "audience");
    this.scopes = requiredAuthorityStringArray(value.scopes, "scopes");
    this.issuedAtMS = requiredAuthorityInteger(value.issued_at_ms, "issued_at_ms");
    this.expiresAtMS = requiredAuthorityInteger(value.expires_at_ms, "expires_at_ms");
    this.signatureBase64 = requiredAuthorityBase64(value.signature_base64, "signature_base64");
    this.signature = Buffer.from(this.signatureBase64, "base64");
    this.metadataValue = authorityOptionalString(value.metadata_value, "metadata_value") ?? "";
    validateDelegationProof(this);
  }

  static fromMetadata(value) {
    const { payload, signatureBase64 } = decodeAuthorityMetadata(value, "delegation");
    return new DelegationProof({
      ...payload,
      signature_base64: signatureBase64,
      metadata_value: value.trim(),
    });
  }

  metadata() {
    validateDelegationProof(this);
    if (!this.metadataValue) {
      throw invalidAuthority("delegation metadata value is required");
    }
    return new AuthorityMetadata({
      kind: "delegation",
      key: DELEGATION_METADATA_KEY,
      value: this.metadataValue,
    });
  }

  toJSON() {
    return {
      issuer_ura: this.issuerURA,
      subject_ura: this.subjectURA,
      caller_ura: this.callerURA,
      audience: this.audience,
      scopes: [...this.scopes],
      issued_at_ms: this.issuedAtMS,
      expires_at_ms: this.expiresAtMS,
      signature_base64: this.signatureBase64,
    };
  }
}

export class SessionAuthority {
  constructor(fields) {
    const value = objectValue(fields, "session authority");
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.sessionID = requiredAuthorityString(value.session_id, "session_id");
    this.sessionOwnerUserID = requiredAuthorityString(value.session_owner_user_id, "session_owner_user_id");
    this.creatorPrincipalID = requiredAuthorityString(value.creator_principal_id, "creator_principal_id");
    this.calleeURA = requiredAuthorityString(value.callee_ura, "callee_ura");
    this.subjectURA = requiredAuthorityString(value.subject_ura, "subject_ura");
    this.audience = requiredAuthorityString(value.audience, "audience");
    this.scopes = requiredAuthorityStringArray(value.scopes, "scopes");
    this.allowedActions = requiredAuthorityStringArray(value.allowed_actions, "allowed_actions");
    this.allowedFollowupAbilities = requiredAuthorityStringArray(
      value.allowed_followup_abilities,
      "allowed_followup_abilities",
    );
    this.issuedAtMS = requiredAuthorityInteger(value.issued_at_ms, "issued_at_ms");
    this.expiresAtMS = requiredAuthorityInteger(value.expires_at_ms, "expires_at_ms");
    this.signatureBase64 = requiredAuthorityBase64(value.signature_base64, "signature_base64");
    this.signature = Buffer.from(this.signatureBase64, "base64");
    this.metadataValue = authorityOptionalString(value.metadata_value, "metadata_value") ?? "";
    validateSessionAuthority(this);
  }

  static fromMetadata(value) {
    const { payload, signatureBase64 } = decodeAuthorityMetadata(value, "session authority");
    return new SessionAuthority({
      ...payload,
      signature_base64: signatureBase64,
      metadata_value: value.trim(),
    });
  }

  metadata() {
    validateSessionAuthority(this);
    if (!this.metadataValue) {
      throw invalidAuthority("session authority metadata value is required");
    }
    return new AuthorityMetadata({
      kind: "session_authority",
      key: SESSION_AUTHORITY_METADATA_KEY,
      value: this.metadataValue,
    });
  }

  toJSON() {
    return {
      issuer_ura: this.issuerURA,
      session_id: this.sessionID,
      session_owner_user_id: this.sessionOwnerUserID,
      creator_principal_id: this.creatorPrincipalID,
      callee_ura: this.calleeURA,
      subject_ura: this.subjectURA,
      audience: this.audience,
      scopes: [...this.scopes],
      allowed_actions: [...this.allowedActions],
      allowed_followup_abilities: [...this.allowedFollowupAbilities],
      issued_at_ms: this.issuedAtMS,
      expires_at_ms: this.expiresAtMS,
      signature_base64: this.signatureBase64,
    };
  }
}

export class DelegationRequest {
  constructor(fields) {
    const value = objectValue(fields, "delegation request");
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.subjectURA = requiredAuthorityString(value.subject_ura, "subject_ura");
    this.callerURA = requiredAuthorityString(value.caller_ura, "caller_ura");
    this.audience = requiredAuthorityString(value.audience, "audience");
    this.scopes = requiredAuthorityStringArray(value.scopes, "scopes");
    this.issuedAtMS = requiredAuthorityInteger(value.issued_at_ms, "issued_at_ms");
    this.expiresAtMS = requiredAuthorityInteger(value.expires_at_ms, "expires_at_ms");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
    validateDelegationRequest(this);
  }

  toJSON() {
    return {
      issuer_ura: this.issuerURA,
      subject_ura: this.subjectURA,
      caller_ura: this.callerURA,
      audience: this.audience,
      scopes: [...this.scopes],
      issued_at_ms: this.issuedAtMS,
      expires_at_ms: this.expiresAtMS,
      metadata: this.metadata,
    };
  }
}

export class SessionAuthorityRequest {
  constructor(fields) {
    const value = objectValue(fields, "session authority request");
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.sessionID = requiredAuthorityString(value.session_id, "session_id");
    this.sessionOwnerUserID = requiredAuthorityString(value.session_owner_user_id, "session_owner_user_id");
    this.creatorPrincipalID = requiredAuthorityString(value.creator_principal_id, "creator_principal_id");
    this.calleeURA = requiredAuthorityString(value.callee_ura, "callee_ura");
    this.subjectURA = requiredAuthorityString(value.subject_ura, "subject_ura");
    this.audience = requiredAuthorityString(value.audience, "audience");
    this.scopes = requiredAuthorityStringArray(value.scopes, "scopes");
    this.allowedActions = requiredAuthorityStringArray(value.allowed_actions, "allowed_actions");
    this.allowedFollowupAbilities = requiredAuthorityStringArray(
      value.allowed_followup_abilities,
      "allowed_followup_abilities",
    );
    this.issuedAtMS = requiredAuthorityInteger(value.issued_at_ms, "issued_at_ms");
    this.expiresAtMS = requiredAuthorityInteger(value.expires_at_ms, "expires_at_ms");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
    validateSessionAuthorityRequest(this);
  }

  toJSON() {
    return {
      issuer_ura: this.issuerURA,
      session_id: this.sessionID,
      session_owner_user_id: this.sessionOwnerUserID,
      creator_principal_id: this.creatorPrincipalID,
      callee_ura: this.calleeURA,
      subject_ura: this.subjectURA,
      audience: this.audience,
      scopes: [...this.scopes],
      allowed_actions: [...this.allowedActions],
      allowed_followup_abilities: [...this.allowedFollowupAbilities],
      issued_at_ms: this.issuedAtMS,
      expires_at_ms: this.expiresAtMS,
      metadata: this.metadata,
    };
  }
}

export class AuthorityClient {
  constructor(transport) {
    if (!transport || typeof transport.mintDelegationProof !== "function") {
      throw invalidSDK("authority transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async mintDelegationProof(request) {
    const payload = request instanceof DelegationRequest ? request : new DelegationRequest(request);
    const raw = await this.requireOpen().mintDelegationProof(Buffer.from(JSON.stringify(payload.toJSON())));
    return DelegationProof.fromMetadata(decodeAuthorityMetadataProjection(raw, DELEGATION_METADATA_KEY, "delegation"));
  }

  async mintSessionAuthority(request) {
    const payload = request instanceof SessionAuthorityRequest ? request : new SessionAuthorityRequest(request);
    const transport = this.requireOpen();
    if (typeof transport.mintSessionAuthority !== "function") {
      throw invalidSDK("mintSessionAuthority transport function is required");
    }
    const raw = await transport.mintSessionAuthority(Buffer.from(JSON.stringify(payload.toJSON())));
    return SessionAuthority.fromMetadata(
      decodeAuthorityMetadataProjection(raw, SESSION_AUTHORITY_METADATA_KEY, "session authority"),
    );
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("authority client is closed");
    }
    return this.transport;
  }
}

export class DirectoryClient {
  constructor(transport) {
    if (!transport || typeof transport.resolve !== "function") {
      throw invalidSDK("directory transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async resolve(query) {
    const payload = directoryRequest(query, [
      ...directoryBaseFields(),
      "query_name",
      "ability_name",
      "qtype",
      "realm_hint",
      "peer_hub_urls",
    ]);
    validateDirectoryBase(payload, false);
    if (!payload.query_name && !payload.realm_hint) {
      throw invalidDirectory("query_name or realm_hint is required");
    }
    return callJSON(this.requireOpen(), "resolve", payload, "directory resolve");
  }

  async listDevices(query) {
    return this.listPage("listDevices", query, "directory devices page", directoryBaseFields());
  }

  async listAgents(query) {
    return this.listPage("listAgents", query, "directory agents page", directoryBaseFields());
  }

  async listAbilities(query) {
    return this.listPage("listAbilities", query, "directory abilities page", [
      ...directoryBaseFields(),
      "scope",
      "owner_ura",
      "ability_ura",
    ]);
  }

  async listPage(method, query, label, allowed) {
    const payload = directoryRequest(query, allowed);
    applyDirectoryDefaultLimit(payload);
    validateDirectoryBase(payload, true);
    return callJSON(this.requireOpen(), method, payload, label);
  }

  async buildDirectorySubscriptionInvocation(request) {
    const payload = directoryRequest(request, [
      ...directoryBaseFields(),
      "stream",
      "realm",
      "owner_ura",
      "device_ura",
      "agent_ura",
      "ability_ura",
      "item_kind",
      "resume_cursor",
      "heartbeat_interval_ms",
    ]);
    validateDirectoryBase(payload, false);
    if (payload.stream && payload.stream !== "directory") {
      throw invalidDirectory("directory subscription stream mismatch");
    }
    payload.stream = payload.stream || "directory";
    if (payload.heartbeat_interval_ms !== undefined && (!Number.isInteger(payload.heartbeat_interval_ms) || payload.heartbeat_interval_ms < 0)) {
      throw invalidDirectory("heartbeat_interval_ms must be non-negative");
    }
    return callJSON(this.requireOpen(), "buildDirectorySubscriptionInvocation", payload, "directory subscription invocation");
  }

  async subscribeDirectory(request) {
    const payload = directoryRequest(request, [
      ...directoryBaseFields(),
      "stream",
      "realm",
      "owner_ura",
      "device_ura",
      "agent_ura",
      "ability_ura",
      "item_kind",
      "resume_cursor",
      "heartbeat_interval_ms",
    ]);
    validateDirectoryBase(payload, false);
    payload.stream = payload.stream || "directory";
    const transport = this.requireOpen();
    if (typeof transport.subscribeDirectory !== "function") {
      throw invalidSDK("directory subscribe transport function is required");
    }
    const result = await transport.subscribeDirectory(Buffer.from(JSON.stringify(payload)));
    return new StreamHandle(result.transport, parseJSON(result.open, "directory subscription open"));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("directory client is closed");
    }
    return this.transport;
  }
}

export class EventCursor {
  constructor(fields) {
    const value = objectValue(fields, "event cursor");
    this.stream = requiredEventStream(value.stream, "stream");
    this.sequence = eventNonNegativeInteger(value.sequence, "sequence");
    this.token = cleanOptionalString(value.token ?? `${this.stream}:${this.sequence}`, "token");
    validateEventCursor(this);
  }

  static fromJSON(raw) {
    return new EventCursor(parseJSON(raw, "event cursor"));
  }

  resumeToken() {
    return this.token || `${this.stream}:${this.sequence}`;
  }

  toJSON() {
    return {
      stream: this.stream,
      sequence: this.sequence,
      token: this.resumeToken(),
    };
  }
}

export class EventFrame {
  constructor(fields) {
    const value = objectValue(fields, "event frame");
    this.profile = requiredEventString(value.profile, "profile");
    if (this.profile !== EVENTS_PROFILE) {
      throw invalidEvents("invalid event frame projection");
    }
    this.stream = requiredEventStream(value.stream, "stream");
    this.kind = requiredEventString(value.kind, "kind");
    this.eventId = requiredEventString(value.event_id, "event_id");
    this.cursor = new EventCursor(objectValue(value.cursor, "cursor"));
    if (this.cursor.stream !== this.stream) {
      throw invalidEvents("event cursor stream mismatch");
    }
    this.resumeToken = requiredEventString(value.resume_token, "resume_token");
    this.occurredUnixMS = eventNonNegativeInteger(value.occurred_unix_ms, "occurred_unix_ms");
    this.occurredAt = requiredEventString(value.occurred_at, "occurred_at");
    this.subjectRef = value.subject_ref ?? null;
    this.tenantRef = value.tenant_ref ?? null;
    this.payload = Object.hasOwn(value, "payload") ? value.payload : null;
    this.droppedCount = eventNonNegativeInteger(value.dropped_count, "dropped_count");
    this.reconnectAfterMS = eventOptionalNonNegativeInteger(
      value.reconnect_after_ms,
      "reconnect_after_ms",
    );
    validateEventReconnectAfterMS(this.reconnectAfterMS);
    this.terminal = eventBoolean(value.terminal, "terminal");
    this.metadata = objectValue(value.metadata, "metadata");
    validateEventFrame(this);
  }

  static fromJSON(raw) {
    return new EventFrame(parseJSON(raw, "event frame"));
  }

  toJSON() {
    return {
      profile: this.profile,
      stream: this.stream,
      kind: this.kind,
      event_id: this.eventId,
      cursor: this.cursor.toJSON(),
      resume_token: this.resumeToken,
      occurred_unix_ms: this.occurredUnixMS,
      occurred_at: this.occurredAt,
      subject_ref: this.subjectRef,
      tenant_ref: this.tenantRef,
      payload: this.payload,
      dropped_count: this.droppedCount,
      reconnect_after_ms: this.reconnectAfterMS,
      terminal: this.terminal,
      metadata: this.metadata,
    };
  }
}

export class DeviceEventPage {
  constructor(fields) {
    const value = objectValue(fields, "device event page");
    this.profile = requiredEventString(value.profile, "profile");
    this.stream = requiredEventStream(value.stream, "stream");
    if (this.profile !== EVENTS_PROFILE || this.stream !== "device") {
      throw invalidEvents("invalid device event page projection");
    }
    this.itemKind = requiredEventString(value.item_kind, "item_kind");
    const items = value.items;
    if (!Array.isArray(items)) {
      throw invalidEvents("items must be a list");
    }
    this.items = items.map((item) => {
      const frame = new EventFrame(item);
      if (frame.stream !== "device") {
        throw invalidEvents("device event page item stream mismatch");
      }
      return frame;
    });
    this.nextCursor = eventOptionalString(value.next_cursor, "next_cursor");
    this.hasMore = eventBoolean(value.has_more, "has_more");
    this.limit = eventPositiveBoundedInteger(value.limit, "limit", MAX_EVENT_PAGE_SIZE);
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new DeviceEventPage(parseJSON(raw, "device event page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      stream: this.stream,
      item_kind: this.itemKind,
      items: this.items.map((item) => item.toJSON()),
      next_cursor: this.nextCursor,
      has_more: this.hasMore,
      limit: this.limit,
      metadata: this.metadata,
    };
  }
}

export class EventStream {
  constructor(stream, handle, open = {}) {
    this.stream = requiredEventStream(stream, "stream");
    if (!(handle instanceof StreamHandle)) {
      throw invalidEvents("runtime stream handle is required");
    }
    const value = objectValue(open, "event stream");
    if (value.stream !== undefined && value.stream !== this.stream) {
      throw invalidEvents("event stream projection mismatch");
    }
    this.handle = handle;
    this.state = eventOptionalString(value.state, "state") ?? "Live";
    this.streamId = eventOptionalString(value.stream_id, "stream_id") ?? "";
    this.resumeToken = eventOptionalString(value.resume_token, "resume_token") ?? "";
    this.metadata = objectValue(value.metadata ?? { profile: EVENTS_PROFILE }, "metadata");
  }

  static fromTransportResult(result, stream) {
    const value = objectValue(result, "event stream transport result");
    const open = parseJSON(value.open, "event stream open");
    return new EventStream(stream, new StreamHandle(value.transport, open), open);
  }

  async receive(options = {}) {
    const raw = await this.handle.receive(options);
    const frame = new EventFrame(raw);
    this.state = this.handle.terminal ? "Terminal" : this.state;
    return frame;
  }

  async *events(options = {}) {
    try {
      while (!this.handle.closed && !this.handle.terminal) {
        yield await this.receive(options);
      }
    } finally {
      if (options.closeOnReturn !== false) {
        await this.close();
      }
    }
  }

  [Symbol.asyncIterator]() {
    return this.events();
  }

  async cancel(reason = "") {
    await this.handle.cancel(reason);
    this.state = "Cancelled";
  }

  async close() {
    await this.handle.close();
    this.state = "Closed";
  }

  terminalEvent() {
    const raw = this.handle.terminalEvent();
    return raw ? new EventFrame(raw) : null;
  }
}

export class EventClient {
  constructor(transport) {
    if (!transport || typeof transport.buildDirectorySubscriptionInvocation !== "function") {
      throw invalidSDK("events transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildDirectorySubscriptionInvocation(request) {
    return this.buildSubscriptionInvocation("buildDirectorySubscriptionInvocation", request, "directory");
  }

  async buildDeviceSubscriptionInvocation(request) {
    return this.buildSubscriptionInvocation("buildDeviceSubscriptionInvocation", request, "device");
  }

  async buildSessionSubscriptionInvocation(request) {
    return this.buildSubscriptionInvocation("buildSessionSubscriptionInvocation", request, "session");
  }

  async buildInvocationSubscriptionInvocation(request) {
    return this.buildSubscriptionInvocation("buildInvocationSubscriptionInvocation", request, "invocation");
  }

  async buildSubscriptionInvocation(method, request, stream) {
    const payload = eventsSubscriptionRequest(request, stream);
    return InvocationDraft.fromJSON(await callRaw(this.requireOpen(), method, payload));
  }

  async subscribeDirectory(request) {
    return this.subscribe("subscribeDirectory", request, "directory");
  }

  async subscribeDevices(request) {
    return this.subscribe("subscribeDevices", request, "device");
  }

  async subscribeSessions(request) {
    return this.subscribe("subscribeSessions", request, "session");
  }

  async subscribeInvocations(request) {
    return this.subscribe("subscribeInvocations", request, "invocation");
  }

  async subscribe(method, request, stream) {
    const payload = eventsSubscriptionRequest(request, stream);
    const transport = this.requireOpen();
    if (typeof transport[method] !== "function") {
      throw invalidSDK(`${method} transport function is required`);
    }
    return EventStream.fromTransportResult(await transport[method](Buffer.from(JSON.stringify(payload))), stream);
  }

  async listDeviceEvents(request) {
    const payload = eventsDeviceEventListRequest(request);
    return DeviceEventPage.fromJSON(await callRaw(this.requireOpen(), "listDeviceEvents", payload));
  }

  async projectDirectoryEvent(input) {
    const payload = eventProjectionInput(input, "directory");
    return EventFrame.fromJSON(await callRaw(this.requireOpen(), "projectDirectoryEvent", payload));
  }

  async projectLiveEvent(input) {
    const payload = eventProjectionInput(input);
    return EventFrame.fromJSON(await callRaw(this.requireOpen(), "projectLiveEvent", payload));
  }

  async projectDropReport(input) {
    const payload = eventDropReportInput(input);
    return EventFrame.fromJSON(await callRaw(this.requireOpen(), "projectDropReport", payload));
  }

  async projectTerminal(input) {
    const payload = eventTerminalInput(input);
    return EventFrame.fromJSON(await callRaw(this.requireOpen(), "projectTerminal", payload));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("events client is closed");
    }
    return this.transport;
  }
}

export class MissionStatus {
  constructor(fields) {
    const value = objectValue(fields, "mission status");
    this.profile = requiredMissionString(value.profile, "profile");
    this.kind = requiredMissionString(value.kind, "kind");
    if (this.profile !== MISSION_PROFILE || this.kind !== "mission_status") {
      throw invalidMission("invalid mission status projection");
    }
    this.missionID = requiredMissionID(value.mission_id, "mission_id");
    this.state = requiredMissionState(value.state, "state");
    this.terminal = missionBoolean(value.terminal, "terminal");
    this.partialFailures = missionNonNegativeInteger(value.partial_failures, "partial_failures");
    this.cancelled = missionBoolean(value.cancelled, "cancelled");
    this.parentInvocationID = missionOptionalString(value.parent_invocation_id, "parent_invocation_id");
    this.parentReceiptURA = missionOptionalString(value.parent_receipt_ura, "parent_receipt_ura");
    this.parentInvocation = value.parent_invocation === null || value.parent_invocation === undefined
      ? null
      : objectValue(value.parent_invocation, "parent_invocation");
    this.childInvocations = missionObjectArray(value.child_invocations, "child_invocations");
    this.childReceipts = missionObjectArray(value.child_receipts, "child_receipts");
    this.outputRefs = missionObjectArray(value.output_refs, "output_refs");
    this.error = value.error ?? null;
    this.metadata = objectValue(value.metadata, "metadata");
    validateMissionStatus(this);
  }

  static fromJSON(raw) {
    return new MissionStatus(parseJSON(raw, "mission status"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      mission_id: this.missionID,
      state: this.state,
      terminal: this.terminal,
      partial_failures: this.partialFailures,
      cancelled: this.cancelled,
      parent_invocation_id: this.parentInvocationID,
      parent_receipt_ura: this.parentReceiptURA,
      parent_invocation: this.parentInvocation,
      child_invocations: this.childInvocations,
      child_receipts: this.childReceipts,
      output_refs: this.outputRefs,
      ...(this.error ? { error: this.error } : {}),
      metadata: this.metadata,
    };
  }
}

export class MissionRun {
  constructor(status) {
    this.status = status instanceof MissionStatus ? status : new MissionStatus(status);
  }

  static fromJSON(raw) {
    return new MissionRun(MissionStatus.fromJSON(raw));
  }

  toJSON() {
    return {
      status: this.status.toJSON(),
    };
  }
}

export const MissionCancelResult = MissionStatus;

export class MissionEvent {
  constructor(fields) {
    const value = objectValue(fields, "mission event");
    this.profile = requiredMissionString(value.profile, "profile");
    this.kind = requiredMissionString(value.kind, "kind");
    if (this.profile !== MISSION_PROFILE || this.kind !== "mission_event") {
      throw invalidMission("invalid mission event projection");
    }
    this.missionID = requiredMissionID(value.mission_id, "mission_id");
    this.sequence = missionNonNegativeInteger(value.sequence, "sequence");
    this.eventType = requiredMissionString(value.event_type, "event_type");
    this.occurredUnixMS = missionNonNegativeInteger(value.occurred_unix_ms, "occurred_unix_ms");
    this.terminal = missionBoolean(value.terminal, "terminal");
    this.payload = objectValue(value.payload, "payload");
    this.receipt = objectValue(value.receipt, "receipt");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new MissionEvent(parseJSON(raw, "mission event"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      mission_id: this.missionID,
      sequence: this.sequence,
      event_type: this.eventType,
      occurred_unix_ms: this.occurredUnixMS,
      terminal: this.terminal,
      payload: this.payload,
      receipt: this.receipt,
      metadata: this.metadata,
    };
  }
}

export class MissionEventPage {
  constructor(fields) {
    const value = objectValue(fields, "mission event page");
    this.profile = requiredMissionString(value.profile, "profile");
    this.kind = requiredMissionString(value.kind, "kind");
    if (this.profile !== MISSION_PROFILE || this.kind !== "mission_event_page") {
      throw invalidMission("invalid mission event page projection");
    }
    this.missionID = requiredMissionID(value.mission_id, "mission_id");
    this.cursorSequence = missionNonNegativeInteger(value.cursor_sequence, "cursor_sequence");
    this.nextCursorSequence = missionOptionalNonNegativeInteger(value.next_cursor_sequence, "next_cursor_sequence");
    this.hasMore = missionBoolean(value.has_more, "has_more");
    this.droppedCount = missionNonNegativeInteger(value.dropped_count, "dropped_count");
    if (!Array.isArray(value.events)) {
      throw invalidMission("mission events must be an array");
    }
    this.events = value.events.map((item) => new MissionEvent(item));
    this.metadata = objectValue(value.metadata, "metadata");
    validateMissionEventPage(this);
  }

  static fromJSON(raw) {
    return new MissionEventPage(parseJSON(raw, "mission event page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      mission_id: this.missionID,
      cursor_sequence: this.cursorSequence,
      next_cursor_sequence: this.nextCursorSequence,
      has_more: this.hasMore,
      dropped_count: this.droppedCount,
      events: this.events.map((event) => event.toJSON()),
      metadata: this.metadata,
    };
  }
}

export class MissionEventStream {
  constructor(handle) {
    if (!(handle instanceof StreamHandle)) {
      throw invalidMission("mission event stream handle is required");
    }
    this.handle = handle;
  }

  get streamId() {
    return missionOptionalString(this.handle.open.stream_id, "stream_id") ?? "";
  }

  get state() {
    return missionOptionalString(this.handle.open.state, "state") ?? "";
  }

  async receive(options = {}) {
    const frame = await this.handle.receive(options);
    if (frame && typeof frame === "object" && frame.profile === MISSION_PROFILE) {
      return new MissionEvent(frame);
    }
    if (frame && typeof frame === "object" && frame.payload_json !== undefined) {
      return MissionEvent.fromJSON(frame.payload_json);
    }
    if (frame && typeof frame === "object" && frame.payload !== undefined) {
      return new MissionEvent(objectValue(frame.payload, "mission event payload"));
    }
    throw invalidMission("mission event stream frame payload is required");
  }

  async *events(options = {}) {
    try {
      while (!this.handle.closed && !this.handle.terminal) {
        const event = await this.receive(options);
        yield event;
        if (event.terminal) {
          break;
        }
      }
    } finally {
      if (options.closeOnReturn !== false) {
        await this.close();
      }
    }
  }

  [Symbol.asyncIterator]() {
    return this.events();
  }

  async cancel(reason = "") {
    return this.handle.cancel(reason);
  }

  async close() {
    await this.handle.close();
  }
}

export class MissionClient {
  constructor(transport) {
    if (!transport || typeof transport.buildRunEALInvocation !== "function") {
      throw invalidSDK("mission transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildRunEALInvocation(request) {
    return this.buildInvocation("buildRunEALInvocation", missionRunRequest(request));
  }

  async buildRunFileInvocation(request) {
    return this.buildInvocation("buildRunFileInvocation", missionRunFileRequest(request));
  }

  async buildTrackInvocation(request) {
    return this.buildInvocation("buildTrackInvocation", missionTrackRequest(request));
  }

  async buildCancelInvocation(request) {
    return this.buildInvocation("buildCancelInvocation", missionCancelRequest(request));
  }

  async buildEventsInvocation(request) {
    return this.buildInvocation("buildEventsInvocation", missionEventsRequest(request));
  }

  async buildInvocation(method, payload) {
    return InvocationDraft.fromJSON(await callRaw(this.requireOpen(), method, payload));
  }

  async runEAL(request) {
    return MissionRun.fromJSON(await callRaw(this.requireOpen(), "runEAL", missionRunRequest(request)));
  }

  async runFile(request) {
    return MissionRun.fromJSON(await callRaw(this.requireOpen(), "runFile", missionRunFileRequest(request)));
  }

  async track(request) {
    return MissionStatus.fromJSON(await callRaw(this.requireOpen(), "track", missionTrackRequest(request)));
  }

  async cancel(request) {
    return MissionStatus.fromJSON(await callRaw(this.requireOpen(), "cancel", missionCancelRequest(request)));
  }

  async events(request) {
    return MissionEventPage.fromJSON(await callRaw(this.requireOpen(), "events", missionEventsRequest(request)));
  }

  async openEventStream(request) {
    const transport = this.requireOpen();
    if (typeof transport.openEventStream !== "function") {
      throw invalidSDK("mission event stream transport function is required");
    }
    const result = await transport.openEventStream(Buffer.from(JSON.stringify(missionEventsRequest(request))));
    if (result instanceof StreamHandle) {
      return new MissionEventStream(result);
    }
    const value = objectValue(result, "mission event stream transport result");
    return new MissionEventStream(new StreamHandle(value.transport, parseJSON(value.open, "mission event stream open")));
  }

  async projectStatus(value) {
    return MissionStatus.fromJSON(await callMissionProjection(this.requireOpen(), "projectStatus", value));
  }

  async projectEvents(value) {
    return MissionEventPage.fromJSON(await callMissionProjection(this.requireOpen(), "projectEvents", value));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("mission client is closed");
    }
    return this.transport;
  }
}

export class GatewayStatus {
  constructor(fields) {
    const value = objectValue(fields, "gateway status");
    this.profile = requiredAdminString(value.profile, "profile");
    if (this.profile !== ADMIN_GATEWAY_PROFILE) {
      throw invalidAdminGateway("invalid gateway status projection");
    }
    this.gatewayID = requiredAdminString(value.gateway_id, "gateway_id");
    this.ready = adminBoolean(value.ready, "ready");
    this.state = requiredAdminString(value.state, "state");
    this.processLive = adminBoolean(value.process_live, "process_live");
    this.controlReady = adminBoolean(value.control_ready, "control_ready");
    this.runtimeReady = adminBoolean(value.runtime_ready, "runtime_ready");
    this.directoryReady = adminBoolean(value.directory_ready, "directory_ready");
    this.trustReady = adminBoolean(value.trust_ready, "trust_ready");
    this.publicListenerReady = adminBoolean(value.public_listener_ready, "public_listener_ready");
    this.listeners = adminObjectArray(value.listeners, "listeners");
    for (const listener of this.listeners) {
      requiredAdminString(listener.kind, "listener.kind");
      requiredAdminString(listener.endpoint, "listener.endpoint");
      adminBoolean(listener.ready, "listener.ready");
      adminBoolean(listener.public, "listener.public");
    }
    this.identity = value.identity === null || value.identity === undefined ? null : objectValue(value.identity, "identity");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new GatewayStatus(parseJSON(raw, "gateway status"));
  }

  toJSON() {
    return {
      profile: this.profile,
      gateway_id: this.gatewayID,
      ready: this.ready,
      state: this.state,
      process_live: this.processLive,
      control_ready: this.controlReady,
      runtime_ready: this.runtimeReady,
      directory_ready: this.directoryReady,
      trust_ready: this.trustReady,
      public_listener_ready: this.publicListenerReady,
      listeners: this.listeners,
      identity: this.identity,
      metadata: this.metadata,
    };
  }
}

export class AdminAgentRecord {
  constructor(fields) {
    const value = objectValue(fields, "admin agent record");
    this.name = requiredAdminString(value.name, "name");
    this.agentURA = adminOptionalString(value.agent_ura, "agent_ura");
    this.ownerURA = adminOptionalString(value.owner_ura, "owner_ura");
    this.deviceURA = adminOptionalString(value.device_ura, "device_ura");
    this.state = requiredAdminString(value.state, "state");
    this.runtime = requiredAdminString(value.runtime, "runtime");
    this.model = adminOptionalString(value.model, "model");
    this.label = adminOptionalString(value.label, "label");
    if (!Array.isArray(value.abilities)) {
      throw invalidAdminGateway("abilities must be an array");
    }
    this.abilities = value.abilities.map((ability) => requiredAdminString(ability, "ability"));
    this.metadata = objectValue(value.metadata, "metadata");
  }

  toJSON() {
    return {
      name: this.name,
      agent_ura: this.agentURA,
      owner_ura: this.ownerURA,
      device_ura: this.deviceURA,
      state: this.state,
      runtime: this.runtime,
      model: this.model,
      label: this.label,
      abilities: [...this.abilities],
      metadata: this.metadata,
    };
  }
}

export class AdminAgentPage {
  constructor(fields) {
    const value = objectValue(fields, "admin agent page");
    this.profile = requiredAdminString(value.profile, "profile");
    this.kind = requiredAdminString(value.kind, "kind");
    if (this.profile !== ADMIN_GATEWAY_PROFILE || this.kind !== "agent_records") {
      throw invalidAdminGateway("invalid admin agent page projection");
    }
    this.state = requiredAdminString(value.state, "state");
    if (!Array.isArray(value.items)) {
      throw invalidAdminGateway("agent items must be an array");
    }
    this.items = value.items.map((item) => new AdminAgentRecord(item));
    this.nextCursor = value.next_cursor ?? null;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new AdminAgentPage(parseJSON(raw, "admin agent page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      state: this.state,
      items: this.items.map((item) => item.toJSON()),
      next_cursor: this.nextCursor,
      metadata: this.metadata,
    };
  }
}

export class AdminGatewayResult {
  constructor(fields) {
    const value = objectValue(fields, "admin gateway result");
    this.profile = requiredAdminString(value.profile, "profile");
    this.kind = requiredAdminString(value.kind, "kind");
    if (this.profile !== ADMIN_GATEWAY_PROFILE) {
      throw invalidAdminGateway("invalid admin gateway result projection");
    }
    this.operation = adminOptionalString(value.operation, "operation");
    this.state = requiredAdminString(value.state, "state");
    this.agentURA = adminOptionalString(value.agent_ura, "agent_ura");
    this.deviceURA = adminOptionalString(value.device_ura, "device_ura");
    this.ack = value.ack === undefined || value.ack === null ? null : adminBoolean(value.ack, "ack");
    this.runtimeNotReady = value.runtime_not_ready === undefined ? false : adminBoolean(value.runtime_not_ready, "runtime_not_ready");
    this.runtimeCatalogNotReady = value.runtime_catalog_not_ready === undefined
      ? false
      : adminBoolean(value.runtime_catalog_not_ready, "runtime_catalog_not_ready");
    this.items = value.items === undefined ? [] : adminObjectArray(value.items, "items");
    this.nextCursor = value.next_cursor ?? null;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new AdminGatewayResult(parseJSON(raw, "admin gateway result"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      operation: this.operation,
      state: this.state,
      agent_ura: this.agentURA,
      device_ura: this.deviceURA,
      ack: this.ack,
      runtime_not_ready: this.runtimeNotReady,
      runtime_catalog_not_ready: this.runtimeCatalogNotReady,
      items: this.items,
      next_cursor: this.nextCursor,
      metadata: this.metadata,
    };
  }
}

export const AgentStartResult = AdminGatewayResult;
export const AgentStopResult = AdminGatewayResult;
export const AgentRefreshResult = AdminGatewayResult;
export const JoinResult = AdminGatewayResult;
export const LeaveResult = AdminGatewayResult;
export const DeviceAdminResult = AdminGatewayResult;

export class PairingPreflight {
  constructor(fields) {
    const value = objectValue(fields, "pairing preflight");
    validateAdminKind(value, "pairing_preflight");
    this.profile = value.profile;
    this.kind = value.kind;
    this.state = requiredAdminString(value.state, "state");
    this.hubURA = requiredAdminURA(value.hub_ura, "hub_ura");
    this.deviceURA = requiredAdminURA(value.device_ura, "device_ura");
    this.pairingRequired = adminBoolean(value.pairing_required, "pairing_required");
    this.trustReady = adminBoolean(value.trust_ready, "trust_ready");
    this.scopes = adminStringArray(value.scopes, "scopes");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new PairingPreflight(parseJSON(raw, "pairing preflight"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      state: this.state,
      hub_ura: this.hubURA,
      device_ura: this.deviceURA,
      pairing_required: this.pairingRequired,
      trust_ready: this.trustReady,
      scopes: [...this.scopes],
      metadata: this.metadata,
    };
  }
}

export class PairingToken {
  constructor(fields) {
    const value = objectValue(fields, "pairing token");
    validateAdminKind(value, "pairing_token");
    this.profile = value.profile;
    this.kind = value.kind;
    this.tokenID = requiredAdminString(value.token_id, "token_id");
    this.token = requiredAdminString(value.token, "token");
    this.hubURA = requiredAdminURA(value.hub_ura, "hub_ura");
    this.deviceURA = requiredAdminURA(value.device_ura, "device_ura");
    this.state = requiredAdminString(value.state, "state");
    this.expiresUnixMS = adminPositiveInteger(value.expires_unix_ms, "expires_unix_ms");
    this.scopes = adminStringArray(value.scopes, "scopes");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new PairingToken(parseJSON(raw, "pairing token"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      token_id: this.tokenID,
      token: this.token,
      hub_ura: this.hubURA,
      device_ura: this.deviceURA,
      state: this.state,
      expires_unix_ms: this.expiresUnixMS,
      scopes: [...this.scopes],
      metadata: this.metadata,
    };
  }
}

export class DeviceCredential {
  constructor(fields) {
    const value = objectValue(fields, "device credential");
    validateAdminKind(value, "device_credential");
    this.profile = value.profile;
    this.kind = value.kind;
    this.credentialID = requiredAdminString(value.credential_id, "credential_id");
    this.deviceURA = requiredAdminURA(value.device_ura, "device_ura");
    this.hubURA = requiredAdminURA(value.hub_ura, "hub_ura");
    this.state = requiredAdminString(value.state, "state");
    this.issuedUnixMS = adminNonNegativeInteger(value.issued_unix_ms, "issued_unix_ms");
    this.expiresUnixMS = adminPositiveInteger(value.expires_unix_ms, "expires_unix_ms");
    this.scopes = adminStringArray(value.scopes, "scopes");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new DeviceCredential(parseJSON(raw, "device credential"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      credential_id: this.credentialID,
      device_ura: this.deviceURA,
      hub_ura: this.hubURA,
      state: this.state,
      issued_unix_ms: this.issuedUnixMS,
      expires_unix_ms: this.expiresUnixMS,
      scopes: [...this.scopes],
      metadata: this.metadata,
    };
  }
}

export class DeviceSession {
  constructor(fields) {
    const value = objectValue(fields, "device session");
    validateAdminKind(value, "device_session");
    this.profile = value.profile;
    this.kind = value.kind;
    this.sessionID = requiredAdminString(value.session_id, "session_id");
    this.deviceURA = requiredAdminURA(value.device_ura, "device_ura");
    this.hubURA = requiredAdminURA(value.hub_ura, "hub_ura");
    this.state = requiredAdminString(value.state, "state");
    this.sessionKind = requiredAdminString(value.session_kind, "session_kind");
    this.createdUnixMS = adminNonNegativeInteger(value.created_unix_ms, "created_unix_ms");
    this.expiresUnixMS = adminPositiveInteger(value.expires_unix_ms, "expires_unix_ms");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new DeviceSession(parseJSON(raw, "device session"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      session_id: this.sessionID,
      device_ura: this.deviceURA,
      hub_ura: this.hubURA,
      state: this.state,
      session_kind: this.sessionKind,
      created_unix_ms: this.createdUnixMS,
      expires_unix_ms: this.expiresUnixMS,
      metadata: this.metadata,
    };
  }
}

export class DeviceSessionPage {
  constructor(fields) {
    const value = objectValue(fields, "device session page");
    validateAdminKind(value, "device_sessions");
    this.profile = value.profile;
    this.kind = value.kind;
    this.state = requiredAdminString(value.state, "state");
    if (!Array.isArray(value.items)) {
      throw invalidAdminGateway("device session items must be an array");
    }
    this.items = value.items.map((item) => new DeviceSession(item));
    this.nextCursor = value.next_cursor ?? null;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new DeviceSessionPage(parseJSON(raw, "device session page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      state: this.state,
      items: this.items.map((item) => item.toJSON()),
      next_cursor: this.nextCursor,
      metadata: this.metadata,
    };
  }
}

export class AdminClient {
  constructor(transport) {
    if (!transport || typeof transport.buildAgentListInvocation !== "function") {
      throw invalidSDK("admin gateway transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildAgentListInvocation(request) {
    return this.buildInvocation("buildAgentListInvocation", adminBaseRequest(request, "admin agent list request"));
  }

  async buildAgentStartInvocation(request) {
    return this.buildInvocation("buildAgentStartInvocation", adminAgentStartRequest(request));
  }

  async buildAgentStopInvocation(request) {
    return this.buildInvocation("buildAgentStopInvocation", adminAgentStopRequest(request));
  }

  async buildAgentRefreshInvocation(request) {
    return this.buildInvocation("buildAgentRefreshInvocation", adminAgentRefreshRequest(request));
  }

  async buildSessionListInvocation(request) {
    return this.buildInvocation("buildSessionListInvocation", adminSessionListRequest(request));
  }

  async buildInvocation(method, payload) {
    return InvocationDraft.fromJSON(await callRaw(this.requireOpen(), method, payload));
  }

  async gatewayStatus(request = {}) {
    return GatewayStatus.fromJSON(await callRaw(this.requireOpen(), "gatewayStatus", adminGatewayStatusRequest(request)));
  }

  async listAgents(request) {
    return AdminAgentPage.fromJSON(await callRaw(this.requireOpen(), "listAgents", adminBaseRequest(request, "admin agent list request")));
  }

  async startAgent(request) {
    return AdminGatewayResult.fromJSON(await callRaw(this.requireOpen(), "startAgent", adminAgentStartRequest(request)));
  }

  async stopAgent(request) {
    return AdminGatewayResult.fromJSON(await callRaw(this.requireOpen(), "stopAgent", adminAgentStopRequest(request)));
  }

  async refreshAgent(request) {
    return AdminGatewayResult.fromJSON(await callRaw(this.requireOpen(), "refreshAgent", adminAgentRefreshRequest(request)));
  }

  async listSessions(request) {
    return DeviceSessionPage.fromJSON(await callRaw(this.requireOpen(), "listSessions", adminSessionListRequest(request)));
  }

  async pairingPreflight(request) {
    return PairingPreflight.fromJSON(await callRaw(this.requireOpen(), "pairingPreflight", adminPairingPreflightRequest(request)));
  }

  async createPairing(request) {
    return PairingToken.fromJSON(await callRaw(this.requireOpen(), "createPairing", adminCreatePairingRequest(request)));
  }

  async validatePairing(request) {
    return DeviceCredential.fromJSON(await callRaw(this.requireOpen(), "validatePairing", adminValidatePairingRequest(request)));
  }

  async createDeviceSession(request) {
    return DeviceSession.fromJSON(await callRaw(this.requireOpen(), "createDeviceSession", adminCreateDeviceSessionRequest(request)));
  }

  async listDeviceSessions(request) {
    return this.listSessions(request);
  }

  async deleteDeviceSession(request) {
    return AdminGatewayResult.fromJSON(await callRaw(this.requireOpen(), "deleteDeviceSession", adminDeleteDeviceSessionRequest(request)));
  }

  async projectGatewayStatus(value) {
    return GatewayStatus.fromJSON(await callAdminProjection(this.requireOpen(), "projectGatewayStatus", value));
  }

  async projectAgentRecords(value) {
    return AdminAgentPage.fromJSON(await callAdminProjection(this.requireOpen(), "projectAgentRecords", value));
  }

  async projectAgentLifecycleResult(value) {
    return AdminGatewayResult.fromJSON(await callAdminProjection(this.requireOpen(), "projectAgentLifecycleResult", value));
  }

  async projectPairingPreflight(value) {
    return PairingPreflight.fromJSON(await callAdminProjection(this.requireOpen(), "projectPairingPreflight", value));
  }

  async projectPairingToken(value) {
    return PairingToken.fromJSON(await callAdminProjection(this.requireOpen(), "projectPairingToken", value));
  }

  async projectDeviceCredential(value) {
    return DeviceCredential.fromJSON(await callAdminProjection(this.requireOpen(), "projectDeviceCredential", value));
  }

  async projectDeviceSession(value) {
    return DeviceSession.fromJSON(await callAdminProjection(this.requireOpen(), "projectDeviceSession", value));
  }

  async projectDeviceSessionPage(value) {
    return DeviceSessionPage.fromJSON(await callAdminProjection(this.requireOpen(), "projectDeviceSessionPage", value));
  }

  async projectDeviceAdminResult(value) {
    return AdminGatewayResult.fromJSON(await callAdminProjection(this.requireOpen(), "projectDeviceAdminResult", value));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("admin gateway client is closed");
    }
    return this.transport;
  }
}

export class WrapperFileRecord {
  constructor(fields) {
    const value = objectValue(fields, "wrapper file record");
    validateWrapperKind(value, "file_record");
    this.profile = value.profile;
    this.kind = value.kind;
    this.fileRef = requiredWrapperURA(value.file_ref, "file_ref");
    this.ownerURA = requiredWrapperURA(value.owner_ura, "owner_ura");
    this.contentType = requiredWrapperString(value.content_type, "content_type");
    this.sizeBytes = wrapperOptionalNonNegativeInteger(value.size_bytes, "size_bytes");
    this.contentHash = wrapperOptionalString(value.content_hash, "content_hash");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
  }

  static fromJSON(raw) {
    return new WrapperFileRecord(parseJSON(raw, "wrapper file record"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      file_ref: this.fileRef,
      owner_ura: this.ownerURA,
      content_type: this.contentType,
      size_bytes: this.sizeBytes,
      content_hash: this.contentHash,
      metadata: this.metadata,
    };
  }
}

class WrapperSessionRecord {
  constructor(fields, kind, refField) {
    const value = objectValue(fields, `wrapper ${kind}`);
    validateWrapperKind(value, kind);
    this.profile = value.profile;
    this.kind = value.kind;
    this.sessionID = requiredWrapperString(value.session_id, "session_id");
    this.ownerURA = requiredWrapperURA(value.owner_ura, "owner_ura");
    this.state = requiredWrapperString(value.state, "state");
    this.refField = refField;
    this.refValue = wrapperOptionalString(value[refField], refField);
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      session_id: this.sessionID,
      owner_ura: this.ownerURA,
      state: this.state,
      [this.refField]: this.refValue,
      metadata: this.metadata,
    };
  }
}

export class WrapperTerminalSession extends WrapperSessionRecord {
  constructor(fields) {
    super(fields, "terminal_session", "terminal_ref");
    this.terminalRef = this.refValue;
  }

  static fromJSON(raw) {
    return new WrapperTerminalSession(parseJSON(raw, "wrapper terminal session"));
  }
}

export class WrapperRemoteDesktopSession extends WrapperSessionRecord {
  constructor(fields) {
    super(fields, "remote_desktop_session", "display_ref");
    this.displayRef = this.refValue;
  }

  static fromJSON(raw) {
    return new WrapperRemoteDesktopSession(parseJSON(raw, "wrapper remote desktop session"));
  }
}

export class WrapperBrowserSession extends WrapperSessionRecord {
  constructor(fields) {
    super(fields, "browser_session", "browser_ref");
    this.browserRef = this.refValue;
  }

  static fromJSON(raw) {
    return new WrapperBrowserSession(parseJSON(raw, "wrapper browser session"));
  }
}

export class WrapperMediaSession extends WrapperSessionRecord {
  constructor(fields) {
    const value = objectValue(fields, "wrapper media session");
    super(value, "media_session", "stream_ref");
    this.mediaKind = requiredWrapperString(value.media_kind, "media_kind");
    this.streamRef = this.refValue;
  }

  static fromJSON(raw) {
    return new WrapperMediaSession(parseJSON(raw, "wrapper media session"));
  }

  toJSON() {
    return {
      ...super.toJSON(),
      media_kind: this.mediaKind,
    };
  }
}

export class WrapperClient {
  constructor(transport) {
    if (!transport || typeof transport.projectFileRecord !== "function") {
      throw invalidSDK("wrapper transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async projectFileRecord(value) {
    return WrapperFileRecord.fromJSON(await callWrapperProjection(this.requireOpen(), "projectFileRecord", value));
  }

  async projectTerminalSession(value) {
    return WrapperTerminalSession.fromJSON(await callWrapperProjection(this.requireOpen(), "projectTerminalSession", value));
  }

  async projectRemoteDesktopSession(value) {
    return WrapperRemoteDesktopSession.fromJSON(await callWrapperProjection(this.requireOpen(), "projectRemoteDesktopSession", value));
  }

  async projectBrowserSession(value) {
    return WrapperBrowserSession.fromJSON(await callWrapperProjection(this.requireOpen(), "projectBrowserSession", value));
  }

  async projectMediaSession(value) {
    return WrapperMediaSession.fromJSON(await callWrapperProjection(this.requireOpen(), "projectMediaSession", value));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("wrapper client is closed");
    }
    return this.transport;
  }
}

export class SurfacePageRecord {
  constructor(fields) {
    const value = objectValue(fields, "surface page record");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    if (this.profile !== SURFACE_PROFILE || this.kind !== "page_record") {
      throw invalidSurface("invalid surface page record projection");
    }
    this.pageId = requiredSurfaceProjectID(value.page_id, "page_id");
    this.ownerURA = requiredSurfaceString(value.owner_ura, "owner_ura");
    this.surfaceRef = requiredSurfaceRef(value.surface_ref, "surface_ref");
    this.publicRef = surfaceOptionalString(value.public_ref, "public_ref");
    this.status = surfaceOptionalString(value.status, "status");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfacePageRecord(parseJSON(raw, "surface page record"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      page_id: this.pageId,
      owner_ura: this.ownerURA,
      surface_ref: this.surfaceRef,
      public_ref: this.publicRef,
      status: this.status,
      metadata: this.metadata,
    };
  }
}

export class SurfacePagePage {
  constructor(fields) {
    const value = objectValue(fields, "surface page page");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    this.itemKind = requiredSurfaceString(value.item_kind, "item_kind");
    this.source = requiredSurfaceString(value.source, "source");
    if (
      this.profile !== SURFACE_PROFILE ||
      this.kind !== "surface_page_page" ||
      this.itemKind !== "page_record" ||
      this.source !== "pages_read_model"
    ) {
      throw invalidSurface("invalid surface page projection");
    }
    if (!Array.isArray(value.items)) {
      throw invalidSurface("items must be an array");
    }
    this.items = value.items.map((item) => new SurfacePageRecord(item));
    this.nextCursor = surfaceOptionalString(value.next_cursor, "next_cursor");
    this.limit = surfacePositiveBoundedInteger(value.limit, "limit", MAX_SURFACE_PAGE_SIZE);
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfacePagePage(parseJSON(raw, "surface page page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      item_kind: this.itemKind,
      items: this.items.map((item) => item.toJSON()),
      next_cursor: this.nextCursor,
      limit: this.limit,
      source: this.source,
      metadata: this.metadata,
    };
  }
}

export class SurfaceManifest {
  constructor(fields) {
    const value = objectValue(fields, "surface manifest");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    if (this.profile !== SURFACE_PROFILE || this.kind !== "surface_manifest") {
      throw invalidSurface("invalid surface manifest projection");
    }
    this.pageId = requiredSurfaceProjectID(value.page_id, "page_id");
    this.ownerURA = requiredSurfaceString(value.owner_ura, "owner_ura");
    this.surfaceRef = requiredSurfaceRef(value.surface_ref, "surface_ref");
    this.publicRef = requiredSurfaceString(value.public_ref, "public_ref");
    this.page = new SurfacePageRecord(objectValue(value.page, "page"));
    this.entrypoint = objectValue(value.entrypoint, "entrypoint");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfaceManifest(parseJSON(raw, "surface manifest"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      page_id: this.pageId,
      owner_ura: this.ownerURA,
      surface_ref: this.surfaceRef,
      public_ref: this.publicRef,
      page: this.page.toJSON(),
      entrypoint: this.entrypoint,
      metadata: this.metadata,
    };
  }
}

export class SurfacePublicPageRef {
  constructor(fields) {
    const value = objectValue(fields, "surface public page ref");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    if (this.profile !== SURFACE_PROFILE || this.kind !== "public_page_ref") {
      throw invalidSurface("invalid surface public page ref projection");
    }
    this.pageId = requiredSurfaceProjectID(value.page_id, "page_id");
    this.ownerURA = requiredSurfaceString(value.owner_ura, "owner_ura");
    this.surfaceRef = requiredSurfaceRef(value.surface_ref, "surface_ref");
    this.publicRef = requiredSurfaceString(value.public_ref, "public_ref");
    this.routeKind = requiredSurfaceString(value.route_kind, "route_kind");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfacePublicPageRef(parseJSON(raw, "surface public page ref"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      page_id: this.pageId,
      owner_ura: this.ownerURA,
      surface_ref: this.surfaceRef,
      public_ref: this.publicRef,
      route_kind: this.routeKind,
      metadata: this.metadata,
    };
  }
}

export class SurfaceMutationResult {
  constructor(fields) {
    const value = objectValue(fields, "surface mutation result");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    this.operation = requiredSurfaceString(value.operation, "operation");
    if (
      this.profile !== SURFACE_PROFILE ||
      this.kind !== "surface_mutation_result" ||
      this.operation !== "delete"
    ) {
      throw invalidSurface("invalid surface mutation result projection");
    }
    this.pageId = requiredSurfaceProjectID(value.page_id, "page_id");
    this.removed = surfaceBoolean(value.removed, "removed");
    this.state = requiredSurfaceString(value.state, "state");
    if (!["deleted", "unknown"].includes(this.state)) {
      throw invalidSurface("invalid surface mutation state");
    }
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfaceMutationResult(parseJSON(raw, "surface mutation result"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      operation: this.operation,
      page_id: this.pageId,
      removed: this.removed,
      state: this.state,
      metadata: this.metadata,
    };
  }
}

export class SurfaceHealthCheck {
  constructor(fields) {
    const value = objectValue(fields, "surface health check");
    this.name = requiredSurfaceString(value.name, "name");
    this.state = requiredSurfaceString(value.state, "state");
    this.ready = surfaceBoolean(value.ready, "ready");
    this.message = surfaceOptionalString(value.message, "message");
    this.latencyMS = surfaceOptionalNonNegativeInteger(value.latency_ms, "latency_ms") ?? 0;
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
  }

  toJSON() {
    return {
      name: this.name,
      state: this.state,
      ready: this.ready,
      message: this.message,
      latency_ms: this.latencyMS,
      metadata: this.metadata,
    };
  }
}

export class SurfaceHealth {
  constructor(fields) {
    const value = objectValue(fields, "surface health");
    this.profile = requiredSurfaceString(value.profile, "profile");
    this.kind = requiredSurfaceString(value.kind, "kind");
    if (this.profile !== SURFACE_PROFILE || this.kind !== "surface_health") {
      throw invalidSurface("invalid surface health projection");
    }
    this.state = requiredSurfaceString(value.state, "state");
    this.ready = surfaceBoolean(value.ready, "ready");
    this.ownerURA = requiredSurfaceString(value.owner_ura, "owner_ura");
    this.surfaceRef = requiredSurfaceRef(value.surface_ref, "surface_ref");
    this.descriptorRef = requiredSurfaceString(value.descriptor_ref, "descriptor_ref");
    this.descriptorVersion = requiredSurfaceString(value.descriptor_version, "descriptor_version");
    this.pageCount = surfaceNonNegativeInteger(value.page_count, "page_count");
    if (!Array.isArray(value.checks)) {
      throw invalidSurface("checks must be an array");
    }
    this.checks = value.checks.map((check) => new SurfaceHealthCheck(check));
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new SurfaceHealth(parseJSON(raw, "surface health"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      state: this.state,
      ready: this.ready,
      owner_ura: this.ownerURA,
      surface_ref: this.surfaceRef,
      descriptor_ref: this.descriptorRef,
      descriptor_version: this.descriptorVersion,
      page_count: this.pageCount,
      checks: this.checks.map((check) => check.toJSON()),
      metadata: this.metadata,
    };
  }
}

export class SurfaceClient {
  constructor(transport) {
    if (!transport || typeof transport.buildListPagesInvocation !== "function") {
      throw invalidSDK("surface transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildListPagesInvocation(request) {
    return this.buildInvocation("buildListPagesInvocation", surfaceListPagesRequest(request));
  }

  async buildCreatePageInvocation(request) {
    return this.buildInvocation("buildCreatePageInvocation", surfaceCreatePageRequest(request));
  }

  async buildDeletePageInvocation(request) {
    return this.buildInvocation("buildDeletePageInvocation", surfaceDeletePageRequest(request));
  }

  async buildManifestInvocation(request) {
    return this.buildInvocation("buildManifestInvocation", surfaceManifestRequest(request));
  }

  async buildHealthInvocation(request) {
    return this.buildInvocation("buildHealthInvocation", surfaceHealthRequest(request));
  }

  async buildInvocation(method, payload) {
    return InvocationDraft.fromJSON(await callRaw(this.requireOpen(), method, payload));
  }

  async listPages(request) {
    return SurfacePagePage.fromJSON(
      await callRaw(this.requireOpen(), "listPages", surfaceListPagesRequest(request)),
    );
  }

  async createPage(request) {
    return SurfacePageRecord.fromJSON(
      await callRaw(this.requireOpen(), "createPage", surfaceCreatePageRequest(request)),
    );
  }

  async deletePage(request) {
    return SurfaceMutationResult.fromJSON(
      await callRaw(this.requireOpen(), "deletePage", surfaceDeletePageRequest(request)),
    );
  }

  async surfaceManifest(request) {
    return SurfaceManifest.fromJSON(
      await callRaw(this.requireOpen(), "surfaceManifest", surfaceManifestRequest(request)),
    );
  }

  async publicPageRef(request) {
    const payload = surfaceRequest(request, ["page"], "surface public page ref request");
    const page = payload.page instanceof SurfacePageRecord
      ? payload.page.toJSON()
      : new SurfacePageRecord(objectValue(payload.page, "page")).toJSON();
    return SurfacePublicPageRef.fromJSON(
      await callRaw(this.requireOpen(), "publicPageRef", { page }),
    );
  }

  async surfaceHealth(request) {
    return SurfaceHealth.fromJSON(
      await callRaw(this.requireOpen(), "surfaceHealth", surfaceHealthRequest(request)),
    );
  }

  async surfaceStatus(request) {
    return this.surfaceHealth(request);
  }

  async projectPageRecord(value) {
    return SurfacePageRecord.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectPageRecord", value));
  }

  async projectPagePage(value) {
    return SurfacePagePage.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectPagePage", value));
  }

  async projectManifest(value) {
    return SurfaceManifest.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectManifest", value));
  }

  async projectPublicPageRef(value) {
    return SurfacePublicPageRef.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectPublicPageRef", value));
  }

  async projectMutationResult(value) {
    return SurfaceMutationResult.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectMutationResult", value));
  }

  async projectHealth(value) {
    return SurfaceHealth.fromJSON(await callSurfaceProjection(this.requireOpen(), "projectHealth", value));
  }

  async projectStatus(value) {
    return this.projectHealth(value);
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("surface client is closed");
    }
    return this.transport;
  }
}

export class CompatibilityModel {
  constructor(fields) {
    const value = objectValue(fields, "compatibility model");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (this.profile !== COMPATIBILITY_PROFILE || this.kind !== "model" || this.object !== "model") {
      throw invalidCompatibility("invalid compatibility model projection");
    }
    this.id = requiredAbilityURA(value.id, "id");
    this.created = compatibilityNonNegativeInteger(value.created, "created");
    this.ownedBy = requiredCompatibilityString(value.owned_by, "owned_by");
    this.abilityRef = requiredAbilityURA(value.ability_ref, "ability_ref");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityModel(parseJSON(raw, "compatibility model"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      id: this.id,
      object: this.object,
      created: this.created,
      owned_by: this.ownedBy,
      ability_ref: this.abilityRef,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityModelPage {
  constructor(fields) {
    const value = objectValue(fields, "compatibility model page");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (this.profile !== COMPATIBILITY_PROFILE || this.kind !== "model_page" || this.object !== "list") {
      throw invalidCompatibility("invalid compatibility model page projection");
    }
    if (!Array.isArray(value.data)) {
      throw invalidCompatibility("model page data must be an array");
    }
    this.data = value.data.map((item) => new CompatibilityModel(item));
    this.nextCursor = compatibilityOptionalString(value.next_cursor, "next_cursor");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityModelPage(parseJSON(raw, "compatibility model page"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      object: this.object,
      data: this.data.map((item) => item.toJSON()),
      next_cursor: this.nextCursor,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityChatCompletion {
  constructor(fields) {
    const value = objectValue(fields, "compatibility chat completion");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (
      this.profile !== COMPATIBILITY_PROFILE ||
      this.kind !== "chat_completion" ||
      this.object !== "chat.completion"
    ) {
      throw invalidCompatibility("invalid compatibility chat completion projection");
    }
    this.id = requiredCompatibilityString(value.id, "id");
    this.created = compatibilityNonNegativeInteger(value.created, "created");
    this.model = requiredAbilityURA(value.model, "model");
    this.choices = compatibilityObjectArray(value.choices, "choices");
    this.usage = objectValue(value.usage, "usage");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityChatCompletion(parseJSON(raw, "compatibility chat completion"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      id: this.id,
      object: this.object,
      created: this.created,
      model: this.model,
      choices: this.choices,
      usage: this.usage,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityChatCompletionChunk {
  constructor(fields) {
    const value = objectValue(fields, "compatibility chat completion chunk");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (
      this.profile !== COMPATIBILITY_PROFILE ||
      this.kind !== "chat_completion_chunk" ||
      this.object !== "chat.completion.chunk"
    ) {
      throw invalidCompatibility("invalid compatibility chat chunk projection");
    }
    this.id = requiredCompatibilityString(value.id, "id");
    this.created = compatibilityNonNegativeInteger(value.created, "created");
    this.model = requiredAbilityURA(value.model, "model");
    this.choices = compatibilityObjectArray(value.choices, "choices");
    this.usage = value.usage ?? null;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      id: this.id,
      object: this.object,
      created: this.created,
      model: this.model,
      choices: this.choices,
      usage: this.usage,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityChatCompletionStream {
  constructor(fields) {
    const value = objectValue(fields, "compatibility chat completion stream");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    if (
      this.profile !== COMPATIBILITY_PROFILE ||
      this.kind !== "chat_completion_stream" ||
      value.stream !== true ||
      value.done_sentinel !== "[DONE]"
    ) {
      throw invalidCompatibility("invalid compatibility chat stream projection");
    }
    if (!Array.isArray(value.items)) {
      throw invalidCompatibility("stream items must be an array");
    }
    this.stream = true;
    this.items = value.items.map((item) => new CompatibilityChatCompletionChunk(item));
    this.doneSentinel = value.done_sentinel;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityChatCompletionStream(parseJSON(raw, "compatibility chat completion stream"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      stream: this.stream,
      items: this.items.map((item) => item.toJSON()),
      done_sentinel: this.doneSentinel,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityFile {
  constructor(fields) {
    const value = objectValue(fields, "compatibility file");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (this.profile !== COMPATIBILITY_PROFILE || this.kind !== "file" || this.object !== "file") {
      throw invalidCompatibility("invalid compatibility file projection");
    }
    this.id = requiredCompatibilityString(value.id, "id");
    this.bytes = compatibilityNonNegativeInteger(value.bytes, "bytes");
    this.createdAt = compatibilityNonNegativeInteger(value.created_at, "created_at");
    this.filename = requiredCompatibilityString(value.filename, "filename");
    this.purpose = requiredCompatibilityString(value.purpose, "purpose");
    this.status = requiredCompatibilityString(value.status, "status");
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityFile(parseJSON(raw, "compatibility file"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      id: this.id,
      object: this.object,
      bytes: this.bytes,
      created_at: this.createdAt,
      filename: this.filename,
      purpose: this.purpose,
      status: this.status,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityFileDeleteResult {
  constructor(fields) {
    const value = objectValue(fields, "compatibility file delete result");
    this.profile = requiredCompatibilityString(value.profile, "profile");
    this.kind = requiredCompatibilityString(value.kind, "kind");
    this.object = requiredCompatibilityString(value.object, "object");
    if (
      this.profile !== COMPATIBILITY_PROFILE ||
      this.kind !== "file_delete_result" ||
      this.object !== "file" ||
      value.deleted !== true
    ) {
      throw invalidCompatibility("invalid compatibility file delete projection");
    }
    this.id = requiredCompatibilityString(value.id, "id");
    this.deleted = true;
    this.metadata = objectValue(value.metadata, "metadata");
  }

  static fromJSON(raw) {
    return new CompatibilityFileDeleteResult(parseJSON(raw, "compatibility file delete result"));
  }

  toJSON() {
    return {
      profile: this.profile,
      kind: this.kind,
      id: this.id,
      object: this.object,
      deleted: this.deleted,
      metadata: this.metadata,
    };
  }
}

export class CompatibilityClient {
  constructor(transport) {
    if (!transport || typeof transport.buildListModelsInvocation !== "function") {
      throw invalidSDK("compatibility transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildListModelsInvocation(request) {
    return this.buildInvocation("buildListModelsInvocation", compatibilityListModelsRequest(request));
  }

  async buildChatCompletionInvocation(request) {
    return this.buildInvocation("buildChatCompletionInvocation", compatibilityChatCompletionRequest(request, false));
  }

  async buildStreamChatCompletionInvocation(request) {
    return this.buildInvocation("buildStreamChatCompletionInvocation", compatibilityChatCompletionRequest(request, true));
  }

  async buildFileUploadInvocation(request) {
    return this.buildInvocation("buildFileUploadInvocation", compatibilityFileUploadRequest(request));
  }

  async buildFileRetrieveInvocation(request) {
    return this.buildInvocation("buildFileRetrieveInvocation", compatibilityFileRequest(request));
  }

  async buildFileDeleteInvocation(request) {
    return this.buildInvocation("buildFileDeleteInvocation", compatibilityFileDeleteRequest(request));
  }

  async buildInvocation(method, payload) {
    return InvocationDraft.fromJSON(await callRaw(this.requireOpen(), method, payload));
  }

  async listModels(request) {
    return CompatibilityModelPage.fromJSON(
      await callRaw(this.requireOpen(), "listModels", compatibilityListModelsRequest(request)),
    );
  }

  async chatCompletions(request) {
    return CompatibilityChatCompletion.fromJSON(
      await callRaw(this.requireOpen(), "chatCompletions", compatibilityChatCompletionRequest(request, false)),
    );
  }

  async streamChatCompletions(request) {
    return CompatibilityChatCompletionStream.fromJSON(
      await callRaw(this.requireOpen(), "streamChatCompletions", compatibilityChatCompletionRequest(request, true)),
    );
  }

  async uploadFile(request) {
    return CompatibilityFile.fromJSON(
      await callRaw(this.requireOpen(), "uploadFile", compatibilityFileUploadRequest(request)),
    );
  }

  async getFile(request) {
    return CompatibilityFile.fromJSON(
      await callRaw(this.requireOpen(), "getFile", compatibilityFileRequest(request)),
    );
  }

  async deleteFile(request) {
    return CompatibilityFileDeleteResult.fromJSON(
      await callRaw(this.requireOpen(), "deleteFile", compatibilityFileDeleteRequest(request)),
    );
  }

  async projectModelPage(value) {
    return CompatibilityModelPage.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectModelPage", value));
  }

  async projectChatCompletion(value) {
    return CompatibilityChatCompletion.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectChatCompletion", value));
  }

  async projectChatStream(value) {
    return CompatibilityChatCompletionStream.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectChatStream", value));
  }

  async projectFileUpload(value) {
    return CompatibilityFile.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectFileUpload", value));
  }

  async projectFile(value) {
    return CompatibilityFile.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectFile", value));
  }

  async projectFileDeleteResult(value) {
    return CompatibilityFileDeleteResult.fromJSON(await callCompatibilityProjection(this.requireOpen(), "projectFileDeleteResult", value));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("compatibility client is closed");
    }
    return this.transport;
  }
}

export class ReceiptRef {
  constructor(fields) {
    const value = objectValue(fields, "receipt ref");
    this.receiptURA = cleanRequiredString(value.receipt_ura, "receipt_ura");
    this.receiptHashHex = normalizeReceiptHashHex(value.receipt_hash_hex, "receipt_hash_hex");
    this.invocationId = cleanOptionalString(value.invocation_id ?? "", "invocation_id");
    this.prevReceiptHashHex = value.prev_receipt_hash_hex
      ? normalizeReceiptHashHex(value.prev_receipt_hash_hex, "prev_receipt_hash_hex")
      : "";
    this.index = optionalNonNegativeInteger(value.index, "index");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
  }

  static fromJSON(raw) {
    return new ReceiptRef(parseJSON(raw, "receipt ref"));
  }

  toJSON() {
    const value = {
      receipt_ura: this.receiptURA,
      receipt_hash_hex: this.receiptHashHex,
    };
    if (this.invocationId) {
      value.invocation_id = this.invocationId;
    }
    if (this.prevReceiptHashHex) {
      value.prev_receipt_hash_hex = this.prevReceiptHashHex;
    }
    if (this.index !== null) {
      value.index = this.index;
    }
    if (Object.keys(this.metadata).length > 0) {
      value.metadata = this.metadata;
    }
    return value;
  }

  toJSONString() {
    return JSON.stringify(this.toJSON());
  }

  async causalContext(client) {
    if (!client || typeof client.causalContext !== "function") {
      throw invalidReceipt("receipt client is required");
    }
    return client.causalContext(this.toJSON());
  }
}

export class ReceiptChain {
  constructor(receipts) {
    if (!Array.isArray(receipts) || receipts.length === 0) {
      throw invalidReceipt("receipt chain requires at least one receipt");
    }
    this.receipts = receipts.map((receipt) =>
      receipt instanceof ReceiptRef ? receipt : new ReceiptRef(receipt),
    );
  }

  toJSON() {
    return this.receipts.map((receipt) => receipt.toJSON());
  }

  async verifyContinuity(client, metadata = {}) {
    if (!client || typeof client.verifyChain !== "function") {
      throw invalidReceipt("receipt client is required");
    }
    return client.verifyChain({ receipts: this.toJSON(), metadata });
  }
}

export class ReceiptClient {
  constructor(transport) {
    if (!transport || typeof transport.fetch !== "function") {
      throw invalidSDK("receipt transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async fetch(request) {
    const payload = receiptFetchRequest(request);
    return callJSON(this.requireOpen(), "fetch", payload, "receipt fetch");
  }

  async buildFetchInvocation(request) {
    const payload = receiptFetchRequest(request);
    return InvocationDraft.fromJSON(
      await callRaw(this.requireOpen(), "buildFetchInvocation", payload),
    );
  }

  async buildListHistoryInvocation(request) {
    return this.buildHistoryInvocation("buildListHistoryInvocation", request);
  }

  async buildGetHistoryInvocation(request) {
    return this.buildHistoryInvocation("buildGetHistoryInvocation", request);
  }

  async buildTraceInvocation(request) {
    return this.buildHistoryInvocation("buildTraceInvocation", request);
  }

  async buildHistoryInvocation(method, request) {
    return InvocationDraft.fromJSON(
      await callRaw(this.requireOpen(), method, receiptHistoryRequest(request)),
    );
  }

  async listHistory(request) {
    return callJSON(this.requireOpen(), "listHistory", receiptHistoryRequest(request), "receipt list history");
  }

  async getHistory(request) {
    return callJSON(this.requireOpen(), "getHistory", receiptHistoryRequest(request), "receipt get history");
  }

  async getTrace(request) {
    return callJSON(this.requireOpen(), "getTrace", receiptHistoryRequest(request), "receipt trace");
  }

  async project(receiptJSON) {
    return parseJSON(await callReceiptJSON(this.requireOpen(), "project", receiptJSON), "receipt projection");
  }

  async verify(receiptJSON) {
    return parseJSON(await callReceiptJSON(this.requireOpen(), "verify", receiptJSON), "receipt verification");
  }

  async verifyChain(request) {
    const payload = receiptChainRequest(request);
    return callJSON(this.requireOpen(), "verifyChain", payload, "receipt chain verification");
  }

  async causalRef(receiptJSON) {
    return parseJSON(await callReceiptJSON(this.requireOpen(), "causalRef", receiptJSON), "receipt causal ref");
  }

  async causalContext(receiptJSON) {
    const ref = await this.causalRef(receiptJSON);
    return objectValue(ref.causal_context, "causal_context");
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("receipt client is closed");
    }
    return this.transport;
  }
}

export class PublicationClient {
  constructor(transport) {
    if (!transport || typeof transport.buildResourceRef !== "function") {
      throw invalidSDK("publication transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  async buildLocalResourceRef(request) {
    const payload = localResourceRefRequest(request);
    return callJSON(this.requireOpen(), "buildResourceRef", payload, "publication resource_ref");
  }

  async validatePackage(request) {
    const payload = packageValidationRequest(request);
    return callJSON(this.requireOpen(), "validatePackage", payload, "publication package validation");
  }

  async deployAbility(request) {
    const payload = abilityDeployRequest(request);
    return callJSON(this.requireOpen(), "deployAbility", payload, "publication deploy result");
  }

  async buildDeployInvocation(request) {
    const payload = abilityDeployRequest(request);
    return InvocationDraft.fromJSON(
      await callRaw(this.requireOpen(), "buildDeployInvocation", payload),
    );
  }

  async installPlugin(request) {
    const payload = pluginInstallRequest(request);
    return callJSON(this.requireOpen(), "installPlugin", payload, "publication plugin install");
  }

  async listAbilities(request) {
    const payload = publishedAbilityQuery(request);
    return callJSON(this.requireOpen(), "listAbilities", payload, "publication ability page");
  }

  async showAbility(request) {
    const payload = showAbilityRequest(request);
    return callJSON(this.requireOpen(), "showAbility", payload, "publication ability");
  }

  async enableAbilityImpl(request) {
    const payload = abilityImplLifecycleRequest(request);
    return callJSON(this.requireOpen(), "enableAbilityImpl", payload, "publication enable ability impl");
  }

  async disableAbilityImpl(request) {
    const payload = abilityImplLifecycleRequest(request);
    return callJSON(this.requireOpen(), "disableAbilityImpl", payload, "publication disable ability impl");
  }

  async buildUnpublishInvocation(request) {
    const payload = unpublishAbilityRequest(request);
    return InvocationDraft.fromJSON(
      await callRaw(this.requireOpen(), "buildUnpublishInvocation", payload),
    );
  }

  async unpublishAbility(request) {
    const payload = unpublishAbilityRequest(request);
    return callJSON(this.requireOpen(), "unpublishAbility", payload, "publication unpublish result");
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("publication client is closed");
    }
    return this.transport;
  }
}

export class HostStreamHashState {
  constructor(fields) {
    const value = objectValue(fields, "host stream hash state");
    this.algorithm = requiredHostBindingString(value.algorithm, "algorithm");
    if (this.algorithm !== HOST_STREAM_HASH_ALGORITHM) {
      throw invalidHostBinding("invalid host stream hash algorithm");
    }
    this.outputHash = normalizeHostOutputHash(value.output_hash, "output_hash");
    this.frames = hostNonNegativeInteger(value.frames, "frames");
    this.lastSeq = hostOptionalNonNegativeInteger(value.last_seq, "last_seq");
    this.canonicalJSON = cleanOptionalString(value.canonical_json ?? "", "canonical_json");
    validateHostHashState(this);
  }

  static initial() {
    return new HostStreamHashState({
      algorithm: HOST_STREAM_HASH_ALGORITHM,
      output_hash: HOST_STREAM_EMPTY_OUTPUT_HASH,
      frames: 0,
      last_seq: null,
    });
  }

  static fromJSON(raw) {
    return new HostStreamHashState(parseJSON(raw, "host stream hash state"));
  }

  toJSON() {
    const value = {
      algorithm: this.algorithm,
      output_hash: this.outputHash,
      frames: this.frames,
      last_seq: this.lastSeq,
    };
    if (this.canonicalJSON) {
      value.canonical_json = this.canonicalJSON;
    }
    return value;
  }
}

export class LocalHostBindingTransport {
  constructor(descriptorRefCanonicalizer) {
    this.descriptorRefCanonicalizer = descriptorRefCanonicalizer;
    this.closed = false;
  }

  async buildHostStreamBinding(requestJSON) {
    this.requireOpen();
    const request = hostBindingRequest(parseJSON(requestJSON, "host stream binding request"));
    const descriptorRef = await this.canonicalDescriptorRef(request.descriptor_ref);
    request.descriptor_ref = descriptorRef;
    validateHostStreamBindingRequest(request);
    const cleanup = objectValue(request.cleanup ?? {}, "cleanup");
    const readiness = objectValue(
      request.readiness ?? { state: "declared", checked: false, endpoint_ready: null },
      "readiness",
    );
    const metadata = {
      ...objectValue(request.metadata ?? {}, "metadata"),
      profile: HOST_BINDING_PROFILE,
      frame_schema: HOST_STREAM_FRAME_SCHEMA,
      hash_algorithm: HOST_STREAM_HASH_ALGORITHM,
    };
    return JSON.stringify({
      binding_id: request.binding_id,
      descriptor_ref: request.descriptor_ref,
      endpoint: request.endpoint,
      frame_schema: request.frame_schema,
      cleanup,
      timeout_ms: request.timeout_ms ?? null,
      readiness,
      lifecycle: {
        endpoint_owner: "product_host",
        process_owner: "product_host",
        frame_contract_owner: "daemon_sdk",
      },
      metadata,
    });
  }

  async decodeRequest(envelopeJSON) {
    this.requireOpen();
    const envelope = parseJSON(envelopeJSON, "host stream envelope");
    const request = objectValue(envelope.request, "request");
    return JSON.stringify({
      function: requiredHostBindingString(request.fn, "fn"),
      args: Object.hasOwn(request, "args") ? request.args : null,
      call_id: requiredHostBindingString(request.call_id, "call_id"),
      caller: requiredHostBindingString(request.caller, "caller"),
      parent_receipt: request.parent_receipt ?? null,
      metadata: {
        wire: "host_stream_request_v1",
        frame_contract_owner: "daemon_sdk",
      },
    });
  }

  async encodeItem(requestJSON) {
    this.requireOpen();
    const request = objectValue(parseJSON(requestJSON, "host stream item request"), "host stream item request");
    const seq = hostNonNegativeInteger(request.seq, "seq");
    return JSON.stringify({
      frame_type: "item",
      seq,
      value: Object.hasOwn(request, "value") ? request.value : null,
      error: null,
      terminal: null,
      output_hash: null,
    });
  }

  async encodeError(requestJSON) {
    this.requireOpen();
    const request = objectValue(parseJSON(requestJSON, "host stream error request"), "host stream error request");
    const error = objectValue(request.error, "error");
    return JSON.stringify({
      frame_type: "error",
      seq: null,
      value: null,
      error,
      terminal: null,
      output_hash: null,
    });
  }

  async encodeTerminal(requestJSON) {
    this.requireOpen();
    const request = objectValue(parseJSON(requestJSON, "host stream terminal request"), "host stream terminal request");
    const summary = hostStreamTerminalSummary(request.summary);
    return JSON.stringify({
      frame_type: "terminal",
      seq: summary.frames,
      value: null,
      error: null,
      terminal: summary,
      output_hash: summary.output_hash,
    });
  }

  async foldOutputHash(requestJSON) {
    this.requireOpen();
    const request = objectValue(parseJSON(requestJSON, "host stream hash fold request"), "host stream hash fold request");
    const state = request.state instanceof HostStreamHashState
      ? request.state
      : new HostStreamHashState(objectValue(request.state, "state"));
    const seq = hostNonNegativeInteger(request.seq, "seq");
    validateHostHashFold(state, seq);
    const canonicalJSON = canonicalJSONString(request.value);
    const outputHash = foldHostOutputHash(state.outputHash, seq, canonicalJSON);
    return JSON.stringify({
      algorithm: HOST_STREAM_HASH_ALGORITHM,
      output_hash: outputHash,
      frames: state.frames + 1,
      last_seq: seq,
      canonical_json: canonicalJSON,
    });
  }

  async close() {
    this.closed = true;
  }

  requireOpen() {
    if (this.closed) {
      throw invalidHostBinding("host binding transport is closed");
    }
  }

  async canonicalDescriptorRef(descriptorRef) {
    if (typeof this.descriptorRefCanonicalizer !== "function") {
      throw invalidHostBinding("descriptor_ref canonicalizer is required");
    }
    const canonical = await this.descriptorRefCanonicalizer(descriptorRef);
    return requiredHostBindingString(canonical, "descriptor_ref");
  }
}

export class HostBindingClient {
  constructor(transport, lifecycleProvider = null) {
    if (
      !transport ||
      typeof transport.buildHostStreamBinding !== "function" ||
      typeof transport.decodeRequest !== "function" ||
      typeof transport.encodeItem !== "function" ||
      typeof transport.encodeError !== "function" ||
      typeof transport.encodeTerminal !== "function" ||
      typeof transport.foldOutputHash !== "function"
    ) {
      throw invalidSDK("host binding transport is required");
    }
    this.transport = transport;
    this.lifecycleProvider = lifecycleProvider;
    this.closed = false;
  }

  async buildHostStreamBinding(request) {
    const payload = hostBindingRequest(request);
    return callJSON(this.requireOpen(), "buildHostStreamBinding", payload, "host stream binding");
  }

  async decodeRequest(envelope) {
    const payload = hostStreamEnvelope(envelope);
    return callJSON(this.requireOpen(), "decodeRequest", payload, "host stream request");
  }

  async encodeItem(seq, value) {
    hostNonNegativeInteger(seq, "seq");
    return callJSON(this.requireOpen(), "encodeItem", { seq, value }, "host stream item frame");
  }

  async encodeError(error) {
    return callJSON(this.requireOpen(), "encodeError", { error: hostBindingErrorDTO(error) }, "host stream error frame");
  }

  async encodeTerminal(summary) {
    return callJSON(
      this.requireOpen(),
      "encodeTerminal",
      { summary: hostStreamTerminalSummary(summary) },
      "host stream terminal frame",
    );
  }

  async foldOutputHash(state, seq, value) {
    const hashState = state instanceof HostStreamHashState ? state : new HostStreamHashState(state);
    hostNonNegativeInteger(seq, "seq");
    validateHostHashFold(hashState, seq);
    return HostStreamHashState.fromJSON(
      await callRaw(this.requireOpen(), "foldOutputHash", {
        state: hashState.toJSON(),
        seq,
        value,
      }),
    );
  }

  openLifecycle(binding, provider = null) {
    if (!binding || typeof binding !== "object") {
      throw invalidHostBinding("host stream binding is required");
    }
    const resolved = provider ?? this.lifecycleProvider;
    if (!resolved) {
      throw invalidHostBinding("host stream lifecycle provider is required");
    }
    return new HostStreamLifecycleController(binding, resolved);
  }

  async checkReadiness(binding, provider = null) {
    return this.openLifecycle(binding, provider).checkReadiness();
  }

  async cleanup(binding, provider = null) {
    return this.openLifecycle(binding, provider).cleanup();
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("host binding client is closed");
    }
    return this.transport;
  }
}

export class HostStreamLifecycleController {
  constructor(binding, provider) {
    this.binding = binding;
    this.provider = provider;
    this.state = "declared";
    this.readiness = null;
    this.cleanupResult = null;
  }

  async checkReadiness() {
    if (["cleaning", "cleaned", "closed"].includes(this.state)) {
      throw invalidHostBinding("host stream lifecycle is not readable");
    }
    this.state = "checking";
    if (!this.provider || typeof this.provider.checkReadiness !== "function") {
      this.state = "failed";
      throw invalidHostBinding("readiness provider is required");
    }
    const readiness = objectValue(await this.provider.checkReadiness(this.binding), "readiness");
    requiredHostBindingString(readiness.state, "state");
    if (readiness.checked !== undefined && typeof readiness.checked !== "boolean") {
      this.state = "failed";
      throw invalidHostBinding("readiness.checked must be boolean");
    }
    this.readiness = readiness;
    this.state = readiness.state === "ready" ? "ready" : "not_ready";
    return readiness;
  }

  async cleanup() {
    if (this.state === "cleaned") {
      return this.cleanupResult;
    }
    if (this.state === "closed") {
      throw invalidHostBinding("host stream lifecycle is closed");
    }
    if (!this.provider || typeof this.provider.cleanup !== "function") {
      this.state = "failed";
      throw invalidHostBinding("cleanup provider is required");
    }
    this.state = "cleaning";
    const cleanup = objectValue(await this.provider.cleanup(this.binding), "cleanup");
    requiredHostBindingString(cleanup.mode, "mode");
    this.cleanupResult = cleanup;
    this.state = "cleaned";
    return cleanup;
  }

  close() {
    if (this.state === "closed") {
      return;
    }
    if (this.state !== "cleaned") {
      throw invalidHostBinding("host stream lifecycle must be cleaned before close");
    }
    this.state = "closed";
  }
}

export class InvocationDraft {
  constructor(fields) {
    this.callerURA = requiredBuilderString(fields.callerURA, "caller_ura");
    this.calleeURA = requiredBuilderString(fields.calleeURA, "callee_ura");
    this.descriptorRef = requiredBuilderString(fields.descriptorRef, "descriptor_ref");
    this.subjectURA = requiredBuilderString(fields.subjectURA, "subject_ura");
    this.nonceBase64 = requiredBuilderString(fields.nonceBase64, "nonce_base64");
    this.causalContext = objectValue(fields.causalContext, "causal_context");
    this.contentType = requiredBuilderString(fields.contentType, "content_type");
    this.args = fields.args;
    this.argumentsBase64 = fields.argumentsBase64 || "";
    this.metadata = objectValue(fields.metadata ?? {}, "metadata");
    this.callerSignature = fields.callerSignature ?? null;
    this.hasArgs = Boolean(fields.hasArgs);
    validateAuthorityMetadata(this.metadata);
    validateInvocationPayloadChoice(this);
    validateBase64(this.nonceBase64, "nonce_base64", 16);
    if (this.argumentsBase64) {
      validateBase64(this.argumentsBase64, "arguments_base64");
    }
  }

  static fromJSON(raw) {
    const decoded = parseJSON(raw, "invocation");
    rejectUnknownFields(decoded, [
      "caller_ura",
      "callee_ura",
      "descriptor_ref",
      "subject_ura",
      "nonce_base64",
      "causal_context",
      "args",
      "arguments_base64",
      "content_type",
      "metadata",
      "caller_signature",
    ]);
    return new InvocationDraft({
      callerURA: decoded.caller_ura,
      calleeURA: decoded.callee_ura,
      descriptorRef: decoded.descriptor_ref,
      subjectURA: decoded.subject_ura,
      nonceBase64: decoded.nonce_base64,
      causalContext: decoded.causal_context,
      contentType: decoded.content_type,
      args: decoded.args,
      argumentsBase64: decoded.arguments_base64,
      metadata: decoded.metadata ?? {},
      callerSignature: decoded.caller_signature ?? null,
      hasArgs: Object.hasOwn(decoded, "args"),
    });
  }

  toJSON() {
    const value = {
      caller_ura: this.callerURA,
      callee_ura: this.calleeURA,
      descriptor_ref: this.descriptorRef,
      subject_ura: this.subjectURA,
      nonce_base64: this.nonceBase64,
      causal_context: this.causalContext,
      content_type: this.contentType,
      metadata: this.metadata,
    };
    if (this.hasArgs) {
      value.args = this.args;
    } else {
      value.arguments_base64 = this.argumentsBase64;
    }
    if (this.callerSignature) {
      value.caller_signature = this.callerSignature;
    }
    return value;
  }

  toJSONString() {
    return JSON.stringify(this.toJSON());
  }
}

export class InvocationBuilder {
  constructor() {
    this.fields = { metadata: {} };
    this.hasArgs = false;
    this.hasArguments = false;
    this.consumed = false;
  }

  withCallerURA(value) {
    this.fields.callerURA = value;
    return this;
  }

  withCalleeURA(value) {
    this.fields.calleeURA = value;
    return this;
  }

  withDescriptorRef(value) {
    this.fields.descriptorRef = value;
    return this;
  }

  withSubjectURA(value) {
    this.fields.subjectURA = value;
    return this;
  }

  withNonceBase64(value) {
    this.fields.nonceBase64 = value;
    return this;
  }

  withCausalContext(value) {
    this.fields.causalContext = objectValue(value, "causal_context");
    return this;
  }

  withJSONArgs(value) {
    this.fields.args = value;
    this.hasArgs = true;
    return this;
  }

  withArgumentsBase64(value) {
    this.fields.argumentsBase64 = value;
    this.hasArguments = true;
    return this;
  }

  withContentType(value) {
    this.fields.contentType = value;
    return this;
  }

  withMetadata(value) {
    this.fields.metadata = objectValue(value, "metadata");
    return this;
  }

  withAuthorityMetadata(value) {
    const authority = value instanceof AuthorityMetadata ? value : new AuthorityMetadata(value);
    this.fields.metadata = authority.mergeInto(this.fields.metadata ?? {});
    return this;
  }

  inspect() {
    this.requireMutable();
    return new InvocationDraft({
      ...this.fields,
      hasArgs: this.hasArgs,
    });
  }

  build() {
    const draft = this.inspect();
    this.consumed = true;
    return draft;
  }

  requireMutable() {
    if (this.consumed) {
      throw new SDKError({
        code: ErrorCode.INVALID_HANDLE,
        stage: "build",
        retry: RetryHint.NEVER,
        message: "invocation builder handle is consumed",
      });
    }
    if (this.hasArgs === this.hasArguments) {
      throw invalidInvocation("exactly one of args or arguments_base64 is required");
    }
  }
}

export class RuntimeClient {
  constructor(transport) {
    if (!transport || typeof transport.invoke !== "function") {
      throw invalidSDK("runtime transport is required");
    }
    this.transport = transport;
    this.closed = false;
  }

  newInvocation() {
    this.requireOpen();
    return new InvocationBuilder();
  }

  async invoke(draft) {
    const raw = await this.requireOpen().invoke(Buffer.from(assertDraft(draft).toJSONString()));
    return parseJSON(raw, "invocation result");
  }

  async prepare(draft, options = {}) {
    const transport = this.requireOpen();
    if (typeof transport.prepare !== "function") {
      throw invalidSDK("runtime prepare transport function is required");
    }
    const raw = await transport.prepare(
      Buffer.from(assertDraft(draft).toJSONString()),
      Buffer.from(JSON.stringify(options ?? {})),
    );
    return PreparedInvocation.fromJSON(raw).bindRuntime(this);
  }

  async submitSigned(signed) {
    const transport = this.requireOpen();
    if (typeof transport.submitSigned !== "function") {
      throw invalidSDK("runtime submit-signed transport function is required");
    }
    if (!(signed instanceof SignedInvocation)) {
      throw invalidRuntime("signed invocation is required");
    }
    const raw = await transport.submitSigned(Buffer.from(JSON.stringify(signed.toJSON())));
    return InvocationHandle.fromJSON(raw).bindRuntime(this);
  }

  async awaitResult(handle) {
    const transport = this.requireOpen();
    if (typeof transport.awaitHandle !== "function") {
      throw invalidSDK("runtime await-handle transport function is required");
    }
    return parseJSON(await transport.awaitHandle(assertHandle(handle).handleId), "invocation result");
  }

  async cancel(handle, reason = "") {
    const transport = this.requireOpen();
    if (typeof transport.cancelHandle !== "function") {
      throw invalidSDK("runtime cancel-handle transport function is required");
    }
    if (typeof reason !== "string") {
      throw invalidRuntime("cancel reason must be a string");
    }
    return InvocationCancel.fromJSON(await transport.cancelHandle(assertHandle(handle).handleId, reason));
  }

  async events(handle) {
    const transport = this.requireOpen();
    if (typeof transport.handleEvents !== "function") {
      throw invalidSDK("runtime handle-events transport function is required");
    }
    return InvocationHandle.fromJSON(await transport.handleEvents(assertHandle(handle).handleId))
      .bindRuntime(this);
  }

  async closeHandle(handle) {
    const transport = this.requireOpen();
    if (typeof transport.freeHandle !== "function") {
      throw invalidSDK("runtime free-handle transport function is required");
    }
    await transport.freeHandle(assertHandle(handle).handleId);
  }

  async invokeStream(draft) {
    const transport = this.requireOpen();
    if (typeof transport.openStream !== "function") {
      throw invalidSDK("runtime open-stream transport function is required");
    }
    const result = await transport.openStream(Buffer.from(assertDraft(draft).toJSONString()));
    return new StreamHandle(result.transport, parseJSON(result.open, "stream open"));
  }

  async openBidi(draft, streams = []) {
    const transport = this.requireOpen();
    if (typeof transport.openBidi !== "function") {
      throw invalidSDK("runtime open-bidi transport function is required");
    }
    const result = await transport.openBidi(
      Buffer.from(assertDraft(draft).toJSONString()),
      Buffer.from(JSON.stringify(streams)),
    );
    return new BidiSession(result.transport, parseJSON(result.open, "bidi open"));
  }

  async close() {
    if (this.closed) {
      return;
    }
    const transport = this.transport;
    this.closed = true;
    this.transport = null;
    if (transport && typeof transport.close === "function") {
      await transport.close();
    }
  }

  requireOpen() {
    if (this.closed || !this.transport) {
      throw invalidSDK("runtime client is closed");
    }
    return this.transport;
  }
}

export class SignerPolicy {
  constructor(fields = {}) {
    const value = objectValue(fields, "signer policy");
    rejectRuntimeFields(value, ["mode", "signer_id", "policy_ref", "expires_at_unix_ms"]);
    this.mode = optionalRuntimeString(value.mode, "mode") ?? "";
    this.signerId = optionalRuntimeString(value.signer_id, "signer_id") ?? "";
    this.policyRef = optionalRuntimeString(value.policy_ref, "policy_ref") ?? "";
    this.expiresAtUnixMS = optionalRuntimeNonNegativeInteger(
      value.expires_at_unix_ms,
      "expires_at_unix_ms",
    ) ?? 0;
    Object.freeze(this);
  }

  toJSON() {
    return {
      mode: this.mode,
      signer_id: this.signerId,
      policy_ref: this.policyRef,
      expires_at_unix_ms: this.expiresAtUnixMS,
    };
  }
}

export class SigningMaterial {
  constructor(fields) {
    const value = objectValue(fields, "signing material");
    rejectRuntimeFields(value, [
      "algorithm",
      "canonical_bytes_base64",
      "args_digest_hex",
      "descriptor_ref",
      "nonce_base64",
      "signed_fields",
      "expires_at_unix_ms",
      "signer_policy",
    ]);
    this.algorithm = optionalRuntimeString(value.algorithm, "algorithm") ?? "";
    this.canonicalBytesBase64 = requiredRuntimeString(
      value.canonical_bytes_base64,
      "canonical_bytes_base64",
    );
    validateRuntimeBase64(this.canonicalBytesBase64, "canonical_bytes_base64");
    this.argsDigestHex = requiredRuntimeString(value.args_digest_hex, "args_digest_hex");
    validateRuntimeHex(this.argsDigestHex, "args_digest_hex", 64);
    this.descriptorRef = requiredRuntimeString(value.descriptor_ref, "descriptor_ref");
    this.nonceBase64 = optionalRuntimeString(value.nonce_base64, "nonce_base64") ?? "";
    if (this.nonceBase64) {
      validateRuntimeBase64(this.nonceBase64, "nonce_base64");
    }
    const signedFields = value.signed_fields ?? [];
    if (!Array.isArray(signedFields) || signedFields.some((field) => typeof field !== "string")) {
      throw invalidRuntime("signed_fields must be an array of strings");
    }
    this.signedFields = Object.freeze([...signedFields]);
    this.expiresAtUnixMS = positiveRuntimeInteger(value.expires_at_unix_ms, "expires_at_unix_ms");
    this.signerPolicy = value.signer_policy === undefined || value.signer_policy === null
      ? null
      : new SignerPolicy(value.signer_policy);
    Object.freeze(this);
  }

  toJSON() {
    const value = {
      algorithm: this.algorithm,
      canonical_bytes_base64: this.canonicalBytesBase64,
      args_digest_hex: this.argsDigestHex,
      descriptor_ref: this.descriptorRef,
      nonce_base64: this.nonceBase64,
      signed_fields: [...this.signedFields],
      expires_at_unix_ms: this.expiresAtUnixMS,
    };
    if (this.signerPolicy) {
      value.signer_policy = this.signerPolicy.toJSON();
    }
    return value;
  }
}

export class InvocationSignature {
  constructor(fields) {
    const value = objectValue(fields, "invocation signature");
    rejectRuntimeFields(value, [
      "algorithm",
      "signature_base64",
      "key_id_hint",
      "signer_public_key_base64",
    ]);
    this.algorithm = requiredRuntimeString(value.algorithm, "signature.algorithm");
    this.signatureBase64 = requiredRuntimeString(value.signature_base64, "signature.signature_base64");
    validateRuntimeBase64(this.signatureBase64, "signature.signature_base64");
    this.keyIdHint = optionalRuntimeString(value.key_id_hint, "key_id_hint") ?? "";
    this.signerPublicKeyBase64 =
      optionalRuntimeString(value.signer_public_key_base64, "signer_public_key_base64") ?? "";
    if (this.signerPublicKeyBase64) {
      validateRuntimeBase64(this.signerPublicKeyBase64, "signer_public_key_base64");
    }
    Object.freeze(this);
  }

  toJSON() {
    return {
      algorithm: this.algorithm,
      signature_base64: this.signatureBase64,
      key_id_hint: this.keyIdHint,
      signer_public_key_base64: this.signerPublicKeyBase64,
    };
  }
}

export class PreparedInvocation {
  constructor(fields) {
    const value = objectValue(fields, "prepared invocation");
    rejectRuntimeFields(value, [
      "prepared_id",
      "request_id",
      "tuple",
      "signing_material",
      "descriptor_ref",
      "descriptor_hash_hex",
      "schema_hash_hex",
      "canonical_hash_hex",
      "expires_at_unix_ms",
      "submit_ready",
    ]);
    if (
      value.submit_ready !== undefined &&
      value.submit_ready !== null &&
      value.submit_ready !== false
    ) {
      throw invalidRuntime("PreparedInvocation must not be submit-ready");
    }
    this.tuple = InvocationDraft.fromJSON(JSON.stringify(objectValue(value.tuple, "tuple")));
    this.signingMaterial = new SigningMaterial(objectValue(value.signing_material, "signing_material"));
    if (this.signingMaterial.descriptorRef !== this.tuple.descriptorRef) {
      throw invalidRuntime("signing_material.descriptor_ref must match tuple descriptor_ref");
    }
    this.preparedId = optionalRuntimeString(value.prepared_id, "prepared_id") ?? "";
    this.requestId = optionalRuntimeString(value.request_id, "request_id") ?? "";
    if (!this.preparedId && !this.requestId) {
      throw invalidRuntime("prepared_id or request_id is required");
    }
    this.descriptorRef =
      optionalRuntimeString(value.descriptor_ref, "descriptor_ref") ?? this.signingMaterial.descriptorRef;
    this.descriptorHashHex = optionalRuntimeString(value.descriptor_hash_hex, "descriptor_hash_hex") ?? "";
    this.schemaHashHex = optionalRuntimeString(value.schema_hash_hex, "schema_hash_hex") ?? "";
    this.canonicalHashHex = optionalRuntimeString(value.canonical_hash_hex, "canonical_hash_hex") ?? "";
    validatePreparedHash(this.signingMaterial.canonicalBytesBase64, this.canonicalHashHex);
    this.expiresAtUnixMS =
      optionalRuntimeNonNegativeInteger(value.expires_at_unix_ms, "expires_at_unix_ms") ??
      this.signingMaterial.expiresAtUnixMS;
    this.runtime = null;
  }

  static fromJSON(raw) {
    return new PreparedInvocation(parseJSON(raw, "prepared invocation"));
  }

  bindRuntime(runtime) {
    this.runtime = runtime;
    return this;
  }

  submitReady() {
    return false;
  }

  signWithCallerSignature(signature) {
    const normalized = signature instanceof InvocationSignature
      ? signature
      : new InvocationSignature(signature);
    let signerId = normalized.keyIdHint || normalized.signerPublicKeyBase64;
    if (this.signingMaterial.signerPolicy && this.signingMaterial.signerPolicy.signerId) {
      signerId = this.signingMaterial.signerPolicy.signerId;
    }
    if (!signerId) {
      throw invalidRuntime("signer id is required");
    }
    return new SignedInvocation({
      prepared: this,
      signature: normalized,
      signer_id: signerId,
      policy: this.signingMaterial.signerPolicy,
    }).bindRuntime(this.runtime);
  }

  toJSON() {
    const value = {
      prepared_id: this.preparedId,
      request_id: this.requestId,
      tuple: this.tuple.toJSON(),
      signing_material: this.signingMaterial.toJSON(),
      descriptor_ref: this.descriptorRef,
      descriptor_hash_hex: this.descriptorHashHex,
      schema_hash_hex: this.schemaHashHex,
      canonical_hash_hex: this.canonicalHashHex,
      expires_at_unix_ms: this.expiresAtUnixMS,
      submit_ready: false,
    };
    return value;
  }
}

export class SignedInvocation {
  constructor(fields) {
    const value = objectValue(fields, "signed invocation");
    rejectRuntimeFields(value, ["prepared", "signature", "signer_id", "policy"]);
    this.prepared = value.prepared instanceof PreparedInvocation
      ? value.prepared
      : new PreparedInvocation(objectValue(value.prepared, "prepared"));
    this.signature = value.signature instanceof InvocationSignature
      ? value.signature
      : new InvocationSignature(objectValue(value.signature, "signature"));
    this.signerId = requiredRuntimeString(value.signer_id, "signer_id");
    this.policy = value.policy === undefined || value.policy === null
      ? null
      : (value.policy instanceof SignerPolicy ? value.policy : new SignerPolicy(value.policy));
    this.runtime = null;
    if (!this.submitReady()) {
      throw invalidRuntime("signed invocation is not submit-ready");
    }
  }

  bindRuntime(runtime) {
    this.runtime = runtime;
    this.prepared.bindRuntime(runtime);
    return this;
  }

  submitReady() {
    return Boolean(
      this.signerId &&
      this.signature.algorithm &&
      this.signature.signatureBase64 &&
      this.prepared.descriptorRef &&
      this.prepared.signingMaterial.canonicalBytesBase64,
    );
  }

  async submit() {
    return requireBoundRuntime(this.runtime).submitSigned(this);
  }

  toJSON() {
    const value = {
      signer_id: this.signerId,
      prepared: {
        prepared_id: this.prepared.preparedId,
        request_id: this.prepared.requestId,
        descriptor_ref: this.prepared.descriptorRef,
        canonical_hash_hex: this.prepared.canonicalHashHex,
        expires_at_unix_ms: this.prepared.expiresAtUnixMS,
        canonical_bytes_base64: this.prepared.signingMaterial.canonicalBytesBase64,
        tuple: this.prepared.tuple.toJSON(),
      },
      signature: this.signature.toJSON(),
    };
    if (this.policy) {
      value.policy = this.policy.toJSON();
    }
    return value;
  }
}

export class InvocationHandleEvent {
  constructor(fields) {
    const value = objectValue(fields, "invocation handle event");
    rejectRuntimeFields(value, ["sequence", "kind", "state", "terminal", "reason", "result"]);
    this.sequence = positiveRuntimeInteger(value.sequence, "sequence");
    this.kind = requiredRuntimeString(value.kind, "kind");
    this.state = requiredRuntimeString(value.state, "state");
    this.terminal = runtimeBoolean(value.terminal, "terminal");
    this.reason = optionalRuntimeString(value.reason, "reason");
    this.result = optionalRuntimeObject(value.result, "result");
  }

  toJSON() {
    const value = {
      sequence: this.sequence,
      kind: this.kind,
      state: this.state,
      terminal: this.terminal,
    };
    if (this.reason !== null) {
      value.reason = this.reason;
    }
    if (this.result !== null) {
      value.result = this.result;
    }
    return value;
  }
}

export class InvocationHandle {
  constructor(fields) {
    const value = objectValue(fields, "invocation handle");
    rejectRuntimeFields(value, ["handle_id", "state", "terminal", "events", "result"]);
    this.handleId = positiveRuntimeInteger(value.handle_id, "handle_id");
    this.state = requiredRuntimeString(value.state, "state");
    this.terminal = runtimeBoolean(value.terminal, "terminal");
    const events = value.events ?? [];
    if (!Array.isArray(events)) {
      throw invalidRuntime("events must be an array");
    }
    this.events = events.map((event) => new InvocationHandleEvent(event));
    this.result = optionalRuntimeObject(value.result, "result");
    this.runtime = null;
    validateInvocationHandleMonotonicity(this);
  }

  static fromJSON(raw) {
    return new InvocationHandle(parseJSON(raw, "invocation handle"));
  }

  bindRuntime(runtime) {
    this.runtime = runtime;
    return this;
  }

  async awaitResult() {
    return requireBoundRuntime(this.runtime).awaitResult(this);
  }

  async cancel(reason = "") {
    return requireBoundRuntime(this.runtime).cancel(this, reason);
  }

  async refreshEvents() {
    return requireBoundRuntime(this.runtime).events(this);
  }

  async close() {
    return requireBoundRuntime(this.runtime).closeHandle(this);
  }

  toJSON() {
    return {
      handle_id: this.handleId,
      state: this.state,
      terminal: this.terminal,
      events: this.events.map((event) => event.toJSON()),
      result: this.result,
    };
  }
}

export class InvocationCancel {
  constructor(fields) {
    const value = objectValue(fields, "invocation cancel");
    rejectRuntimeFields(value, ["handle_id", "cancelled", "state", "terminal"]);
    this.handleId = positiveRuntimeInteger(value.handle_id, "handle_id");
    this.cancelled = runtimeBoolean(value.cancelled, "cancelled");
    this.state = requiredRuntimeString(value.state, "state");
    this.terminal = runtimeBoolean(value.terminal, "terminal");
  }

  static fromJSON(raw) {
    return new InvocationCancel(parseJSON(raw, "invocation cancel"));
  }

  toJSON() {
    return {
      handle_id: this.handleId,
      cancelled: this.cancelled,
      state: this.state,
      terminal: this.terminal,
    };
  }
}

export class StreamHandle {
  constructor(transport, open) {
    if (!transport || typeof transport.receive !== "function") {
      throw invalidSDK("stream transport is required");
    }
    this.transport = transport;
    this.open = objectValue(open, "stream open");
    this.maxBufferedEvents = boundedRuntimeLimit(
      this.open.max_buffered_events,
      "max_buffered_events",
      MAX_STREAM_BUFFERED_EVENTS,
    );
    this.retainedEvents = [];
    this.overflow = null;
    this.closed = false;
    this.terminal = false;
  }

  async receive(options = {}) {
    if (this.closed) {
      throw invalidSDK("stream handle is closed");
    }
    if (this.terminal) {
      throw invalidSDK("stream handle is terminal");
    }
    const event = parseJSON(
      await withAbortSignal(this.transport.receive(), options.signal, () =>
        this.cancel(options.cancelReason ?? abortReason(options.signal)),
      ),
      "stream event",
    );
    if (isTerminalFrame(event)) {
      this.terminal = true;
    }
    return this.recordEvent(event);
  }

  async *events(options = {}) {
    try {
      while (!this.closed && !this.terminal) {
        const event = await this.receive(options);
        yield event;
      }
    } finally {
      if (options.closeOnReturn !== false) {
        await this.close();
      }
    }
  }

  [Symbol.asyncIterator]() {
    return this.events();
  }

  async cancel(reason = "") {
    if (!this.closed && typeof this.transport.cancel === "function") {
      await this.transport.cancel(reason);
    }
    this.terminal = true;
    this.closed = true;
  }

  async close() {
    if (!this.closed && typeof this.transport.close === "function") {
      await this.transport.close();
    }
    this.terminal = true;
    this.closed = true;
  }

  terminalEvent() {
    if (this.overflow) {
      return this.overflow;
    }
    return this.retainedEvents.findLast((event) => event.terminal === true) ?? null;
  }

  recordEvent(event) {
    if (this.retainedEvents.length >= this.maxBufferedEvents) {
      this.overflow = streamBackpressureTerminal(this.maxBufferedEvents, this.retainedEvents.length);
      this.terminal = true;
      return this.overflow;
    }
    this.retainedEvents.push(event);
    return event;
  }
}

export class BidiSession {
  constructor(transport, open) {
    if (!transport || typeof transport.send !== "function" || typeof transport.receive !== "function") {
      throw invalidSDK("bidi transport is required");
    }
    this.transport = transport;
    this.open = objectValue(open, "bidi open");
    this.maxBufferedFrames = boundedRuntimeLimit(
      this.open.max_buffered_frames,
      "max_buffered_frames",
      MAX_BIDI_BUFFERED_FRAMES,
    );
    this.sentFrames = [];
    this.receivedFrames = [];
    this.overflow = null;
    this.closed = false;
    this.terminal = false;
  }

  async send(frame, options = {}) {
    if (this.closed) {
      throw invalidSDK("bidi session is closed");
    }
    if (this.terminal) {
      throw invalidSDK("bidi session is terminal");
    }
    const normalizedFrame = objectValue(frame, "bidi send frame");
    this.ensureSentFrameCapacity();
    await withAbortSignal(
      this.transport.send(Buffer.from(JSON.stringify(normalizedFrame))),
      options.signal,
      () => this.cancel(options.cancelReason ?? abortReason(options.signal)),
    );
    this.sentFrames.push(normalizedFrame);
  }

  async receive(options = {}) {
    if (this.closed) {
      throw invalidSDK("bidi session is closed");
    }
    if (this.terminal) {
      throw invalidSDK("bidi session is terminal");
    }
    const frame = parseJSON(
      await withAbortSignal(this.transport.receive(), options.signal, () =>
        this.cancel(options.cancelReason ?? abortReason(options.signal)),
      ),
      "bidi frame",
    );
    if (isTerminalFrame(frame)) {
      this.terminal = true;
    }
    return this.recordReceivedFrame(frame);
  }

  async *frames(options = {}) {
    try {
      while (!this.closed && !this.terminal) {
        const frame = await this.receive(options);
        yield frame;
      }
    } finally {
      if (options.closeOnReturn !== false) {
        await this.close();
      }
    }
  }

  [Symbol.asyncIterator]() {
    return this.frames();
  }

  async closeSend() {
    if (typeof this.transport.closeSend === "function") {
      await this.transport.closeSend();
    }
  }

  async cancel(reason = "") {
    if (!this.closed && typeof this.transport.cancel === "function") {
      await this.transport.cancel(reason);
    }
    this.terminal = true;
    this.closed = true;
  }

  async close() {
    if (!this.closed && typeof this.transport.close === "function") {
      await this.transport.close();
    }
    this.terminal = true;
    this.closed = true;
  }

  terminalFrame() {
    if (this.overflow) {
      return this.overflow;
    }
    return this.receivedFrames.findLast((frame) => frame.terminal === true) ?? null;
  }

  ensureSentFrameCapacity() {
    if (this.sentFrames.length >= this.maxBufferedFrames) {
      this.overflow = bidiBackpressureTerminal("send", this.maxBufferedFrames, this.sentFrames.length);
      this.terminal = true;
      throw backpressureSDK("bidi send buffer limit exceeded", {
        direction: "send",
        max_buffered_frames: this.maxBufferedFrames,
        buffered_frames: this.sentFrames.length,
      });
    }
  }

  recordReceivedFrame(frame) {
    if (this.receivedFrames.length >= this.maxBufferedFrames) {
      this.overflow = bidiBackpressureTerminal("receive", this.maxBufferedFrames, this.receivedFrames.length);
      this.terminal = true;
      return this.overflow;
    }
    this.receivedFrames.push(frame);
    return frame;
  }
}

function callJSON(transport, method, payload, label) {
  return Promise.resolve(callRaw(transport, method, payload)).then((raw) =>
    parseJSON(raw, label),
  );
}

function callRaw(transport, method, payload) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  return Promise.resolve(transport[method](Buffer.from(JSON.stringify(payload))));
}

function callReceiptJSON(transport, method, receiptJSON) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  return Promise.resolve(transport[method](jsonPayloadBytes(receiptJSON, "receipt JSON")));
}

function identityRequest(value, allowed) {
  return requestObject(value, allowed, "identity request");
}

function directoryRequest(value, allowed) {
  return requestObject(value, allowed, "directory request");
}

function requestObject(value, allowed, label) {
  const payload = objectValue(value, label);
  rejectUnknownFields(payload, allowed);
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidSDK(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function directoryBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "limit",
    "cursor",
    "metadata",
  ];
}

function receiptCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "timeout_ms",
    "metadata",
  ];
}

function eventsCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
  ];
}

function surfaceCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
  ];
}

function missionCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
  ];
}

function missionRunRequest(value) {
  const payload = missionRequest(
    value,
    [...missionCarrierBaseFields(), "source", "label"],
    "mission run request",
  );
  validateMissionCarrierBase(payload);
  requiredMissionString(payload.source, "source");
  if (payload.label !== undefined) {
    requiredMissionString(payload.label, "label");
  }
  return payload;
}

function missionRunFileRequest(value) {
  const payload = missionRequest(
    value,
    [...missionCarrierBaseFields(), "path", "label"],
    "mission run file request",
  );
  validateMissionCarrierBase(payload);
  const path = requiredMissionString(payload.path, "path");
  if (!path.startsWith("/")) {
    throw invalidMission("absolute mission file path is required");
  }
  if (payload.label !== undefined) {
    requiredMissionString(payload.label, "label");
  }
  return payload;
}

function missionTrackRequest(value) {
  const payload = missionRequest(
    value,
    [...missionCarrierBaseFields(), "mission_id"],
    "mission track request",
  );
  validateMissionCarrierBase(payload);
  requiredMissionID(payload.mission_id, "mission_id");
  return payload;
}

function missionCancelRequest(value) {
  const payload = missionRequest(
    value,
    [...missionCarrierBaseFields(), "mission_id"],
    "mission cancel request",
  );
  validateMissionCarrierBase(payload);
  requiredMissionID(payload.mission_id, "mission_id");
  return payload;
}

function missionEventsRequest(value) {
  const payload = missionRequest(
    value,
    [...missionCarrierBaseFields(), "mission_id", "cursor_sequence", "limit"],
    "mission events request",
  );
  validateMissionCarrierBase(payload);
  requiredMissionID(payload.mission_id, "mission_id");
  if (payload.cursor_sequence !== undefined) {
    missionNonNegativeInteger(payload.cursor_sequence, "cursor_sequence");
  }
  if (payload.limit !== undefined) {
    missionPositiveBoundedInteger(payload.limit, "limit", 1000);
  }
  return payload;
}

function missionRequest(value, allowed, label) {
  const payload = objectValue(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(payload)) {
    if (!allowedSet.has(key)) {
      throw invalidMission(`${key} is not a mission field`);
    }
  }
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidMission(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function validateMissionCarrierBase(payload) {
  requiredMissionURA(payload.caller_ura, "caller_ura");
  requiredMissionURA(payload.callee_ura, "callee_ura");
  requiredMissionURA(payload.subject_ura, "subject_ura");
  requiredMissionString(payload.descriptor_version, "descriptor_version");
  requiredMissionString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function validateMissionStatus(status) {
  if (status.childReceipts.length > 0 && !status.parentReceiptURA) {
    throw invalidMission("child receipts require a parent receipt anchor");
  }
  for (const receipt of status.childReceipts) {
    requiredMissionString(receipt.receipt_ura, "receipt_ura");
    requiredMissionString(receipt.receipt_hash, "receipt_hash");
  }
  for (const child of status.childInvocations) {
    if (child.receipt !== undefined && child.receipt !== null) {
      objectValue(child.receipt, "child receipt");
    }
  }
}

function validateMissionEventPage(page) {
  if (page.nextCursorSequence !== null && page.nextCursorSequence < page.cursorSequence) {
    throw invalidMission("next_cursor_sequence must not go backwards");
  }
  let previous = null;
  for (const event of page.events) {
    if (event.missionID !== page.missionID) {
      throw invalidMission("mission event page mission_id mismatch");
    }
    if (previous !== null && event.sequence <= previous) {
      throw invalidMission("mission events must be strictly ordered by sequence");
    }
    previous = event.sequence;
  }
}

function callMissionProjection(transport, method, value) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  if (value instanceof MissionStatus || value instanceof MissionEventPage) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value.toJSON()))));
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidMission("mission projection JSON is required");
    }
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value))));
  }
  throw invalidMission("mission projection must be bytes, string, or object");
}

function adminCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
  ];
}

function adminGatewayStatusRequest(value) {
  const payload = adminRequest(value, ["require_public_listener", "metadata"], "gateway status request");
  if (payload.require_public_listener !== undefined) {
    adminBoolean(payload.require_public_listener, "require_public_listener");
  }
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function adminBaseRequest(value, label) {
  const payload = adminRequest(value, adminCarrierBaseFields(), label);
  validateAdminCarrierBase(payload);
  return payload;
}

function adminAgentStartRequest(value) {
  const payload = adminRequest(
    value,
    [
      ...adminCarrierBaseFields(),
      "name",
      "agent_type",
      "entry",
      "model",
      "label",
      "command",
      "command_args",
      "root_path",
      "model_present",
      "materialize_directory",
      "update_existing_spec",
      "project_workspace",
    ],
    "admin agent start request",
  );
  validateAdminCarrierBase(payload);
  validateAdminAgentName(payload.name, "name");
  if (payload.agent_type === undefined && payload.entry === undefined) {
    throw invalidAdminGateway("agent_type or entry is required");
  }
  for (const field of ["agent_type", "model", "label", "command", "root_path"]) {
    if (payload[field] !== undefined) {
      requiredAdminString(payload[field], field);
    }
  }
  if (payload.entry !== undefined) {
    objectValue(payload.entry, "entry");
  }
  if (payload.command_args !== undefined) {
    adminStringArray(payload.command_args, "command_args");
  }
  for (const field of ["model_present", "materialize_directory", "update_existing_spec", "project_workspace"]) {
    if (payload[field] !== undefined) {
      adminBoolean(payload[field], field);
    }
  }
  return payload;
}

function adminAgentStopRequest(value) {
  const payload = adminRequest(value, [...adminCarrierBaseFields(), "name", "agent_ura"], "admin agent stop request");
  validateAdminCarrierBase(payload);
  if (!payload.name && !payload.agent_ura) {
    throw invalidAdminGateway("name or agent_ura is required");
  }
  if (payload.name !== undefined) {
    validateAdminAgentName(payload.name, "name");
  }
  if (payload.agent_ura !== undefined) {
    requiredAdminURA(payload.agent_ura, "agent_ura");
  }
  return payload;
}

function adminAgentRefreshRequest(value) {
  const payload = adminRequest(value, [...adminCarrierBaseFields(), "name"], "admin agent refresh request");
  validateAdminCarrierBase(payload);
  if (payload.name !== undefined) {
    validateAdminAgentName(payload.name, "name");
  }
  return payload;
}

function adminSessionListRequest(value) {
  const payload = adminRequest(value, [...adminCarrierBaseFields(), "include_terminated"], "admin session list request");
  validateAdminCarrierBase(payload);
  if (payload.include_terminated !== undefined) {
    adminBoolean(payload.include_terminated, "include_terminated");
  }
  return payload;
}

function adminPairingPreflightRequest(value) {
  const payload = adminRequest(
    value,
    [...adminCarrierBaseFields(), "hub_ura", "device_ura", "requested_scopes"],
    "admin pairing preflight request",
  );
  validateAdminCarrierBase(payload);
  requiredAdminURA(payload.hub_ura, "hub_ura");
  requiredAdminURA(payload.device_ura, "device_ura");
  if (payload.requested_scopes !== undefined) {
    adminStringArray(payload.requested_scopes, "requested_scopes");
  }
  return payload;
}

function adminCreatePairingRequest(value) {
  const payload = adminRequest(
    value,
    [...adminCarrierBaseFields(), "hub_ura", "device_ura", "expires_unix_ms", "scopes"],
    "admin pairing create request",
  );
  validateAdminCarrierBase(payload);
  requiredAdminURA(payload.hub_ura, "hub_ura");
  requiredAdminURA(payload.device_ura, "device_ura");
  adminPositiveInteger(payload.expires_unix_ms, "expires_unix_ms");
  if (payload.scopes !== undefined) {
    adminStringArray(payload.scopes, "scopes");
  }
  return payload;
}

function adminValidatePairingRequest(value) {
  const payload = adminRequest(
    value,
    [...adminCarrierBaseFields(), "token", "device_ura"],
    "admin pairing validate request",
  );
  validateAdminCarrierBase(payload);
  requiredAdminString(payload.token, "token");
  requiredAdminURA(payload.device_ura, "device_ura");
  return payload;
}

function adminCreateDeviceSessionRequest(value) {
  const payload = adminRequest(
    value,
    [...adminCarrierBaseFields(), "device_ura", "hub_ura", "session_kind", "expires_unix_ms"],
    "admin device session create request",
  );
  validateAdminCarrierBase(payload);
  requiredAdminURA(payload.device_ura, "device_ura");
  requiredAdminURA(payload.hub_ura, "hub_ura");
  requiredAdminString(payload.session_kind, "session_kind");
  if (payload.expires_unix_ms !== undefined) {
    adminPositiveInteger(payload.expires_unix_ms, "expires_unix_ms");
  }
  return payload;
}

function adminDeleteDeviceSessionRequest(value) {
  const payload = adminRequest(
    value,
    [...adminCarrierBaseFields(), "session_id", "reason"],
    "admin device session delete request",
  );
  validateAdminCarrierBase(payload);
  requiredAdminString(payload.session_id, "session_id");
  if (payload.reason !== undefined) {
    requiredAdminString(payload.reason, "reason");
  }
  return payload;
}

function adminRequest(value, allowed, label) {
  const payload = objectValue(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(payload)) {
    if (!allowedSet.has(key)) {
      throw invalidAdminGateway(`${key} is not an admin gateway field`);
    }
  }
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidAdminGateway(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function validateAdminCarrierBase(payload) {
  requiredAdminURA(payload.caller_ura, "caller_ura");
  requiredAdminURA(payload.callee_ura, "callee_ura");
  requiredAdminURA(payload.subject_ura, "subject_ura");
  requiredAdminString(payload.descriptor_version, "descriptor_version");
  requiredAdminString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function validateAdminAgentName(value, field) {
  const text = requiredAdminString(value, field);
  if (["system", "daemon", "gateway"].includes(text)) {
    throw invalidAdminGateway("system agent lifecycle is not allowed");
  }
  return text;
}

function validateAdminKind(value, kind) {
  if (requiredAdminString(value.profile, "profile") !== ADMIN_GATEWAY_PROFILE || requiredAdminString(value.kind, "kind") !== kind) {
    throw invalidAdminGateway(`invalid ${kind} projection`);
  }
}

function callAdminProjection(transport, method, value) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  if (
    value instanceof GatewayStatus ||
    value instanceof AdminAgentPage ||
    value instanceof AdminGatewayResult ||
    value instanceof PairingPreflight ||
    value instanceof PairingToken ||
    value instanceof DeviceCredential ||
    value instanceof DeviceSession ||
    value instanceof DeviceSessionPage
  ) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value.toJSON()))));
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidAdminGateway("admin gateway projection JSON is required");
    }
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value))));
  }
  throw invalidAdminGateway("admin gateway projection must be bytes, string, or object");
}

function validateWrapperKind(value, kind) {
  if (requiredWrapperString(value.profile, "profile") !== WRAPPERS_PROFILE || requiredWrapperString(value.kind, "kind") !== kind) {
    throw invalidWrappers(`invalid ${kind} projection`);
  }
}

function callWrapperProjection(transport, method, value) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  if (
    value instanceof WrapperFileRecord ||
    value instanceof WrapperTerminalSession ||
    value instanceof WrapperRemoteDesktopSession ||
    value instanceof WrapperBrowserSession ||
    value instanceof WrapperMediaSession
  ) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value.toJSON()))));
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidWrappers("wrapper projection JSON is required");
    }
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value))));
  }
  throw invalidWrappers("wrapper projection must be bytes, string, or object");
}

function compatibilityCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "auth_token",
    "metadata",
  ];
}

function compatibilityListModelsRequest(value) {
  const payload = compatibilityRequest(
    value,
    compatibilityCarrierBaseFields(),
    "compatibility list models request",
  );
  validateCompatibilityCarrierBase(payload);
  return payload;
}

function compatibilityChatCompletionRequest(value, stream) {
  const payload = compatibilityRequest(
    value,
    [...compatibilityCarrierBaseFields(), "request"],
    stream ? "compatibility stream chat completion request" : "compatibility chat completion request",
  );
  validateCompatibilityCarrierBase(payload);
  const request = objectValue(payload.request, "request");
  const model = requiredAbilityURA(request.model, "model");
  if (!Array.isArray(request.messages) || request.messages.length === 0) {
    throw invalidCompatibility("messages must be a non-empty array");
  }
  for (const message of request.messages) {
    objectValue(message, "message");
  }
  if (!stream && request.stream === true) {
    throw invalidCompatibility("unary chat completion request must not set stream=true");
  }
  payload.request = { ...request, model };
  if (stream) {
    payload.request.stream = true;
  }
  return payload;
}

function compatibilityFileUploadRequest(value) {
  const payload = compatibilityRequest(
    value,
    [
      ...compatibilityCarrierBaseFields(),
      "id",
      "file_id",
      "file_ref",
      "resource_ref",
      "resource_ura",
      "filename",
      "purpose",
      "owner_ura",
      "content_type",
      "content_hash",
      "bytes_b64",
      "bytes",
      "size_bytes",
      "created_at",
      "status",
    ],
    "compatibility file upload request",
  );
  validateCompatibilityCarrierBase(payload);
  requiredCompatibilityString(payload.purpose, "purpose");
  validateCompatibilityFileSelector(payload);
  validateCompatibilityFileFacts(payload, true);
  mergeCompatibilityMetadata(payload, value);
  return payload;
}

function compatibilityFileRequest(value) {
  const payload = compatibilityRequest(
    value,
    [
      ...compatibilityCarrierBaseFields(),
      "id",
      "file_id",
      "file_ref",
      "resource_ref",
      "resource_ura",
      "filename",
      "purpose",
      "owner_ura",
      "content_type",
      "content_hash",
      "bytes",
      "size_bytes",
      "created_at",
      "created",
      "status",
    ],
    "compatibility file request",
  );
  validateCompatibilityCarrierBase(payload);
  validateCompatibilityFileSelector(payload);
  validateCompatibilityFileFacts(payload, false);
  mergeCompatibilityMetadata(payload, value);
  return payload;
}

function compatibilityFileDeleteRequest(value) {
  const payload = compatibilityRequest(
    value,
    [
      ...compatibilityCarrierBaseFields(),
      "id",
      "file_id",
      "file_ref",
      "resource_ref",
      "resource_ura",
      "content_hash",
      "deleted",
    ],
    "compatibility file delete request",
  );
  validateCompatibilityCarrierBase(payload);
  validateCompatibilityFileSelector(payload);
  if (payload.deleted !== true) {
    throw invalidCompatibility("deleted must be true");
  }
  if (payload.content_hash !== undefined) {
    validateCompatibilityHash(payload.content_hash, "content_hash");
  }
  mergeCompatibilityMetadata(payload, value);
  return payload;
}

function compatibilityRequest(value, allowed, label) {
  const payload = objectValue(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(payload)) {
    if (!allowedSet.has(key)) {
      throw invalidCompatibility(`${key} is not a compatibility field`);
    }
  }
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidCompatibility(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function validateCompatibilityCarrierBase(payload) {
  requiredCompatibilityURA(payload.caller_ura, "caller_ura");
  requiredCompatibilityURA(payload.callee_ura, "callee_ura");
  requiredCompatibilityURA(payload.subject_ura, "subject_ura");
  requiredCompatibilityString(payload.descriptor_version, "descriptor_version");
  requiredCompatibilityString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.auth_token !== undefined) {
    compatibilityOptionalString(payload.auth_token, "auth_token");
  }
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function validateCompatibilityFileSelector(payload) {
  const selectors = [
    payload.id,
    payload.file_id,
    payload.file_ref,
    payload.resource_ref,
    payload.resource_ura,
  ].filter((value) => typeof value === "string" && value !== "");
  if (selectors.length === 0) {
    throw invalidCompatibility("file selector is required");
  }
  for (const field of ["id", "file_id", "filename", "purpose", "owner_ura", "content_type", "status"]) {
    if (payload[field] !== undefined && payload[field] !== "") {
      requiredCompatibilityString(payload[field], field);
    }
  }
  for (const field of ["file_ref", "resource_ref", "resource_ura"]) {
    if (payload[field] !== undefined && payload[field] !== "") {
      requiredCompatibilityURA(payload[field], field);
    }
  }
}

function validateCompatibilityFileFacts(payload, upload) {
  if (upload && !payload.filename) {
    throw invalidCompatibility("filename is required");
  }
  if (payload.content_hash !== undefined && payload.content_hash !== "") {
    validateCompatibilityHash(payload.content_hash, "content_hash");
  }
  for (const field of ["bytes", "size_bytes", "created_at", "created"]) {
    if (payload[field] !== undefined) {
      compatibilityNonNegativeInteger(payload[field], field);
    }
  }
}

function mergeCompatibilityMetadata(payload, original) {
  const carrierMetadata = objectValue(payload.metadata ?? {}, "metadata");
  const fileMetadata = objectValue(original.metadata ?? {}, "metadata");
  const merged = { ...carrierMetadata, ...fileMetadata };
  if (Object.keys(merged).length > 0) {
    payload.metadata = merged;
  }
}

function callCompatibilityProjection(transport, method, value) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  if (
    value instanceof CompatibilityModelPage ||
    value instanceof CompatibilityChatCompletion ||
    value instanceof CompatibilityChatCompletionStream ||
    value instanceof CompatibilityFile ||
    value instanceof CompatibilityFileDeleteResult
  ) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value.toJSON()))));
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidCompatibility("compatibility projection JSON is required");
    }
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value))));
  }
  throw invalidCompatibility("compatibility projection must be bytes, string, or object");
}

function surfaceListPagesRequest(value) {
  const payload = surfaceRequest(
    value,
    [...surfaceCarrierBaseFields(), "limit", "cursor"],
    "surface list pages request",
  );
  validateSurfaceCarrierBase(payload);
  if (payload.limit === undefined || payload.limit === 0) {
    payload.limit = DEFAULT_SURFACE_PAGE_SIZE;
  }
  surfacePositiveBoundedInteger(payload.limit, "limit", MAX_SURFACE_PAGE_SIZE);
  if (payload.cursor !== undefined) {
    surfaceOptionalString(payload.cursor, "cursor");
  }
  return payload;
}

function surfaceCreatePageRequest(value) {
  const payload = surfaceRequest(
    value,
    [...surfaceCarrierBaseFields(), "project_id", "folder", "visibility"],
    "surface create page request",
  );
  validateSurfaceCarrierBase(payload);
  requiredSurfaceProjectID(payload.project_id, "project_id");
  const folder = requiredSurfaceString(payload.folder, "folder");
  if (!isAbsolutePath(folder)) {
    throw invalidSurface("surface folder must be absolute");
  }
  if (payload.visibility !== undefined && !["public", "private"].includes(payload.visibility)) {
    throw invalidSurface("invalid surface visibility");
  }
  return payload;
}

function surfaceDeletePageRequest(value) {
  const payload = surfaceRequest(
    value,
    [...surfaceCarrierBaseFields(), "project_id"],
    "surface delete page request",
  );
  validateSurfaceCarrierBase(payload);
  requiredSurfaceProjectID(payload.project_id, "project_id");
  return payload;
}

function surfaceManifestRequest(value) {
  const payload = surfaceRequest(
    value,
    [...surfaceCarrierBaseFields(), "project_id"],
    "surface manifest request",
  );
  validateSurfaceCarrierBase(payload);
  requiredSurfaceProjectID(payload.project_id, "project_id");
  return payload;
}

function surfaceHealthRequest(value) {
  const payload = surfaceRequest(
    value,
    [...surfaceCarrierBaseFields(), "project_id", "surface_ref"],
    "surface health request",
  );
  validateSurfaceCarrierBase(payload);
  if (payload.project_id !== undefined && payload.project_id !== "") {
    requiredSurfaceProjectID(payload.project_id, "project_id");
  }
  if (payload.surface_ref !== undefined && payload.surface_ref !== "") {
    requiredSurfaceRef(payload.surface_ref, "surface_ref");
  }
  if (!payload.project_id && !payload.surface_ref) {
    throw invalidSurface("project_id or surface_ref is required");
  }
  return payload;
}

function surfaceRequest(value, allowed, label) {
  const payload = objectValue(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(payload)) {
    if (!allowedSet.has(key)) {
      throw invalidSurface(`${key} is not a surface field`);
    }
  }
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidSurface(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function validateSurfaceCarrierBase(payload) {
  requiredSurfaceString(payload.caller_ura, "caller_ura");
  requiredSurfaceString(payload.callee_ura, "callee_ura");
  requiredSurfaceString(payload.subject_ura, "subject_ura");
  requiredSurfaceString(payload.descriptor_version, "descriptor_version");
  requiredSurfaceString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function callSurfaceProjection(transport, method, value) {
  if (typeof transport[method] !== "function") {
    throw invalidSDK(`${method} transport function is required`);
  }
  if (value instanceof SurfacePageRecord ||
      value instanceof SurfacePagePage ||
      value instanceof SurfaceManifest ||
      value instanceof SurfacePublicPageRef ||
      value instanceof SurfaceMutationResult ||
      value instanceof SurfaceHealth) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value.toJSON()))));
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidSurface("surface projection JSON is required");
    }
    return Promise.resolve(transport[method](Buffer.from(value)));
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Promise.resolve(transport[method](Buffer.from(JSON.stringify(value))));
  }
  throw invalidSurface("surface projection must be bytes, string, or object");
}

function eventsSubscriptionRequest(value, expectedStream) {
  requiredEventStream(expectedStream, "stream");
  const payload = eventsRequest(value, [
    ...eventsCarrierBaseFields(),
    "stream",
    "filter",
    "realm",
    "owner_ura",
    "device_ura",
    "agent_ura",
    "session_id",
    "session_ura",
    "invocation_id",
    "resume_cursor",
    "heartbeat_interval_ms",
  ], "events subscription request");
  if (payload.stream === undefined || payload.stream === "") {
    payload.stream = expectedStream;
  }
  if (payload.stream !== expectedStream) {
    throw invalidEvents("event subscription stream mismatch");
  }
  validateEventsCarrierBase(payload);
  normalizeEventFilterFields(payload);
  if (payload.resume_cursor !== undefined && payload.resume_cursor !== null) {
    const cursor = new EventCursor(payload.resume_cursor);
    if (cursor.stream !== expectedStream) {
      throw invalidEvents("resume cursor stream mismatch");
    }
    payload.resume_cursor = cursor.toJSON();
  }
  if (expectedStream === "session") {
    if (payload.session_ura !== undefined && payload.session_ura !== "") {
      throw invalidEvents("session_ura cannot be converted into daemon session_id");
    }
    requiredNoWhitespaceEventString(payload.session_id, "session_id");
  }
  if (expectedStream === "invocation") {
    requiredNoWhitespaceEventString(payload.invocation_id, "invocation_id");
  }
  if (payload.heartbeat_interval_ms !== undefined) {
    eventHeartbeatInterval(payload.heartbeat_interval_ms, "heartbeat_interval_ms");
  }
  return payload;
}

function eventsDeviceEventListRequest(value) {
  const payload = eventsRequest(value, [
    ...eventsCarrierBaseFields(),
    "filter",
    "device_ura",
    "limit",
    "cursor",
  ], "events device event list request");
  validateEventsCarrierBase(payload);
  normalizeEventFilterFields(payload);
  if (payload.limit === undefined || payload.limit === 0) {
    payload.limit = DEFAULT_EVENT_PAGE_SIZE;
  }
  eventPositiveBoundedInteger(payload.limit, "limit", MAX_EVENT_PAGE_SIZE);
  if (payload.cursor !== undefined) {
    cleanOptionalString(payload.cursor, "cursor");
  }
  return payload;
}

function eventProjectionInput(value, expectedStream = "") {
  const payload = eventsRequest(value, [
    "cursor",
    "event",
    "event_id",
    "resume_token",
    "tenant_ref",
  ], "event projection input");
  const cursor = new EventCursor(payload.cursor);
  if (expectedStream && cursor.stream !== expectedStream) {
    throw invalidEvents("event cursor stream mismatch");
  }
  payload.cursor = cursor.toJSON();
  objectValue(payload.event, "event");
  return payload;
}

function eventDropReportInput(value) {
  const payload = eventsRequest(value, [
    "cursor",
    "occurred_unix_ms",
    "dropped_count",
    "reconnect_after_ms",
    "reason",
    "event_id",
    "resume_token",
    "tenant_ref",
  ], "event drop report input");
  const cursor = new EventCursor(payload.cursor);
  if (cursor.stream !== "directory") {
    throw invalidEvents("event cursor stream mismatch");
  }
  payload.cursor = cursor.toJSON();
  eventNonNegativeInteger(payload.occurred_unix_ms, "occurred_unix_ms");
  if (!Number.isInteger(payload.dropped_count) || payload.dropped_count <= 0) {
    throw invalidEvents("dropped_count must be greater than zero");
  }
  validateEventReconnectAfterMS(eventOptionalNonNegativeInteger(payload.reconnect_after_ms, "reconnect_after_ms"));
  return payload;
}

function eventTerminalInput(value) {
  const payload = eventsRequest(value, [
    "cursor",
    "occurred_unix_ms",
    "reconnect_after_ms",
    "reason",
    "event_id",
    "resume_token",
    "tenant_ref",
  ], "event terminal input");
  const cursor = new EventCursor(payload.cursor);
  if (cursor.stream !== "directory") {
    throw invalidEvents("event cursor stream mismatch");
  }
  payload.cursor = cursor.toJSON();
  eventNonNegativeInteger(payload.occurred_unix_ms, "occurred_unix_ms");
  validateEventReconnectAfterMS(eventOptionalNonNegativeInteger(payload.reconnect_after_ms, "reconnect_after_ms"));
  return payload;
}

function eventsRequest(value, allowed, label) {
  const payload = objectValue(value, label);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(payload)) {
    if (!allowedSet.has(key)) {
      throw invalidEvents(`${key} is not an events field`);
    }
  }
  for (const [key, raw] of Object.entries(payload)) {
    if (typeof raw === "string" && raw.trim() !== raw) {
      throw invalidEvents(`${key} must not contain surrounding whitespace`);
    }
  }
  return payload;
}

function validateEventsCarrierBase(payload) {
  cleanRequiredString(payload.caller_ura, "caller_ura");
  cleanRequiredString(payload.callee_ura, "callee_ura");
  cleanRequiredString(payload.subject_ura, "subject_ura");
  cleanRequiredString(payload.descriptor_version, "descriptor_version");
  cleanRequiredString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function normalizeEventFilterFields(payload) {
  const filter = payload.filter === undefined || payload.filter === null
    ? {}
    : objectValue(payload.filter, "filter");
  const fields = ["realm", "owner_ura", "device_ura", "agent_ura", "session_id", "invocation_id"];
  for (const field of fields) {
    if (filter[field] !== undefined) {
      const filtered = cleanOptionalString(filter[field], `filter.${field}`);
      if (field === "realm" || field === "session_id" || field === "invocation_id") {
        rejectEventWhitespace(filtered, field);
      }
      if (payload[field] !== undefined && payload[field] !== "" && payload[field] !== filtered) {
        throw invalidEvents(`${field} conflicts with filter field`);
      }
      payload[field] = payload[field] || filtered;
      filter[field] = filtered;
    } else if (payload[field] !== undefined && payload[field] !== "") {
      filter[field] = payload[field];
    }
  }
  if (Object.keys(filter).length > 0) {
    payload.filter = filter;
  }
  for (const field of fields) {
    if (payload[field] !== undefined && payload[field] !== "") {
      cleanOptionalString(payload[field], field);
      if (field === "realm" || field === "session_id" || field === "invocation_id") {
        rejectEventWhitespace(payload[field], field);
      }
    }
  }
}

function validateEventCursor(cursor) {
  requiredEventStream(cursor.stream, "stream");
  eventNonNegativeInteger(cursor.sequence, "sequence");
  const token = cursor.resumeToken();
  requiredEventString(token, "token");
  rejectEventWhitespace(cursor.stream, "stream");
  rejectEventWhitespace(token, "token");
  if (token !== `${cursor.stream}:${cursor.sequence}`) {
    throw invalidEvents("event cursor token must match stream sequence");
  }
}

function validateEventFrame(frame) {
  if (frame.kind.includes("drop_report") && frame.droppedCount === 0) {
    throw invalidEvents("dropped_count must be greater than zero");
  }
  if (frame.kind.includes("terminal") && !frame.terminal) {
    throw invalidEvents("terminal event frame must be terminal");
  }
}

function requiredEventStream(value, field) {
  const stream = requiredEventString(value, field);
  if (!["directory", "device", "session", "invocation"].includes(stream)) {
    throw invalidEvents("unsupported event stream");
  }
  return stream;
}

function requiredEventString(value, field) {
  if (typeof value !== "string" || value === "") {
    throw invalidEvents(`${field} is required`);
  }
  return value;
}

function requiredNoWhitespaceEventString(value, field) {
  const text = requiredEventString(value, field);
  rejectEventWhitespace(text, field);
  return text;
}

function eventOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string") {
    throw invalidEvents(`${field} must be a string or null`);
  }
  return value;
}

function eventBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidEvents(`${field} must be a boolean`);
  }
  return value;
}

function eventNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidEvents(`${field} must be a non-negative integer`);
  }
  return value;
}

function eventOptionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  return eventNonNegativeInteger(value, field);
}

function eventPositiveBoundedInteger(value, field, max) {
  if (!Number.isInteger(value) || value < 1 || value > max) {
    throw invalidEvents(`${field} exceeds bounds`);
  }
  return value;
}

function eventHeartbeatInterval(value, field) {
  if (
    !Number.isInteger(value) ||
    value < MIN_EVENT_HEARTBEAT_INTERVAL_MS ||
    value > MAX_EVENT_HEARTBEAT_INTERVAL_MS
  ) {
    throw invalidEvents(`${field} exceeds bounds`);
  }
  return value;
}

function validateEventReconnectAfterMS(value) {
  if (value !== null && value > MAX_EVENT_HEARTBEAT_INTERVAL_MS) {
    throw invalidEvents("reconnect_after_ms exceeds bounds");
  }
}

function rejectEventWhitespace(value, field) {
  if (typeof value === "string" && /\s/.test(value)) {
    throw invalidEvents(`${field} must not contain whitespace`);
  }
}

function requiredSurfaceString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidSurface(`${field} is required`);
  }
  return value;
}

function surfaceOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidSurface(`${field} must be a string or null`);
  }
  return value;
}

function requiredSurfaceProjectID(value, field) {
  const text = requiredSurfaceString(value, field);
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(text) || text.includes("..")) {
    throw invalidSurface("invalid surface project_id");
  }
  return text;
}

function requiredSurfaceRef(value, field) {
  const text = requiredSurfaceString(value, field);
  if (!text.startsWith("easynet:///r/")) {
    throw invalidSurface(`${field} must be a URA`);
  }
  return text;
}

function surfaceBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidSurface(`${field} must be a boolean`);
  }
  return value;
}

function surfaceNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidSurface(`${field} must be a non-negative integer`);
  }
  return value;
}

function surfaceOptionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  return surfaceNonNegativeInteger(value, field);
}

function surfacePositiveBoundedInteger(value, field, max) {
  if (!Number.isInteger(value) || value < 1 || value > max) {
    throw invalidSurface(`${field} exceeds bounds`);
  }
  return value;
}

function requiredMissionString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidMission(`${field} is required`);
  }
  return value;
}

function missionOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidMission(`${field} must be a string or null`);
  }
  return value;
}

function requiredMissionURA(value, field) {
  const text = requiredMissionString(value, field);
  if (!text.startsWith("easynet:///r/")) {
    throw invalidMission(`${field} must be a URA`);
  }
  return text;
}

function requiredMissionID(value, field) {
  const text = requiredMissionString(value, field);
  if (/[\\/]/.test(text)) {
    throw invalidMission(`${field} must not be path-like`);
  }
  return text;
}

function requiredMissionState(value, field) {
  const text = requiredMissionString(value, field);
  if (!["running", "ok", "partial", "error", "cancelled"].includes(text)) {
    throw invalidMission(`${field} is not a mission state`);
  }
  return text;
}

function missionBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidMission(`${field} must be a boolean`);
  }
  return value;
}

function missionNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidMission(`${field} must be a non-negative integer`);
  }
  return value;
}

function missionOptionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  return missionNonNegativeInteger(value, field);
}

function missionPositiveBoundedInteger(value, field, max) {
  if (!Number.isInteger(value) || value < 1 || value > max) {
    throw invalidMission(`${field} exceeds bounds`);
  }
  return value;
}

function missionObjectArray(value, field) {
  if (!Array.isArray(value)) {
    throw invalidMission(`${field} must be an array`);
  }
  return value.map((item) => objectValue(item, field));
}

function requiredAdminString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidAdminGateway(`${field} is required`);
  }
  return value;
}

function adminOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidAdminGateway(`${field} must be a string or null`);
  }
  return value;
}

function requiredAdminURA(value, field) {
  const text = requiredAdminString(value, field);
  if (!text.startsWith("easynet:///r/")) {
    throw invalidAdminGateway(`${field} must be a URA`);
  }
  return text;
}

function adminBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidAdminGateway(`${field} must be a boolean`);
  }
  return value;
}

function adminNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidAdminGateway(`${field} must be a non-negative integer`);
  }
  return value;
}

function adminPositiveInteger(value, field) {
  if (!Number.isInteger(value) || value < 1) {
    throw invalidAdminGateway(`${field} must be a positive integer`);
  }
  return value;
}

function adminObjectArray(value, field) {
  if (!Array.isArray(value)) {
    throw invalidAdminGateway(`${field} must be an array`);
  }
  return value.map((item) => objectValue(item, field));
}

function adminStringArray(value, field) {
  if (!Array.isArray(value)) {
    throw invalidAdminGateway(`${field} must be an array`);
  }
  return value.map((item) => requiredAdminString(item, field));
}

function requiredWrapperString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidWrappers(`${field} is required`);
  }
  return value;
}

function wrapperOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidWrappers(`${field} must be a string or null`);
  }
  return value;
}

function requiredWrapperURA(value, field) {
  const text = requiredWrapperString(value, field);
  if (!text.startsWith("easynet:///r/")) {
    throw invalidWrappers(`${field} must be a URA`);
  }
  return text;
}

function wrapperOptionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (!Number.isInteger(value) || value < 0) {
    throw invalidWrappers(`${field} must be a non-negative integer`);
  }
  return value;
}

function requiredCompatibilityString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidCompatibility(`${field} is required`);
  }
  return value;
}

function compatibilityOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidCompatibility(`${field} must be a string or null`);
  }
  return value;
}

function requiredCompatibilityURA(value, field) {
  const text = requiredCompatibilityString(value, field);
  if (!text.startsWith("easynet:///r/")) {
    throw invalidCompatibility(`${field} must be a URA`);
  }
  return text;
}

function requiredAbilityURA(value, field) {
  const text = requiredCompatibilityURA(value, field);
  if (!text.includes("/ability/")) {
    throw invalidCompatibility(`${field} must be an Ability URA`);
  }
  return text;
}

function compatibilityNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidCompatibility(`${field} must be a non-negative integer`);
  }
  return value;
}

function compatibilityObjectArray(value, field) {
  if (!Array.isArray(value)) {
    throw invalidCompatibility(`${field} must be an array`);
  }
  return value.map((item) => objectValue(item, field));
}

function validateCompatibilityHash(value, field) {
  const text = requiredCompatibilityString(value, field);
  if (!/^sha256:[0-9a-f]{64}$/.test(text)) {
    throw invalidCompatibility(`${field} must use sha256:<64 lowercase hex> form`);
  }
  return text;
}

function receiptFetchRequest(value) {
  const payload = requestObject(
    value,
    [
      "caller_ura",
      "callee_ura",
      "descriptor_ref",
      "subject_ura",
      "descriptor_version",
      "nonce_base64",
      "causal_context",
      "invocation_ura",
      "request_id",
      "trace_id",
      "metadata",
    ],
    "receipt fetch request",
  );
  cleanRequiredString(payload.caller_ura, "caller_ura");
  cleanRequiredString(payload.callee_ura, "callee_ura");
  cleanRequiredString(payload.descriptor_ref, "descriptor_ref");
  cleanRequiredString(payload.subject_ura, "subject_ura");
  cleanRequiredString(payload.descriptor_version, "descriptor_version");
  cleanRequiredString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (!payload.invocation_ura && !payload.request_id && !payload.trace_id) {
    throw invalidReceipt("exactly one receipt fetch selector is required");
  }
  const selectors = [payload.invocation_ura, payload.request_id, payload.trace_id].filter(Boolean);
  if (selectors.length !== 1) {
    throw invalidReceipt("exactly one receipt fetch selector is required");
  }
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function receiptHistoryRequest(value) {
  const payload = requestObject(
    value,
    [...receiptCarrierBaseFields(), "arguments"],
    "receipt history request",
  );
  validateReceiptCarrierBase(payload);
  if (payload.arguments !== undefined) {
    objectValue(payload.arguments, "arguments");
  }
  return payload;
}

function validateReceiptCarrierBase(payload) {
  cleanRequiredString(payload.caller_ura, "caller_ura");
  cleanRequiredString(payload.callee_ura, "callee_ura");
  cleanRequiredString(payload.subject_ura, "subject_ura");
  cleanRequiredString(payload.descriptor_version, "descriptor_version");
  cleanRequiredString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (
    payload.timeout_ms !== undefined &&
    (!Number.isInteger(payload.timeout_ms) || payload.timeout_ms < 0)
  ) {
    throw invalidReceipt("timeout_ms must be non-negative");
  }
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
}

function receiptChainRequest(value) {
  const payload = requestObject(value, ["receipts", "metadata"], "receipt chain request");
  if (!Array.isArray(payload.receipts) || payload.receipts.length === 0) {
    throw invalidReceipt("receipts must be a non-empty array");
  }
  payload.receipts = payload.receipts.map((receipt) =>
    receipt instanceof ReceiptRef ? receipt.toJSON() : new ReceiptRef(receipt).toJSON(),
  );
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function publicationCarrierBaseFields() {
  return [
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
  ];
}

function localResourceRefRequest(value) {
  const payload = requestObject(value, ["path", "capability"], "local resource ref request");
  const resourcePath = publicationRequiredString(payload.path, "path");
  if (!isAbsolutePath(resourcePath)) {
    throw invalidPublication("absolute resource path is required", {
      reason: "resource_ref_path_must_be_absolute",
      operation: "build_local_resource_ref",
    });
  }
  if (!["list", "stat", "read", "write"].includes(payload.capability)) {
    throw invalidPublication("capability must be one of list, stat, read, or write", {
      reason: "resource_ref_capability_invalid",
      operation: "build_local_resource_ref",
    });
  }
  return payload;
}

function packageValidationRequest(value) {
  const payload = requestObject(
    value,
    ["package_path", "manifest", "metadata"],
    "package validation request",
  );
  if (payload.package_path !== undefined) {
    publicationRequiredString(payload.package_path, "package_path");
  }
  if (payload.manifest !== undefined) {
    objectValue(payload.manifest, "manifest");
  }
  if (!payload.package_path && payload.manifest === undefined) {
    throw invalidPublication("package_path or manifest is required");
  }
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function abilityDeployRequest(value) {
  const payload = requestObject(
    value,
    [...publicationCarrierBaseFields(), "resource_ref", "node_id"],
    "ability deploy request",
  );
  validatePublicationCarrierBase(payload, "complete deploy invocation carrier is required");
  validateResourceRef(payload.resource_ref);
  publicationRequiredString(payload.node_id, "node_id");
  return payload;
}

function pluginInstallRequest(value) {
  const payload = requestObject(value, ["source", "metadata"], "plugin install request");
  publicationRequiredString(payload.source, "source");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function publishedAbilityQuery(value) {
  const payload = requestObject(
    value,
    [...publicationCarrierBaseFields(), "limit", "cursor", "owner_ura", "ability_ura"],
    "published ability query",
  );
  validatePublicationCarrierBase(payload, "complete publication query carrier is required");
  if (payload.limit === undefined || payload.limit === 0) {
    payload.limit = DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE;
  }
  validatePublishedAbilityLimit(payload.limit);
  if (payload.cursor !== undefined) {
    cleanOptionalString(payload.cursor, "cursor");
  }
  if (payload.owner_ura !== undefined) {
    publicationRequiredString(payload.owner_ura, "owner_ura");
  }
  if (payload.ability_ura !== undefined) {
    publicationRequiredString(payload.ability_ura, "ability_ura");
  }
  return payload;
}

function showAbilityRequest(value) {
  const payload = requestObject(
    value,
    [...publicationCarrierBaseFields(), "descriptor_ref", "owner_ura"],
    "show ability request",
  );
  publicationRequiredString(payload.descriptor_ref, "descriptor_ref");
  if (hasAnyField(payload, publicationCarrierBaseFields())) {
    validatePublicationCarrierBase(payload, "complete show invocation carrier is required");
  } else if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  if (payload.owner_ura !== undefined) {
    publicationRequiredString(payload.owner_ura, "owner_ura");
  }
  return payload;
}

function abilityImplLifecycleRequest(value) {
  const payload = requestObject(
    value,
    [...publicationCarrierBaseFields(), "impl_id", "ability_ura"],
    "ability impl lifecycle request",
  );
  publicationRequiredString(payload.impl_id, "impl_id");
  publicationRequiredString(payload.ability_ura, "ability_ura");
  if (hasAnyField(payload, publicationCarrierBaseFields())) {
    validatePublicationCarrierBase(payload, "complete ability impl lifecycle invocation carrier is required");
  } else if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function unpublishAbilityRequest(value) {
  const payload = requestObject(
    value,
    [...publicationCarrierBaseFields(), "ability_ura"],
    "unpublish ability request",
  );
  validatePublicationCarrierBase(payload, "complete unpublish invocation carrier is required");
  publicationRequiredString(payload.ability_ura, "ability_ura");
  return payload;
}

function validatePublicationCarrierBase(payload, message) {
  try {
    cleanRequiredString(payload.caller_ura, "caller_ura");
    cleanRequiredString(payload.callee_ura, "callee_ura");
    cleanRequiredString(payload.subject_ura, "subject_ura");
    cleanRequiredString(payload.descriptor_version, "descriptor_version");
    cleanRequiredString(payload.nonce_base64, "nonce_base64");
    objectValue(payload.causal_context, "causal_context");
    if (payload.metadata !== undefined) {
      objectValue(payload.metadata, "metadata");
    }
  } catch (error) {
    if (error instanceof SDKError) {
      throw invalidPublication(message);
    }
    throw error;
  }
}

function validateResourceRef(value) {
  const ref = objectValue(value, "resource_ref");
  publicationRequiredString(ref.resource_ura, "resource_ura");
  publicationRequiredString(ref.owner_ura, "owner_ura");
  const namespace = publicationRequiredString(ref.namespace, "namespace");
  publicationRequiredString(ref.capability, "capability");
  publicationRequiredString(ref.revision, "revision");
  if (["axon", "daemon", "easynet", "internal", "system"].includes(namespace.toLowerCase())) {
    throw invalidPublication("resource_ref namespace is reserved", {
      reason: "resource_ref_namespace_reserved",
    });
  }
  if (ref.display_path !== undefined) {
    cleanOptionalString(ref.display_path, "display_path");
  }
}

function publicationRequiredString(value, field) {
  try {
    return cleanRequiredString(value, field);
  } catch (error) {
    if (error instanceof SDKError) {
      throw invalidPublication(`${field} is required`);
    }
    throw error;
  }
}

function validatePublishedAbilityLimit(value) {
  if (!Number.isInteger(value) || value < 1 || value > MAX_PUBLISHED_ABILITY_PAGE_SIZE) {
    throw invalidPublication(`limit must be between 1 and ${MAX_PUBLISHED_ABILITY_PAGE_SIZE}`);
  }
}

function hasAnyField(payload, fields) {
  return fields.some((field) => Object.hasOwn(payload, field));
}

function isAbsolutePath(value) {
  return value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\");
}

function hostBindingRequest(value) {
  const payload = hostObject(value, "host stream binding request");
  hostRejectUnknownFields(payload, [
    "binding_id",
    "descriptor_ref",
    "endpoint",
    "frame_schema",
    "cleanup",
    "timeout_ms",
    "readiness",
    "metadata",
  ]);
  validateHostStreamBindingRequest(payload);
  if (payload.cleanup !== undefined && payload.cleanup !== null) {
    objectValue(payload.cleanup, "cleanup");
  }
  if (payload.readiness !== undefined && payload.readiness !== null) {
    objectValue(payload.readiness, "readiness");
  }
  if (payload.metadata !== undefined && payload.metadata !== null) {
    objectValue(payload.metadata, "metadata");
  }
  return payload;
}

function validateHostStreamBindingRequest(payload) {
  requiredHostBindingString(payload.binding_id, "binding_id");
  requiredHostBindingString(payload.descriptor_ref, "descriptor_ref");
  const endpoint = requiredHostBindingString(payload.endpoint, "endpoint");
  if (!isAbsoluteHostEndpoint(endpoint)) {
    throw invalidHostBinding("host stream endpoint must be absolute");
  }
  if (payload.frame_schema !== HOST_STREAM_FRAME_SCHEMA) {
    throw invalidHostBinding("frame_schema must be host-stream-frame.schema.json");
  }
  if (
    payload.timeout_ms !== undefined &&
    payload.timeout_ms !== null &&
    (!Number.isInteger(payload.timeout_ms) || payload.timeout_ms < 0)
  ) {
    throw invalidHostBinding("timeout_ms must be non-negative or null");
  }
}

function hostStreamEnvelope(value) {
  const envelope = hostObject(value, "host stream envelope");
  hostRejectUnknownFields(envelope, ["request"]);
  const request = hostObject(envelope.request, "request");
  hostRejectUnknownFields(request, ["fn", "args", "call_id", "caller", "parent_receipt"]);
  requiredHostBindingString(request.fn, "fn");
  requiredHostBindingString(request.call_id, "call_id");
  requiredHostBindingString(request.caller, "caller");
  return { request };
}

function hostStreamTerminalSummary(value) {
  const summary = hostObject(value, "summary");
  hostRejectUnknownFields(summary, ["output_hash", "frames", "metadata"]);
  const outputHash = normalizeHostOutputHash(summary.output_hash, "output_hash");
  const frames = hostNonNegativeInteger(summary.frames, "frames");
  const projected = {
    output_hash: outputHash,
    frames,
    metadata: objectValue(summary.metadata ?? {}, "metadata"),
  };
  return projected;
}

function hostBindingErrorDTO(error) {
  if (error instanceof SDKError) {
    return {
      code: error.code,
      stage: error.stage || "host_binding",
      message: error.message,
      retry: error.retry || RetryHint.NEVER,
      source: error.source || null,
      invocation_id: error.invocationId || null,
      receipt_ura: error.receiptURA || null,
      details: error.details ?? {},
    };
  }
  if (error instanceof Error) {
    return {
      code: ErrorCode.GENERIC,
      stage: "host_binding",
      message: error.message,
      retry: RetryHint.NEVER,
      details: {},
    };
  }
  if (error && typeof error === "object" && !Array.isArray(error)) {
    const value = objectValue(error, "error");
    requiredHostBindingString(value.code, "code");
    requiredHostBindingString(value.stage, "stage");
    requiredHostBindingString(value.retry, "retry");
    requiredHostBindingString(value.message ?? "host binding error", "message");
    return {
      ...value,
      details: objectValue(value.details ?? {}, "details"),
    };
  }
  throw invalidHostBinding("error is required");
}

function hostObject(value, field) {
  try {
    return objectValue(value, field);
  } catch (error) {
    if (error instanceof SDKError) {
      throw invalidHostBinding(`${field} must be an object`);
    }
    throw error;
  }
}

function requiredHostBindingString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidHostBinding(`${field} is required`);
  }
  return value;
}

function hostNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidHostBinding(`${field} must be a non-negative integer`);
  }
  return value;
}

function hostOptionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  return hostNonNegativeInteger(value, field);
}

function normalizeHostOutputHash(value, field) {
  const text = requiredHostBindingString(value, field);
  if (!/^sha256:[0-9a-f]{64}$/.test(text)) {
    throw invalidHostBinding(`${field} must use sha256:<64 lowercase hex> form`);
  }
  return text;
}

function validateHostHashState(state) {
  if (state.frames === 0) {
    if (state.lastSeq !== null) {
      throw invalidHostBinding("host stream hash state cannot have last_seq when frames is zero");
    }
    return;
  }
  if (state.lastSeq !== state.frames - 1) {
    throw invalidHostBinding("host stream hash state last_seq must match frames");
  }
}

function validateHostHashFold(state, seq) {
  validateHostHashState(state);
  if (seq !== state.frames) {
    throw invalidHostBinding("host stream hash sequence gap");
  }
}

function canonicalJSONString(value) {
  if (value === null) {
    return "null";
  }
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw invalidHostBinding("host stream frame is not valid JSON");
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((item) => canonicalJSONString(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    const keys = Object.keys(value).sort();
    const parts = [];
    for (const key of keys) {
      const item = value[key];
      if (item === undefined || typeof item === "function" || typeof item === "symbol") {
        throw invalidHostBinding("host stream frame is not valid JSON");
      }
      parts.push(`${JSON.stringify(key)}:${canonicalJSONString(item)}`);
    }
    return `{${parts.join(",")}}`;
  }
  throw invalidHostBinding("host stream frame is not valid JSON");
}

function foldHostOutputHash(previousOutputHash, seq, canonicalJSON) {
  const hashHex = normalizeHostOutputHash(previousOutputHash, "previous output_hash").slice("sha256:".length);
  const previous = Buffer.from(hashHex, "hex");
  const seqBytes = Buffer.alloc(8);
  seqBytes.writeBigUInt64BE(BigInt(seq));
  const digest = createHash("sha256")
    .update(previous)
    .update(seqBytes)
    .update(Buffer.from(canonicalJSON, "utf8"))
    .digest("hex");
  return `sha256:${digest}`;
}

function isAbsoluteHostEndpoint(endpoint) {
  return endpoint.startsWith("/") || endpoint.startsWith("unix:///");
}

function hostRejectUnknownFields(value, allowed) {
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) {
      throw invalidHostBinding(`${key} is not a host_binding field`);
    }
  }
}

function requiredHealthString(value, field) {
  if (typeof value !== "string" || value === "") {
    throw invalidHealth(`${field} must be a non-empty string`);
  }
  return value;
}

function optionalHealthString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string") {
    throw invalidHealth(`${field} must be a string or null`);
  }
  return value;
}

function requiredHealthBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidHealth(`${field} must be a boolean`);
  }
  return value;
}

function requiredHealthNonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidHealth(`${field} must be a non-negative integer`);
  }
  return value;
}

function optionalHealthNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  return requiredHealthNonNegativeInteger(value, field);
}

function optionalHealthObject(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  try {
    return objectValue(value, field);
  } catch (error) {
    if (error instanceof SDKError) {
      throw invalidHealth(`${field} must be an object or null`);
    }
    throw error;
  }
}

function healthDiagnostics(value) {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    throw invalidHealth("diagnostics must be an array of strings");
  }
  return [...value];
}

function jsonPayloadBytes(value, field) {
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    if (value.length === 0) {
      throw invalidReceipt(`${field} is required`);
    }
    return Buffer.from(value);
  }
  if (typeof value === "string") {
    if (value.trim() === "") {
      throw invalidReceipt(`${field} is required`);
    }
    return Buffer.from(value);
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return Buffer.from(JSON.stringify(value));
  }
  throw invalidReceipt(`${field} must be bytes, string, or object`);
}

function normalizeReceiptHashHex(value, field) {
  const text = cleanRequiredString(value, field).toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(text)) {
    throw invalidReceipt(`${field} must be 64 lowercase hex characters`);
  }
  return text;
}

function optionalNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (!Number.isInteger(value) || value < 0) {
    throw invalidReceipt(`${field} must be a non-negative integer`);
  }
  return value;
}

function validateDirectoryBase(payload, requireLimit) {
  cleanRequiredString(payload.caller_ura, "caller_ura");
  cleanRequiredString(payload.callee_ura, "callee_ura");
  cleanRequiredString(payload.subject_ura, "subject_ura");
  cleanRequiredString(payload.descriptor_version, "descriptor_version");
  cleanRequiredString(payload.nonce_base64, "nonce_base64");
  objectValue(payload.causal_context, "causal_context");
  if (payload.metadata !== undefined) {
    objectValue(payload.metadata, "metadata");
  }
  if (payload.cursor !== undefined) {
    cleanOptionalString(payload.cursor, "cursor");
  }
  if (requireLimit) {
    validateDirectoryLimit(payload.limit);
  } else if (payload.limit !== undefined) {
    validateDirectoryLimit(payload.limit);
  }
}

function applyDirectoryDefaultLimit(payload) {
  if (payload.limit === undefined || payload.limit === 0) {
    payload.limit = DEFAULT_DIRECTORY_PAGE_SIZE;
  }
}

function validateDirectoryLimit(value) {
  if (!Number.isInteger(value) || value < 1 || value > MAX_DIRECTORY_PAGE_SIZE) {
    throw invalidDirectory(`limit must be between 1 and ${MAX_DIRECTORY_PAGE_SIZE}`);
  }
}

function cleanRequiredString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidSDK(`${field} is required`);
  }
  return value;
}

function cleanOptionalString(value, field) {
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidSDK(`${field} must be a string without surrounding whitespace`);
  }
  return value;
}

function assertDraft(value) {
  if (!(value instanceof InvocationDraft)) {
    throw invalidInvocation("invocation draft is required");
  }
  return value;
}

function assertHandle(value) {
  if (!(value instanceof InvocationHandle)) {
    throw invalidRuntime("invocation handle is required");
  }
  return value;
}

function requireBoundRuntime(runtime) {
  if (!runtime || typeof runtime.awaitResult !== "function") {
    throw invalidRuntime("invocation handle is not bound to a runtime client");
  }
  return runtime;
}

function rejectRuntimeFields(value, allowed) {
  const set = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!set.has(key)) {
      throw invalidRuntime(`${key} is not a runtime field`);
    }
  }
}

function positiveRuntimeInteger(value, field) {
  if (!Number.isInteger(value) || value <= 0) {
    throw invalidRuntime(`${field} must be a positive integer`);
  }
  return value;
}

function optionalRuntimeNonNegativeInteger(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (!Number.isInteger(value) || value < 0) {
    throw invalidRuntime(`${field} must be a non-negative integer`);
  }
  return value;
}

function requiredRuntimeString(value, field) {
  if (typeof value !== "string" || value === "") {
    throw invalidRuntime(`${field} must be a non-empty string`);
  }
  return value;
}

function optionalRuntimeString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string") {
    throw invalidRuntime(`${field} must be a string or null`);
  }
  return value;
}

function runtimeBoolean(value, field) {
  if (typeof value !== "boolean") {
    throw invalidRuntime(`${field} must be a boolean`);
  }
  return value;
}

function optionalRuntimeObject(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  try {
    return objectValue(value, field);
  } catch (error) {
    if (error instanceof SDKError) {
      throw invalidRuntime(`${field} must be an object or null`);
    }
    throw error;
  }
}

function validateInvocationHandleMonotonicity(handle) {
  const terminalEvents = handle.events.filter((event) => event.terminal);
  if (terminalEvents.length > 1) {
    throw invalidRuntime("invocation handle can contain at most one terminal event");
  }
  if (terminalEvents.length === 1) {
    const terminalEvent = terminalEvents[0];
    if (!handle.terminal) {
      throw invalidRuntime("terminal event requires terminal handle state");
    }
    if (terminalEvent.state !== handle.state) {
      throw invalidRuntime("terminal event state must match handle state");
    }
  }
  if (handle.result !== null && !handle.terminal) {
    throw invalidRuntime("result requires terminal handle state");
  }
}

function boundedRuntimeLimit(value, field, defaultValue) {
  const limit = optionalRuntimeNonNegativeInteger(value, field);
  if (limit === null || limit === 0) {
    return defaultValue;
  }
  return limit;
}

function streamBackpressureTerminal(maxBufferedEvents, bufferedEvents) {
  return {
    kind: "backpressure",
    frame_type: "terminal",
    state: "Failed",
    terminal: true,
    error: backpressureErrorJSON("stream callback queue limit exceeded", {
      max_buffered_events: maxBufferedEvents,
      buffered_events: bufferedEvents,
    }),
  };
}

function bidiBackpressureTerminal(direction, maxBufferedFrames, bufferedFrames) {
  return {
    kind: "backpressure",
    frame_type: "terminal",
    direction,
    state: "Failed",
    terminal: true,
    error: backpressureErrorJSON("bidi callback queue limit exceeded", {
      direction,
      max_buffered_frames: maxBufferedFrames,
      buffered_frames: bufferedFrames,
    }),
  };
}

function backpressureErrorJSON(message, details = {}) {
  return {
    code: ErrorCode.ADMISSION_DENIED,
    stage: "runtime",
    retry: RetryHint.AFTER_BACKOFF,
    message,
    details: {
      reason: "callback_queue_overflow",
      wire_code: "RESOURCE_EXHAUSTED",
      ...details,
    },
  };
}

function backpressureSDK(message, details = {}) {
  return new SDKError({
    code: ErrorCode.ADMISSION_DENIED,
    stage: "runtime",
    retry: RetryHint.AFTER_BACKOFF,
    message,
    details: {
      reason: "callback_queue_overflow",
      wire_code: "RESOURCE_EXHAUSTED",
      ...details,
    },
  });
}

function validateRuntimeBase64(value, field) {
  let raw;
  try {
    raw = Buffer.from(value, "base64");
  } catch {
    throw invalidRuntime(`${field} must be base64`);
  }
  if (raw.length === 0 || raw.toString("base64") !== value) {
    throw invalidRuntime(`${field} must be base64`);
  }
}

function validateRuntimeHex(value, field, expectedLength = null) {
  if (!/^[0-9a-f]+$/i.test(value)) {
    throw invalidRuntime(`${field} must be hex`);
  }
  if (expectedLength !== null && value.length !== expectedLength) {
    throw invalidRuntime(`${field} must be ${expectedLength} hex characters`);
  }
}

function validatePreparedHash(canonicalBytesBase64, canonicalHashHex) {
  if (!canonicalHashHex) {
    return;
  }
  const normalized = canonicalHashHex.startsWith("sha256:")
    ? canonicalHashHex.slice("sha256:".length)
    : canonicalHashHex;
  validateRuntimeHex(normalized, "canonical_hash_hex", 64);
  const computed = createHash("sha256")
    .update(Buffer.from(canonicalBytesBase64, "base64"))
    .digest("hex");
  if (computed !== normalized.toLowerCase()) {
    throw invalidRuntime("canonical_hash_hex must match canonical_bytes_base64");
  }
}

function normalizeErrorCode(code) {
  const text = requiredWireString(code, "code");
  if (!ERROR_CODES.has(text)) {
    throw invalidDaemonError(`unknown daemon error code: ${text}`);
  }
  return text;
}

export function profileSourceRef(profile) {
  if (typeof profile !== "string") {
    throw invalidSDK("profile must be a string");
  }
  const clean = profile.trim();
  return clean ? `node_sdk.profile.${clean}` : "";
}

export function profileErrorDetails(profile, details = {}) {
  const value = objectValue(details, "details");
  if (value.profile === undefined) {
    value.profile = profile;
  }
  if (value.source_ref === undefined) {
    value.source_ref = profileSourceRef(profile);
  }
  return value;
}

function detailString(details, key) {
  const value = details?.[key];
  return typeof value === "string" ? value : "";
}

function errorClassForCode(code) {
  switch (code) {
    case ErrorCode.INVALID_ARGUMENT:
    case ErrorCode.NULL_POINTER:
    case ErrorCode.INVALID_UTF8:
    case ErrorCode.INVALID_INVOCATION:
      return ErrorClass.VALIDATION;
    case ErrorCode.INVALID_HANDLE:
      return ErrorClass.HANDLE;
    case ErrorCode.NOT_INITIALIZED:
    case ErrorCode.ALREADY_INIT:
      return ErrorClass.LIFECYCLE;
    case ErrorCode.DAEMON_OFFLINE:
    case ErrorCode.TRANSPORT:
      return ErrorClass.AVAILABILITY;
    case ErrorCode.PERMISSION_DENIED:
      return ErrorClass.PERMISSION;
    case ErrorCode.ADMISSION_DENIED:
    case ErrorCode.ABILITY_FAILED:
      return ErrorClass.ADMISSION;
    case ErrorCode.ABILITY_NOT_FOUND:
    case ErrorCode.ROUTE_UNAVAILABLE:
    case ErrorCode.NOT_FOUND:
      return ErrorClass.ROUTING;
    case ErrorCode.TIMEOUT:
      return ErrorClass.TIMEOUT;
    case ErrorCode.CANCELLED:
      return ErrorClass.CANCELLATION;
    case ErrorCode.PROTOCOL_MISMATCH:
    case ErrorCode.PROTOCOL:
      return ErrorClass.PROTOCOL;
    case ErrorCode.VERSION_MISMATCH:
    case ErrorCode.VERSION_INCOMPATIBLE:
      return ErrorClass.VERSION;
    case ErrorCode.CONTROL_ONLY:
      return ErrorClass.CONTROL;
    case ErrorCode.NOT_IMPLEMENTED:
      return ErrorClass.UNSUPPORTED;
    default:
      return ErrorClass.GENERIC;
  }
}

function parseRetryHint(value) {
  for (const hint of Object.values(RetryHint)) {
    if (value === hint) {
      return value;
    }
  }
  throw invalidDaemonError("retry must be never, safe, after_backoff, or unknown");
}

function parseJSON(raw, label) {
  const text = decodeText(raw);
  let decoded;
  try {
    decoded = JSON.parse(text);
  } catch (cause) {
    throw new SDKError({
      code: ErrorCode.INVALID_ARGUMENT,
      stage: "decode",
      retry: RetryHint.NEVER,
      message: `decode ${label} JSON: ${cause.message}`,
      cause,
    });
  }
  return objectValue(decoded, `${label} JSON`);
}

function decodeText(raw) {
  if (Buffer.isBuffer(raw) || raw instanceof Uint8Array) {
    return Buffer.from(raw).toString("utf8");
  }
  if (typeof raw === "string") {
    return raw;
  }
  throw invalidDaemonError("JSON payload must be bytes or string");
}

function rejectUnknownFields(value, allowed) {
  const set = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!set.has(key)) {
      throw invalidInvocation(`${key} is not an invocation field`);
    }
  }
}

function validateInvocationPayloadChoice(draft) {
  const hasArguments = draft.argumentsBase64 !== "";
  if (draft.hasArgs === hasArguments) {
    throw invalidInvocation("exactly one of args or arguments_base64 is required");
  }
}

function validateBase64(value, field, expectedLength = null) {
  const raw = Buffer.from(value, "base64");
  if (raw.toString("base64") !== value) {
    throw invalidInvocation(`${field} must be base64`);
  }
  if (expectedLength !== null && raw.length !== expectedLength) {
    throw invalidInvocation(`${field} must decode to ${expectedLength} bytes`);
  }
}

function requiredBuilderString(value, field) {
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidInvocation(`${field} is required`);
  }
  return value;
}

function requiredWireString(value, field) {
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidDaemonError(`${field} is required`);
  }
  return value;
}

function stringValue(value, field, allowEmpty = false) {
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value !== "string" || (!allowEmpty && value === "")) {
    throw invalidDaemonError(`${field} must be a string`);
  }
  return value;
}

function objectValue(value, field) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw invalidDaemonError(`${field} must be an object`);
  }
  return { ...value };
}

function nonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidDaemonError(`${field} must be a non-negative integer`);
  }
  return value;
}

function booleanValue(value, field) {
  if (typeof value !== "boolean") {
    throw invalidDaemonError(`${field} must be a boolean`);
  }
  return value;
}

function stringMap(value, field) {
  const map = objectValue(value, field);
  for (const item of Object.values(map)) {
    if (typeof item !== "string") {
      throw invalidDaemonError(`${field} must map strings to strings`);
    }
  }
  return map;
}

function boolMap(value, field) {
  const map = objectValue(value, field);
  for (const item of Object.values(map)) {
    if (typeof item !== "boolean") {
      throw invalidDaemonError(`${field} must map strings to booleans`);
    }
  }
  return map;
}

function requiredAuthorityString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw invalidAuthority(`${field} is required`);
  }
  return value;
}

function authorityOptionalString(value, field) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw invalidAuthority(`${field} must be a string or null`);
  }
  return value;
}

function requiredAuthorityInteger(value, field) {
  if (!Number.isInteger(value)) {
    throw invalidAuthority(`${field} must be an integer`);
  }
  return value;
}

function requiredAuthorityStringArray(value, field) {
  if (!Array.isArray(value) || value.length === 0) {
    throw invalidAuthority(`${field} must be a non-empty string list`);
  }
  return value.map((item) => requiredAuthorityString(item, field));
}

function requiredAuthorityBase64(value, field) {
  const text = requiredAuthorityString(value, field);
  decodeAuthorityBase64(text, field);
  return text;
}

function decodeAuthorityBase64(value, field) {
  let decoded;
  try {
    decoded = Buffer.from(value, "base64");
  } catch (cause) {
    throw invalidAuthority(`${field} must be base64`, { cause: cause.message });
  }
  if (decoded.length === 0 || decoded.toString("base64") !== value) {
    throw invalidAuthority(`${field} must be base64`);
  }
  return decoded;
}

function decodeAuthorityMetadata(value, label) {
  const text = requiredAuthorityString(value, `${label} metadata value`);
  const decoded = decodeAuthorityBase64(text, `${label} metadata`);
  let wire;
  try {
    wire = objectValue(JSON.parse(decoded.toString("utf8")), `${label} metadata`);
  } catch (cause) {
    if (cause instanceof SDKError) {
      throw cause;
    }
    throw invalidAuthority(`${label} metadata JSON parse failed`, { cause: cause.message });
  }
  const payload = objectValue(wire.payload, `${label} metadata payload`);
  const signatureBase64 = requiredAuthorityBase64(wire.signature, `${label} metadata signature`);
  return { payload, signatureBase64 };
}

function decodeAuthorityMetadataProjection(raw, metadataKey, label) {
  const text = decodeText(raw).trim();
  if (!text) {
    throw invalidAuthority(`${label} metadata projection is required`);
  }
  if (text.startsWith("{")) {
    const projection = parseJSON(text, `${label} metadata projection`);
    for (const key of ["metadata_value", "value"]) {
      const value = projection[key];
      if (typeof value === "string" && value.trim() !== "") {
        return value.trim();
      }
    }
    if (projection.metadata !== undefined) {
      const value = authorityMetadataValue(objectValue(projection.metadata, "metadata"), metadataKey);
      if (value) {
        return value;
      }
    }
    throw invalidAuthority(`${label} metadata projection missing metadata_value`);
  }
  if (text.startsWith("\"")) {
    try {
      const value = JSON.parse(text);
      return requiredAuthorityString(value, `${label} metadata value`);
    } catch (cause) {
      if (cause instanceof SDKError) {
        throw cause;
      }
      throw invalidAuthority(`${label} metadata value JSON parse failed`, { cause: cause.message });
    }
  }
  return text;
}

function validateAuthorityMetadataEnvelope(kind, key) {
  if (kind === "delegation" && key === DELEGATION_METADATA_KEY) {
    return;
  }
  if (kind === "session_authority" && key === SESSION_AUTHORITY_METADATA_KEY) {
    return;
  }
  throw invalidAuthority("authority metadata key does not match authority kind");
}

function validateDelegationProof(proof) {
  if (proof.expiresAtMS <= proof.issuedAtMS) {
    throw invalidAuthority("delegation authority expires_at_ms must be greater than issued_at_ms");
  }
  if (proof.signature.length === 0) {
    throw invalidAuthority("delegation authority signature is required");
  }
}

function validateSessionAuthority(authority) {
  if (authority.expiresAtMS <= authority.issuedAtMS) {
    throw invalidAuthority("session authority expires_at_ms must be greater than issued_at_ms");
  }
  if (authority.signature.length === 0) {
    throw invalidAuthority("session authority signature is required");
  }
}

function validateDelegationRequest(request) {
  if (request.expiresAtMS <= request.issuedAtMS) {
    throw invalidAuthority("delegation authority expires_at_ms must be greater than issued_at_ms");
  }
  rejectAuthorityPrivateKeyMetadata(request.metadata);
}

function validateSessionAuthorityRequest(request) {
  if (request.expiresAtMS <= request.issuedAtMS) {
    throw invalidAuthority("session authority expires_at_ms must be greater than issued_at_ms");
  }
  rejectAuthorityPrivateKeyMetadata(request.metadata);
}

function rejectAuthorityPrivateKeyMetadata(metadata) {
  for (const key of Object.keys(metadata ?? {})) {
    switch (key.trim().toLowerCase()) {
      case "private_key":
      case "private_key_seed":
      case "private_key_seed_base64":
      case "private_key_hex":
      case "signing_key":
      case "ed25519_seed":
        throw invalidAuthority("private key material must not be supplied to authority facade");
      default:
        break;
    }
  }
}

function validateAuthorityMetadata(metadata) {
  const value = objectValue(metadata ?? {}, "metadata");
  const delegation = authorityMetadataValue(value, DELEGATION_METADATA_KEY);
  const session = authorityMetadataValue(value, SESSION_AUTHORITY_METADATA_KEY);
  if (delegation && session) {
    throw invalidAuthority("invocation authority metadata is ambiguous");
  }
}

function authorityMetadataValue(metadata, key) {
  if (!metadata || !Object.hasOwn(metadata, key) || metadata[key] === null || metadata[key] === undefined) {
    return "";
  }
  if (typeof metadata[key] !== "string") {
    throw invalidAuthority(`${key} must be a string metadata value`);
  }
  return metadata[key].trim();
}

function withAbortSignal(operation, signal, onAbort) {
  if (!signal) {
    return Promise.resolve(operation);
  }
  if (signal.aborted) {
    return Promise.resolve(onAbort()).then(() => {
      throw cancelledSDK(abortReason(signal));
    });
  }
  let abortHandler;
  const abort = new Promise((_, reject) => {
    abortHandler = () => {
      Promise.resolve(onAbort())
        .catch(() => {})
        .finally(() => reject(cancelledSDK(abortReason(signal))));
    };
    signal.addEventListener("abort", abortHandler, { once: true });
  });
  return Promise.race([Promise.resolve(operation), abort]).finally(() => {
    signal.removeEventListener("abort", abortHandler);
  });
}

function abortReason(signal) {
  const reason = signal?.reason;
  if (typeof reason === "string" && reason.trim() !== "") {
    return reason;
  }
  if (reason && typeof reason.message === "string" && reason.message.trim() !== "") {
    return reason.message;
  }
  return "aborted";
}

function isTerminalFrame(value) {
  if (!value || typeof value !== "object") {
    return false;
  }
  if (value.terminal === true) {
    return true;
  }
  for (const key of ["state", "frame_type", "event_type", "type", "kind"]) {
    if (terminalToken(value[key])) {
      return true;
    }
  }
  return false;
}

function terminalToken(value) {
  if (typeof value !== "string") {
    return false;
  }
  const token = value.toLowerCase().replace(/[^a-z]/g, "");
  return [
    "terminal",
    "closed",
    "completed",
    "failed",
    "cancelled",
    "canceled",
    "timedout",
    "done",
  ].includes(token);
}

function invalidSDK(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "sdk",
    retry: RetryHint.NEVER,
    message,
  });
}

function invalidDirectory(message, details = {}) {
  return invalidProfile(DIRECTORY_IDENTITY_PROFILE, "directory", message, details);
}

function invalidReceipt(message, details = {}) {
  return invalidProfile(RECEIPT_PROFILE, "receipt", message, details);
}

function invalidPublication(message, details = {}) {
  return invalidProfile(PUBLICATION_PROFILE, "publication", message, details);
}

function invalidHostBinding(message, details = {}) {
  return invalidProfile(HOST_BINDING_PROFILE, "host_binding", message, details);
}

function invalidHealth(message, details = {}) {
  return invalidProfile(HEALTH_PROFILE, "decode", message, details);
}

function invalidAuthority(message, details = {}) {
  return invalidProfile(AUTHORITY_PROFILE, "authority", message, details);
}

function invalidEvents(message, details = {}) {
  return invalidProfile(EVENTS_PROFILE, "events", message, details);
}

function invalidMission(message, details = {}) {
  return invalidProfile(MISSION_PROFILE, "mission", message, details);
}

function invalidAdminGateway(message, details = {}) {
  return invalidProfile(ADMIN_GATEWAY_PROFILE, "admin_gateway", message, details);
}

function invalidWrappers(message, details = {}) {
  return invalidProfile(WRAPPERS_PROFILE, "wrappers", message, details);
}

function invalidSurface(message, details = {}) {
  return invalidProfile(SURFACE_PROFILE, "surface", message, details);
}

function invalidCompatibility(message, details = {}) {
  return invalidProfile(COMPATIBILITY_PROFILE, "compatibility", message, details);
}

function invalidProfile(profile, stage, message, details = {}) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage,
    retry: RetryHint.NEVER,
    source: profile,
    message,
    details: profileErrorDetails(profile, details),
  });
}

function cancelledSDK(message) {
  return new SDKError({
    code: ErrorCode.CANCELLED,
    stage: "sdk",
    retry: RetryHint.NEVER,
    message,
  });
}

function invalidInvocation(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "build",
    retry: RetryHint.NEVER,
    message,
  });
}

function invalidRuntime(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "runtime",
    retry: RetryHint.NEVER,
    message,
  });
}

function invalidDaemonError(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "decode",
    retry: RetryHint.NEVER,
    message,
  });
}
