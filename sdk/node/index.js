import { createHash } from "node:crypto";
import { containsAllZeroPrincipal } from "./runtime-principals.js";
import {
  canonicalResourceSubject,
  isRuntimeOwnerReadSubjectURA,
  isRuntimeStateReadSubjectURA,
  runtimeStateReadSubjectURA as buildRuntimeStateReadSubjectURA,
} from "./runtime-subjects.js";

const INVOCATION_CONTROL_RUNTIME_TOKEN = Symbol("invocation-control-runtime-token");

const ERROR_CODES = new Set([
  "INVALID_ARGUMENT",
  "INVALID_HANDLE",
  "NULL_POINTER",
  "INVALID_UTF8",
  "NOT_INITIALIZED",
  "ALREADY_INIT",
  "RUNTIME_OFFLINE",
  "PERMISSION_DENIED",
  "ADMISSION_DENIED",
  "HTTP_AUTH_DENIED",
  "SIGNATURE_DENIED",
  "POLICY_DENIED",
  "AUTHORITY_DENIED",
  "AUTHORITY_SUBJECT_MISMATCH",
  "ABILITY_NOT_FOUND",
  "ROUTE_UNAVAILABLE",
  "EXECUTION_FAILED",
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
  "CALLER_IDENTITY_UNAVAILABLE",
  "CALLER_SIGNER_UNAVAILABLE",
  "DESCRIPTOR_NOT_FOUND",
  "DESCRIPTOR_OWNER_OFFLINE",
  "DESCRIPTOR_MODE_UNSUPPORTED",
  "DESCRIPTOR_STALE",
  "RUNTIME_ROUTE_UNAVAILABLE",
  "INVOCATION_CANCELLED",
  "INVOCATION_TIMEOUT",
  "TERMINAL_RECEIPT_UNAVAILABLE",
  "RECEIPT_PROOF_FACTS_MISSING",
  "PROVIDER_UNAVAILABLE",
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

export const HEALTH_PROFILE = "health";
export const AUTHORITY_PROFILE = "authority";
export const DELEGATION_METADATA_KEY = "x-runtime-delegation";
export const SESSION_AUTHORITY_METADATA_KEY = "x-runtime-session-authority";
export const MAX_STREAM_BUFFERED_EVENTS = 1024;
export const MAX_BIDI_BUFFERED_FRAMES = 1024;
const RUNTIME_GOVERNANCE_READ_ABILITIES = Object.freeze([
  "meta.list_abilities",
  "runtime.catalog.list",
  "invocation.history.list",
  "invocation.history.get",
  "invocation.history.path",
  "invocation.record.get",
  "invocation.trace.get",
]);
const RUNTIME_ABILITY_DESCRIPTOR_PROVIDER = "ability_descriptor";
const RUNTIME_RECEIPT_HISTORY_PROVIDER = "receipt_history";
const CANONICAL_SESSION_AUTHORITY_ID = /^[A-Za-z0-9.-]+$/u;

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
    const decoded = parseJSON(text, "runtime error");
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
        message: `runtime ABI version ${features.abiVersion} does not match expected ${expected}`,
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
    return this.apiReady;
  }

  ready() {
    return this.runtimeReady;
  }

  toJSON() {
    return {
      api_ready: this.apiReady,
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
    const normalized = normalizeSessionAuthorityPrincipals({
      sessionOwnerUserID: authorityOptionalString(value.session_owner_user_id, "session_owner_user_id") ?? "",
      sessionOwnerURA: authorityOptionalString(value.session_owner_ura, "session_owner_ura") ?? "",
      creatorPrincipalID: authorityOptionalString(value.creator_principal_id, "creator_principal_id") ?? "",
      creatorPrincipalURA: authorityOptionalString(value.creator_principal_ura, "creator_principal_ura") ?? "",
      subjectURA: requiredAuthorityString(value.subject_ura, "subject_ura"),
    });
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.sessionID = requiredAuthorityString(value.session_id, "session_id");
    this.sessionOwnerUserID = normalized.sessionOwnerUserID;
    this.sessionOwnerURA = normalized.sessionOwnerURA;
    this.creatorPrincipalID = normalized.creatorPrincipalID;
    this.creatorPrincipalURA = normalized.creatorPrincipalURA;
    this.calleeURA = requiredAuthorityString(value.callee_ura, "callee_ura");
    this.subjectURA = normalized.subjectURA;
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
      ...(this.sessionOwnerURA ? { session_owner_ura: this.sessionOwnerURA } : {}),
      creator_principal_id: this.creatorPrincipalID,
      ...(this.creatorPrincipalURA ? { creator_principal_ura: this.creatorPrincipalURA } : {}),
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
    const normalized = normalizeSessionAuthorityPrincipals({
      sessionOwnerUserID: authorityOptionalString(value.session_owner_user_id, "session_owner_user_id") ?? "",
      sessionOwnerURA: authorityOptionalString(value.session_owner_ura, "session_owner_ura") ?? "",
      creatorPrincipalID: authorityOptionalString(value.creator_principal_id, "creator_principal_id") ?? "",
      creatorPrincipalURA: authorityOptionalString(value.creator_principal_ura, "creator_principal_ura") ?? "",
      subjectURA: requiredAuthorityString(value.subject_ura, "subject_ura"),
    });
    this.issuerURA = requiredAuthorityString(value.issuer_ura, "issuer_ura");
    this.sessionID = requiredAuthorityString(value.session_id, "session_id");
    this.sessionOwnerUserID = normalized.sessionOwnerUserID;
    this.sessionOwnerURA = normalized.sessionOwnerURA;
    this.creatorPrincipalID = normalized.creatorPrincipalID;
    this.creatorPrincipalURA = normalized.creatorPrincipalURA;
    this.calleeURA = requiredAuthorityString(value.callee_ura, "callee_ura");
    this.subjectURA = normalized.subjectURA;
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

export class InvocationDraft {
  constructor(fields) {
    this.callerURA = requiredBuilderPrincipalString(fields.callerURA, "caller_ura");
    this.calleeURA = requiredBuilderPrincipalString(fields.calleeURA, "callee_ura");
    this.descriptorRef = requiredBuilderString(fields.descriptorRef, "descriptor_ref");
    this.subjectURA = requiredBuilderPrincipalString(fields.subjectURA, "subject_ura");
    this.nonceBase64 = requiredBuilderString(fields.nonceBase64, "nonce_base64");
    this.causalContext = objectValue(fields.causalContext, "causal_context");
    this.contentType = requiredBuilderString(fields.contentType, "content_type");
    this.args = fields.args;
    this.argumentsBase64 = fields.argumentsBase64 || "";
    this.metadata = objectValue(fields.metadata ?? {}, "metadata");
    this.callerSignature = fields.callerSignature ?? null;
    this.hasArgs = Boolean(fields.hasArgs);
    this.governanceRead = fields.governanceRead === true;
    if (!this.governanceRead) {
      rejectGovernanceReadPublicInvocationDescriptor(this.calleeURA, this.descriptorRef);
    }
    validateAuthorityMetadata(this.metadata);
    validateInvocationAuthorityBinding(this);
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

function immutableRuntimeReceiptProjection(raw, field) {
  const value = objectValue(raw, field);
  for (const [key, item] of Object.entries(value)) {
    value[key] = immutableRuntimeReceiptValue(item, `${field}.${key}`);
  }
  return Object.freeze(value);
}

function immutableRuntimeReceiptValue(value, field) {
  if (Array.isArray(value)) {
    return Object.freeze(
      value.map((item, index) => immutableRuntimeReceiptValue(item, `${field}[${index}]`)),
    );
  }
  if (value && typeof value === "object") {
    return immutableRuntimeReceiptProjection(value, field);
  }
  return value;
}

function mutableRuntimeReceiptProjection(raw) {
  if (Array.isArray(raw)) {
    return raw.map((item) => mutableRuntimeReceiptProjection(item));
  }
  if (raw && typeof raw === "object") {
    const value = {};
    for (const [key, item] of Object.entries(raw)) {
      value[key] = mutableRuntimeReceiptProjection(item);
    }
    return value;
  }
  return raw;
}

export class RuntimeReceipt {
  constructor(raw) {
    this.raw = immutableRuntimeReceiptProjection(raw, "runtime receipt");
    this.invocationId = requiredRuntimeString(this.raw.invocation_id, "invocation_id");
    this.receiptType = requiredRuntimeString(this.raw.receipt_type, "receipt_type");
    this.state = requiredRuntimeString(this.raw.state, "state");
    this.validateSummary();
  }

  static fromObject(raw) {
    return new RuntimeReceipt(raw);
  }

  lifecycleState() {
    return canonicalRuntimeReceiptState(this.state);
  }

  rawProjection() {
    return mutableRuntimeReceiptProjection(this.raw);
  }

  validateSummary() {
    const lifecycleState = canonicalRuntimeReceiptState(this.state);
    if (lifecycleState === "UNSPECIFIED") {
      throw invalidRuntime("runtime receipt lifecycle state must not be UNSPECIFIED");
    }
    if (this.receiptType !== canonicalRuntimeReceiptType(lifecycleState)) {
      throw invalidRuntime("runtime receipt receipt_type does not match its lifecycle state");
    }
    runtimeReceiptHash(this.raw.prev_receipt_hash_hex, "prev_receipt_hash_hex", true);
    runtimeReceiptHash(this.raw.self_hash_hex, "self_hash_hex", false);
    validateRuntimeReceiptProofFacts(this.raw);
  }
}

export class RuntimeCallContext {
  constructor(fields) {
    const value = objectValue(fields, "runtime call context");
    rejectRuntimeFields(value, [
      "caller_ura",
      "callee_ura",
      "subject_ura",
      "nonce_base64",
      "causal_context",
      "metadata",
      "authority",
    ]);
    this.callerURA = requiredHistoryPrincipalString(value.caller_ura, "caller_ura");
    this.calleeURA = requiredHistoryPrincipalString(value.callee_ura, "callee_ura");
    this.subjectURA = requiredHistoryPrincipalString(value.subject_ura, "subject_ura");
    this.nonceBase64 = optionalRuntimeString(value.nonce_base64, "nonce_base64") ?? "";
    if (this.nonceBase64) {
      validateRuntimeBase64(this.nonceBase64, "nonce_base64", 16);
    }
    this.causalContext = value.causal_context === undefined || value.causal_context === null
      ? null
      : objectValue(value.causal_context, "causal_context");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
    this.authority = normalizeRuntimeCallAuthority(value.authority ?? null);
  }

  toJSON() {
    const value = {
      caller_ura: this.callerURA,
      callee_ura: this.calleeURA,
      subject_ura: this.subjectURA,
      metadata: { ...this.metadata },
    };
    if (this.nonceBase64) {
      value.nonce_base64 = this.nonceBase64;
    }
    if (this.causalContext !== null) {
      value.causal_context = { ...this.causalContext };
    }
    return value;
  }
}

export function receiptReadCallContext(fields) {
  const value = objectValue(fields, "receipt read call context");
  rejectRuntimeFields(value, [
    "caller_ura",
    "callee_ura",
    "nonce_base64",
    "causal_context",
    "metadata",
    "authority",
  ]);
  const callerURA = requiredHistoryPrincipalString(value.caller_ura, "caller_ura");
  const calleeURA = requiredHistoryPrincipalString(value.callee_ura, "callee_ura");
  const authority = normalizeRuntimeCallAuthority(value.authority ?? null);
  if (authority === null) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "receipt read call context requires typed runtime authority",
      { caller_ura: callerURA, callee_ura: calleeURA },
    );
  }
  return new RuntimeCallContext({
    caller_ura: callerURA,
    callee_ura: calleeURA,
    subject_ura: receiptReadSubjectURA(calleeURA, authority),
    nonce_base64: value.nonce_base64,
    causal_context: value.causal_context,
    metadata: objectValue(value.metadata ?? {}, "metadata"),
    authority,
  });
}

export class ReceiptFilter {
  constructor(fields = {}) {
    const value = objectValue(fields, "receipt filter");
    rejectRuntimeFields(value, [
      "caller_ura",
      "callee_ura",
      "subject_ura",
      "ability_ura",
      "state",
    ]);
    this.callerURA = optionalHistoryPrincipalString(value.caller_ura, "filter.caller_ura");
    this.calleeURA = optionalHistoryPrincipalString(value.callee_ura, "filter.callee_ura");
    this.subjectURA = optionalHistoryPrincipalString(value.subject_ura, "filter.subject_ura");
    this.abilityURA = optionalRuntimeString(value.ability_ura, "filter.ability_ura") ?? "";
    this.state = optionalRuntimeString(value.state, "filter.state") ?? "";
  }

  toJSON() {
    const value = {};
    if (this.callerURA) value.caller_ura = this.callerURA;
    if (this.calleeURA) value.callee_ura = this.calleeURA;
    if (this.subjectURA) value.subject_ura = this.subjectURA;
    if (this.abilityURA) value.ability_ura = this.abilityURA;
    if (this.state) value.state = this.state;
    return value;
  }
}

export class ReceiptListRequest {
  constructor(fields) {
    const value = objectValue(fields, "receipt list request");
    rejectRuntimeFields(value, ["call", "filter", "limit", "cursor"]);
    this.call = value.call instanceof RuntimeCallContext
      ? value.call
      : new RuntimeCallContext(objectValue(value.call, "call"));
    this.filter = value.filter === undefined || value.filter === null
      ? null
      : (value.filter instanceof ReceiptFilter ? value.filter : new ReceiptFilter(value.filter));
    this.limit = boundedRuntimeLimit(value.limit, "limit", 50);
    this.cursor = optionalRuntimeString(value.cursor, "cursor") ?? "";
  }

  toJSON() {
    return {
      call: this.call.toJSON(),
      filter: this.filter ? this.filter.toJSON() : {},
      limit: this.limit,
      cursor: this.cursor,
    };
  }
}

export class ReceiptHistoryPage {
  constructor(fields) {
    const value = objectValue(fields, "receipt history page");
    rejectRuntimeFields(value, ["records", "next_cursor", "limit", "source"]);
    if (!Array.isArray(value.records)) {
      throw invalidHistory("records must be an array");
    }
    this.records = value.records.map((record, index) => objectValue(record, `records[${index}]`));
    this.nextCursor = optionalRuntimeString(value.next_cursor, "next_cursor") ?? "";
    this.limit = boundedRuntimeLimit(value.limit, "limit", 50);
    this.source = requiredRuntimeString(value.source, "source");
  }

  static fromJSON(raw) {
    return new ReceiptHistoryPage(parseJSON(raw, "receipt history page"));
  }

  toJSON() {
    return {
      records: this.records.map((record) => ({ ...record })),
      next_cursor: this.nextCursor,
      limit: this.limit,
      source: this.source,
    };
  }
}

export class SessionHistoryOperations {
  constructor(receipts) {
    if (!receipts || typeof receipts.list !== "function") {
      throw invalidHistory("receipt provider is required");
    }
    this.receipts = receipts;
  }

  async list(request) {
    const payload = request instanceof ReceiptListRequest
      ? request
      : new ReceiptListRequest(request);
    validateSessionHistoryRequest(payload);
    const result = await this.receipts.list(payload);
    if (result instanceof ReceiptHistoryPage) {
      return result;
    }
    return result instanceof Uint8Array || Buffer.isBuffer(result) || typeof result === "string"
      ? ReceiptHistoryPage.fromJSON(result)
      : new ReceiptHistoryPage(result);
  }
}

export class RuntimeReceiptProvider {
  constructor(ability) {
    if (!(ability instanceof RuntimeAbilityClient)) {
      throw invalidHistory("runtime ability client is required");
    }
    this.ability = ability;
  }

  receiptHistoryListAuthorityScope() {
    return RUNTIME_RECEIPT_HISTORY_PROVIDER === "receipt_history"
      ? "invocation.history.list"
      : RUNTIME_RECEIPT_HISTORY_PROVIDER;
  }

  async list(request) {
    const payload = request instanceof ReceiptListRequest
      ? request
      : new ReceiptListRequest(request);
    validateSessionHistoryRequest(payload);
    const args = receiptListArguments(payload);
    const output = await this.ability.invokeGovernanceRead(
      payload.call,
      "invocation.history.list",
      args,
      RUNTIME_RECEIPT_HISTORY_PROVIDER,
    );
    return new ReceiptHistoryPage({
      records: historyOutputRecords(output),
      next_cursor: optionalRuntimeString(output.next_cursor, "next_cursor") ?? "",
      limit: payload.limit,
      source: receiptLedgerSource(output),
    });
  }
}

export class AbilityDescriptorProjection {
  constructor(fields = {}) {
    const value = objectValue(fields, "ability descriptor");
    this.abilityURA = requiredRuntimeText(value.ability_ura, "ability_ura");
    this.descriptorRef = requiredRuntimeText(value.descriptor_ref, "descriptor_ref");
    this.name = requiredRuntimeText(value.name, "name");
    this.ownerURA = requiredRuntimeText(value.owner_ura, "owner_ura");
    this.version = requiredRuntimeText(value.version, "version");
    this.schemaHash = optionalRuntimeString(value.schema_hash, "schema_hash") ?? "";
    this.descriptorHash = optionalRuntimeString(value.descriptor_hash, "descriptor_hash") ?? "";
    this.callMode = optionalRuntimeString(value.call_mode, "call_mode") ?? "";
    this.className = optionalRuntimeString(value.class, "class") ?? "";
    this.receiptSemantics = objectValue(value.receipt_semantics ?? {}, "receipt_semantics");
    this.visibility = optionalRuntimeString(value.visibility, "visibility") ?? "";
    this.source = optionalRuntimeString(value.source, "source") ?? "";
    this.description = optionalRuntimeString(value.description, "description") ?? "";
    this.hints = objectValue(value.hints ?? {}, "hints");
    this.schemaSummary = objectValue(value.schema_summary ?? {}, "schema_summary");
    this.inputSchema = objectValue(value.input_schema ?? {}, "input_schema");
    this.metadata = objectValue(value.metadata ?? {}, "metadata");
    validateAbilityDescriptorProjection(this);
    Object.freeze(this);
  }

  toJSON() {
    return {
      ability_ura: this.abilityURA,
      descriptor_ref: this.descriptorRef,
      name: this.name,
      owner_ura: this.ownerURA,
      version: this.version,
      schema_hash: this.schemaHash,
      descriptor_hash: this.descriptorHash,
      call_mode: this.callMode,
      class: this.className,
      receipt_semantics: { ...this.receiptSemantics },
      visibility: this.visibility,
      source: this.source,
      description: this.description,
      hints: { ...this.hints },
      schema_summary: { ...this.schemaSummary },
      input_schema: { ...this.inputSchema },
      metadata: { ...this.metadata },
    };
  }
}

export class AbilityDescriptorListRequest {
  constructor(fields) {
    const value = objectValue(fields, "ability descriptor list request");
    rejectRuntimeFields(value, ["call", "scope", "owner_ura", "ability_ura"]);
    this.call = value.call instanceof RuntimeCallContext
      ? value.call
      : new RuntimeCallContext(objectValue(value.call, "call"));
    this.scope = optionalRuntimeString(value.scope, "scope") ?? "";
    this.ownerURA = optionalRuntimeString(value.owner_ura, "owner_ura") ?? "";
    this.abilityURA = optionalRuntimeString(value.ability_ura, "ability_ura") ?? "";
    Object.freeze(this);
  }

  toJSON() {
    const value = { call: this.call.toJSON() };
    if (this.scope) value.scope = this.scope;
    if (this.ownerURA) value.owner_ura = this.ownerURA;
    if (this.abilityURA) value.ability_ura = this.abilityURA;
    return value;
  }
}

export class AbilityDescriptorGetRequest {
  constructor(fields) {
    const value = objectValue(fields, "ability descriptor get request");
    rejectRuntimeFields(value, [
      "call",
      "ability_ura",
      "descriptor_version",
      "call_mode",
      "scope",
    ]);
    this.call = value.call instanceof RuntimeCallContext
      ? value.call
      : new RuntimeCallContext(objectValue(value.call, "call"));
    this.abilityURA = requiredRuntimeText(value.ability_ura, "ability_ura");
    this.descriptorVersion = optionalRuntimeString(value.descriptor_version, "descriptor_version") ?? "";
    this.callMode = optionalRuntimeString(value.call_mode, "call_mode") ?? "";
    this.scope = optionalRuntimeString(value.scope, "scope") ?? "";
    Object.freeze(this);
  }
}

export class AbilityDescriptorPage {
  constructor(fields) {
    const value = objectValue(fields, "ability descriptor page");
    const rawDescriptors = value.descriptors ?? [];
    if (!Array.isArray(rawDescriptors)) {
      throw invalidRuntime("ability descriptor page descriptors must be an array");
    }
    this.descriptors = rawDescriptors.map((descriptor) =>
      descriptor instanceof AbilityDescriptorProjection
        ? descriptor
        : new AbilityDescriptorProjection(descriptor),
    );
    Object.freeze(this.descriptors);
    Object.freeze(this);
  }
}

export class AbilityDescriptorClient {
  constructor(provider) {
    if (!provider || typeof provider.list !== "function" || typeof provider.get !== "function") {
      throw invalidRuntime("AbilityDescriptor provider is required");
    }
    this.provider = provider;
  }

  async list(request) {
    const result = await this.provider.list(
      request instanceof AbilityDescriptorListRequest
        ? request
        : new AbilityDescriptorListRequest(request),
    );
    return result instanceof AbilityDescriptorPage
      ? result
      : new AbilityDescriptorPage(result);
  }

  async get(request) {
    const result = await this.provider.get(
      request instanceof AbilityDescriptorGetRequest
        ? request
        : new AbilityDescriptorGetRequest(request),
    );
    return result instanceof AbilityDescriptorProjection
      ? result
      : new AbilityDescriptorProjection(result);
  }
}

export class RuntimeAbilityDescriptorProvider {
  constructor(ability) {
    if (!(ability instanceof RuntimeAbilityClient)) {
      throw invalidRuntime("runtime ability client is required");
    }
    this.ability = ability;
  }

  async list(request) {
    const payload = request instanceof AbilityDescriptorListRequest
      ? request
      : new AbilityDescriptorListRequest(request);
    const args = {};
    if (payload.scope) args.scope = payload.scope;
    if (payload.ownerURA) args.owner_ura = payload.ownerURA;
    if (payload.abilityURA) args.ability_ura = payload.abilityURA;
    const output = await this.ability.invokeGovernanceRead(
      payload.call,
      "meta.list_abilities",
      args,
      RUNTIME_ABILITY_DESCRIPTOR_PROVIDER,
    );
    const rows = output.abilities;
    if (!Array.isArray(rows)) {
      throw invalidRuntime("runtime descriptor catalog output must include descriptor rows");
    }
    return new AbilityDescriptorPage({
      descriptors: rows.map((row, index) => {
        const descriptor = new AbilityDescriptorProjection(
          objectValue(row, `ability descriptor row ${index}`),
        );
        if (payload.abilityURA && descriptor.abilityURA !== payload.abilityURA) {
          throw invalidRuntime("runtime returned descriptor outside requested ability_ura");
        }
        return descriptor;
      }),
    });
  }

  async get(request) {
    const payload = request instanceof AbilityDescriptorGetRequest
      ? request
      : new AbilityDescriptorGetRequest(request);
    const page = await this.list(new AbilityDescriptorListRequest({
      call: payload.call,
      scope: payload.scope,
      ability_ura: payload.abilityURA,
    }));
    const matches = page.descriptors.filter((descriptor) => {
      if (descriptor.abilityURA !== payload.abilityURA) return false;
      if (payload.descriptorVersion && descriptor.version !== payload.descriptorVersion) {
        return false;
      }
      if (payload.callMode && descriptor.callMode !== payload.callMode) {
        return false;
      }
      return true;
    });
    if (matches.length === 1) {
      return matches[0];
    }
    if (matches.length === 0) {
      throw invalidRuntime("ability descriptor not found");
    }
    throw invalidRuntime("ability descriptor query is ambiguous");
  }
}

export class InvocationBuilder {
  constructor() {
    this.fields = { metadata: {}, governanceRead: false };
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

  withRuntimeGovernanceRead() {
    this.fields.governanceRead = true;
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
      governanceRead: this.fields.governanceRead === true,
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
    return invocationResultFromJSON(raw);
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

  async resolveDescriptorRef(request) {
    const transport = this.requireOpen();
    if (typeof transport.resolveDescriptorRef !== "function") {
      throw invalidSDK("runtime descriptor resolver transport function is required");
    }
    const raw = await transport.resolveDescriptorRef(
      Buffer.from(JSON.stringify(runtimeDescriptorRefRequest(request))),
    );
    return runtimeDescriptorRefResponse(raw);
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
    return invocationHandleFromRuntimeJSON(raw).bindRuntime(this);
  }

  async awaitResult(handle) {
    const transport = this.requireOpen();
    if (typeof transport.awaitHandle !== "function") {
      throw invalidSDK("runtime await-handle transport function is required");
    }
    const control = runtimeControlCapability(handle);
    return invocationResultFromJSON(await transport.awaitHandle(control));
  }

  async cancel(handle, reason = "") {
    const transport = this.requireOpen();
    if (typeof transport.cancelHandle !== "function") {
      throw invalidSDK("runtime cancel-handle transport function is required");
    }
    if (typeof reason !== "string") {
      throw invalidRuntime("cancel reason must be a string");
    }
    const control = runtimeControlCapability(handle);
    return invocationCancelFromJSONWithControl(await transport.cancelHandle(control, reason), control);
  }

  async events(handle) {
    const transport = this.requireOpen();
    if (typeof transport.handleEvents !== "function") {
      throw invalidSDK("runtime handle-events transport function is required");
    }
    const control = runtimeControlCapability(handle);
    return invocationHandleFromJSONWithControl(await transport.handleEvents(control), control).bindRuntime(this);
  }

  async closeHandle(handle) {
    const transport = this.requireOpen();
    if (typeof transport.freeHandle !== "function") {
      throw invalidSDK("runtime free-handle transport function is required");
    }
    await transport.freeHandle(runtimeControlCapability(handle));
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
    const normalizedStreams = assertBidiOpenStreams(streams);
    const result = await transport.openBidi(
      Buffer.from(assertDraft(draft).toJSONString()),
      Buffer.from(JSON.stringify(normalizedStreams)),
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

export class RuntimeAbilityClient {
  constructor(runtime) {
    if (!(runtime instanceof RuntimeClient)) {
      throw invalidSDK("runtime client is required");
    }
    this.runtime = runtime;
  }

  async build(call, abilityName, argumentsValue, options = {}) {
    const policy = runtimeAbilityDispatchPolicy(options);
    return this.buildWithPolicy(call, abilityName, argumentsValue, "rpc", policy);
  }

  async invoke(call, abilityName, argumentsValue) {
    const result = await this.runtime.invoke(await this.build(call, abilityName, argumentsValue));
    return runtimeAbilityObjectOutput(result);
  }

  async buildGovernanceRead(call, abilityName, argumentsValue, provider = "") {
    const selectedProvider = runtimeGovernanceDescriptorProviderForRequest(abilityName, provider);
    return this.buildWithPolicy(
      call,
      abilityName,
      argumentsValue,
      "rpc",
      runtimeAbilityGovernanceReadPolicy(selectedProvider),
    );
  }

  async invokeGovernanceRead(call, abilityName, argumentsValue, provider = "") {
    const result = await this.runtime.invoke(
      await this.buildGovernanceRead(call, abilityName, argumentsValue, provider),
    );
    return runtimeAbilityObjectOutput(result);
  }

  async buildWithPolicy(call, abilityName, argumentsValue, callMode, policy) {
    const context = call instanceof RuntimeCallContext ? call : new RuntimeCallContext(call);
    validateRuntimeAbilityCallContext(context);
    const ability = requiredRuntimeText(abilityName, "ability name");
    if (!policy.allowGovernanceRead && runtimeGovernanceReadAbility(ability)) {
      throw invalidInvocation(
        "runtime governance receipt/history/catalogue abilities must use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider",
      );
    }
    const subjectURA = runtimeAbilitySubjectURA(context, policy);
    const descriptorRef = await this.runtime.resolveDescriptorRef({
      callee_ura: context.calleeURA,
      ability,
      call_mode: requiredRuntimeText(callMode, "call_mode"),
      caller_ura: context.callerURA,
      subject_ura: runtimeAbilityDescriptorResolutionSubjectURA(context, subjectURA, policy),
      provider: policy.descriptorProvider,
    });
    const projection = RuntimeAbilityProjection.fromDescriptorRef(context.calleeURA, descriptorRef);
    const metadata = runtimeAbilityMetadata(context, projection, subjectURA);
    const builder = new InvocationBuilder()
      .withCallerURA(context.callerURA)
      .withCalleeURA(context.calleeURA)
      .withDescriptorRef(descriptorRef)
      .withSubjectURA(subjectURA)
      .withNonceBase64(context.nonceBase64)
      .withCausalContext(context.causalContext)
      .withJSONArgs(argumentsValue)
      .withContentType("application/json")
      .withMetadata(metadata);
    if (policy.allowGovernanceRead) {
      builder.withRuntimeGovernanceRead();
    }
    return builder.build();
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
    if (this.mode.trim() === "provider_managed_signing") {
      if (this.signerId.trim() === "") {
        throw invalidRuntime("provider-managed signer_policy requires signer_id");
      }
      if (this.policyRef.trim() === "") {
        throw invalidRuntime("provider-managed signer_policy requires policy_ref");
      }
    }
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
    if (!this.preparedId) {
      throw invalidRuntime("prepared_id is required");
    }
    this.descriptorRef = requiredRuntimeString(value.descriptor_ref, "descriptor_ref");
    if (this.descriptorRef !== this.signingMaterial.descriptorRef) {
      throw invalidRuntime("signing_material.descriptor_ref must match tuple descriptor_ref");
    }
    this.descriptorHashHex = optionalRuntimeString(value.descriptor_hash_hex, "descriptor_hash_hex") ?? "";
    this.schemaHashHex = optionalRuntimeString(value.schema_hash_hex, "schema_hash_hex") ?? "";
    this.canonicalHashHex = optionalRuntimeString(value.canonical_hash_hex, "canonical_hash_hex") ?? "";
    validatePreparedHash(this.signingMaterial.canonicalBytesBase64, this.canonicalHashHex);
    this.expiresAtUnixMS = positiveRuntimeInteger(value.expires_at_unix_ms, "expires_at_unix_ms");
    if (this.expiresAtUnixMS !== this.signingMaterial.expiresAtUnixMS) {
      throw invalidRuntime("expires_at_unix_ms must match signing_material.expires_at_unix_ms");
    }
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

class InvocationControlCapability {
  #handleId;
  #runtimeBound;

  constructor(fields) {
    const value = objectValue(fields, "invocation control capability");
    rejectRuntimeFields(value, ["handle_id", "runtime_bound"]);
    this.#handleId = positiveRuntimeInteger(value.handle_id, "handle_id");
    this.#runtimeBound =
      value.runtime_bound === true && value[INVOCATION_CONTROL_RUNTIME_TOKEN] === true;
  }

  static fromHandleId(handleId) {
    return InvocationControlCapability.fromSnapshotHandleId(handleId);
  }

  static fromSnapshotHandleId(handleId) {
    return new InvocationControlCapability({ handle_id: handleId, runtime_bound: false });
  }

  _adapterHandleId() {
    if (!this.#runtimeBound) {
      throw invalidRuntime("runtime-bound invocation control capability is required");
    }
    return this.#handleId;
  }

  _rawHandleId() {
    return this.#handleId;
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
    this.controlCapability = InvocationControlCapability.fromSnapshotHandleId(
      positiveRuntimeInteger(value.handle_id, "handle_id"),
    );
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
      handle_id: this.controlCapability._rawHandleId(),
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
    rejectRuntimeFields(value, [
      "handle_id",
      "request_accepted",
      "deduplicated",
      "cancelled",
      "state",
      "terminal",
    ]);
    this.controlCapability = InvocationControlCapability.fromSnapshotHandleId(
      positiveRuntimeInteger(value.handle_id, "handle_id"),
    );
    this.requestAccepted = runtimeBoolean(value.request_accepted, "request_accepted");
    this.deduplicated = runtimeBoolean(value.deduplicated, "deduplicated");
    this.cancelled = runtimeBoolean(value.cancelled, "cancelled");
    this.state = requiredRuntimeString(value.state, "state");
    this.terminal = runtimeBoolean(value.terminal, "terminal");
  }

  static fromJSON(raw) {
    return new InvocationCancel(parseJSON(raw, "invocation cancel"));
  }

  toJSON() {
    return {
      handle_id: this.controlCapability._rawHandleId(),
      request_accepted: this.requestAccepted,
      deduplicated: this.deduplicated,
      cancelled: this.cancelled,
      state: this.state,
      terminal: this.terminal,
    };
  }
}

function invocationHandleFromRuntimeJSON(raw) {
  const handle = new InvocationHandle(parseJSON(raw, "invocation handle"));
  handle.controlCapability = runtimeControlCapabilityFromHandleId(handle.controlCapability._rawHandleId());
  return handle;
}

function invocationHandleFromJSONWithControl(raw, control) {
  const handle = new InvocationHandle(parseJSON(raw, "invocation handle"));
  if (handle.controlCapability._rawHandleId() !== control._adapterHandleId()) {
    throw invalidRuntime("handle_id does not match invocation control capability");
  }
  handle.controlCapability = control;
  return handle;
}

function invocationCancelFromJSONWithControl(raw, control) {
  const cancel = new InvocationCancel(parseJSON(raw, "invocation cancel"));
  if (cancel.controlCapability._rawHandleId() !== control._adapterHandleId()) {
    throw invalidRuntime("handle_id does not match invocation control capability");
  }
  cancel.controlCapability = control;
  return cancel;
}

function runtimeControlCapabilityFromHandleId(handleId) {
  return new InvocationControlCapability({
    handle_id: handleId,
    runtime_bound: true,
    [INVOCATION_CONTROL_RUNTIME_TOKEN]: true,
  });
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

function assertDraft(value) {
  if (!(value instanceof InvocationDraft)) {
    throw invalidInvocation("invocation draft is required");
  }
  return value;
}

function assertBidiOpenStreams(value) {
  if (!Array.isArray(value)) {
    throw invalidRuntime("bidi_streams must be an array");
  }
  if (value.length === 0) {
    throw invalidRuntime("bidi_streams must not be empty");
  }
  return value.map((stream, index) => objectValue(stream, `bidi_streams[${index}]`));
}

function assertHandle(value) {
  if (!(value instanceof InvocationHandle)) {
    throw invalidRuntime("invocation handle is required");
  }
  return value;
}

function runtimeControlCapability(value) {
  const control = assertHandle(value).controlCapability;
  control._adapterHandleId();
  return control;
}

function requireBoundRuntime(runtime) {
  if (!runtime || typeof runtime.awaitResult !== "function") {
    throw invalidRuntime("invocation handle is not bound to a runtime client");
  }
  return runtime;
}

function invocationResultFromJSON(raw) {
  const decoded = parseJSON(raw, "invocation result");
  if (Object.hasOwn(decoded, "receipt")) {
    throw invalidRuntime("invocation result must use terminal_receipt; retired receipt alias is not accepted");
  }
  const result = { ...decoded };
  if (decoded.terminal_receipt === undefined || decoded.terminal_receipt === null) {
    delete result.terminalReceipt;
  } else {
    result.terminalReceipt = validatedTerminalReceipt(
      decoded.terminal_receipt,
      decoded.terminal_state,
      decoded.ok,
    );
  }
  delete result.terminal_receipt;
  return result;
}

function validatedTerminalReceipt(value, terminalState, ok) {
  const receipt = RuntimeReceipt.fromObject(objectValue(value, "terminal_receipt"));
  const receiptState = receipt.lifecycleState();
  const resultState = canonicalRuntimeReceiptState(requiredRuntimeString(terminalState, "terminal_state"));
  const resultOk = runtimeBoolean(ok, "ok");
  if (receiptState !== resultState) {
    throw invalidRuntime("terminal_receipt state does not match invocation terminal_state");
  }
  if (resultOk !== (receiptState === "COMPLETED")) {
    throw invalidRuntime("invocation result ok flag does not match terminal receipt state");
  }
  return receipt.rawProjection();
}

function validateRuntimeReceiptProofFacts(raw) {
  requireRuntimeReceiptExactKeys(raw, "runtime_receipt", [
    "receipt_ura",
    "invocation_id",
    "receipt_type",
    "state",
    "index",
    "timestamp_unix_ms",
    "prev_receipt_hash_hex",
    "self_hash_hex",
    "payload_base64",
    "payload_content_type",
    "cleanup_complete",
    "caller_binding",
    "callee_binding",
    "subject_binding",
    "invocation_nonce_base64",
    "causal_binding_kind",
    "causal_binding",
    "callee_signature",
    "signer_binding",
    "host_attestation_base64",
    "authority_binding_kind",
    "authority_binding",
    "ability_binding",
    "usage",
    "subject_ref",
    "descriptor_version",
    "schema_hash_hex",
    "impl_hash_hex",
    "runtime_env",
    "authority_proof",
    "input_hash_hex",
    "output_hash_hex",
    "parent_receipts",
  ]);
  requireRuntimeReceiptRequiredKeys(raw, "runtime_receipt", [
    "receipt_ura",
    "invocation_id",
    "receipt_type",
    "state",
    "index",
    "timestamp_unix_ms",
    "prev_receipt_hash_hex",
    "self_hash_hex",
    "payload_base64",
    "payload_content_type",
    "cleanup_complete",
    "caller_binding",
    "callee_binding",
    "subject_binding",
    "invocation_nonce_base64",
    "causal_binding_kind",
    "causal_binding",
    "callee_signature",
    "signer_binding",
    "host_attestation_base64",
    "authority_binding_kind",
    "authority_binding",
    "ability_binding",
    "usage",
    "subject_ref",
    "descriptor_version",
    "schema_hash_hex",
    "impl_hash_hex",
    "runtime_env",
    "authority_proof",
    "input_hash_hex",
    "output_hash_hex",
    "parent_receipts",
  ]);
  validateRuntimeBase64(
    requiredRuntimeStringAllowEmpty(raw.payload_base64, "payload_base64"),
    "payload_base64",
    null,
    true,
  );
  requiredRuntimeString(raw.payload_content_type, "payload_content_type");
  requiredRuntimeStringAllowEmpty(raw.host_attestation_base64, "host_attestation_base64");
  requireRuntimeReceiptUsage(raw.usage);
  requireRuntimeReceiptAgentBinding(raw.caller_binding, "caller_binding");
  const calleeBinding = requireRuntimeReceiptAgentBinding(raw.callee_binding, "callee_binding");
  requireRuntimeReceiptAgentBinding(raw.subject_binding, "subject_binding");
  validateRuntimeBase64(
    requiredRuntimeString(raw.invocation_nonce_base64, "invocation_nonce_base64"),
    "invocation_nonce_base64",
    16,
    false,
  );
  const causalKind = requiredRuntimeString(raw.causal_binding_kind, "causal_binding_kind");
  validateRuntimeReceiptCausalBinding(causalKind, objectValue(raw.causal_binding, "causal_binding"));
  requireRuntimeReceiptSignature(raw.callee_signature, "callee_signature");
  const signerBinding = requireRuntimeReceiptAgentBinding(raw.signer_binding, "signer_binding");
  validateRuntimeReceiptSigningModel(calleeBinding, signerBinding, stringValue(raw.host_attestation_base64, "host_attestation_base64", true));
  const authorityKind = requiredRuntimeString(raw.authority_binding_kind, "authority_binding_kind");
  const authorityBinding = requireRuntimeReceiptAuthorityBinding(raw.authority_binding, "authority_binding");
  if (authorityBinding.kind !== authorityKind) {
    throw invalidRuntime("runtime receipt authority_binding kind does not match authority_binding_kind");
  }
  requiredRuntimeString(raw.ability_binding, "ability_binding");
  requireRuntimeReceiptEntityRef(raw.subject_ref, "subject_ref");
  requiredRuntimeString(raw.descriptor_version, "descriptor_version");
  runtimeReceiptHash(raw.schema_hash_hex, "schema_hash_hex", false);
  runtimeReceiptHash(raw.impl_hash_hex, "impl_hash_hex", false);
  requiredRuntimeString(raw.runtime_env, "runtime_env");
  const proof = objectValue(raw.authority_proof, "authority_proof");
  requireRuntimeReceiptExactKeys(proof, "authority_proof", [
    "proof_type",
    "binding_kind",
    "binding",
    "proof_payload_base64",
    "proof_hash_hex",
    "issuer",
    "signature",
    "admission_hook",
  ]);
  requiredRuntimeString(proof.proof_type, "authority_proof.proof_type");
  const proofBindingKind = requiredRuntimeString(proof.binding_kind, "authority_proof.binding_kind");
  if (proofBindingKind !== authorityKind) {
    throw invalidRuntime("runtime receipt authority_proof binding_kind does not match authority_binding_kind");
  }
  const proofBinding = requireRuntimeReceiptAuthorityBinding(proof.binding, "authority_proof.binding");
  if (stableRuntimeObjectJSON(proofBinding) !== stableRuntimeObjectJSON(authorityBinding)) {
    throw invalidRuntime("runtime receipt authority_proof binding does not match authority_binding");
  }
  const proofPayload = validateRuntimeBase64(
    requiredRuntimeStringAllowEmpty(
      proof.proof_payload_base64,
      "authority_proof.proof_payload_base64",
    ),
    "authority_proof.proof_payload_base64",
    null,
    true,
  );
  const proofHash = runtimeReceiptHash(proof.proof_hash_hex, "authority_proof.proof_hash_hex", false);
  validateRuntimeReceiptAuthorityProofHash(proofPayload, proofBinding, proofHash);
  const issuer = requireRuntimeReceiptAgentBinding(proof.issuer, "authority_proof.issuer");
  requireRuntimeReceiptSameIdentity(issuer, calleeBinding);
  requireRuntimeReceiptSignature(proof.signature, "authority_proof.signature");
  requiredRuntimeString(proof.admission_hook, "authority_proof.admission_hook");
  runtimeReceiptHash(raw.input_hash_hex, "input_hash_hex", false);
  runtimeReceiptHash(raw.output_hash_hex, "output_hash_hex", false);
  requireRuntimeReceiptParents(raw.parent_receipts);
}

function validateRuntimeReceiptSigningModel(calleeBinding, signerBinding, hostAttestationBase64) {
  if (signerBinding.ura === calleeBinding.ura) {
    if (hostAttestationBase64 !== "") {
      throw invalidRuntime("self-signed runtime receipt must not carry host_attestation_base64");
    }
    return;
  }
  if (hostAttestationBase64 === "") {
    throw invalidRuntime("hosted runtime receipt is missing host_attestation_base64");
  }
  validateRuntimeBase64(hostAttestationBase64, "host_attestation_base64", 64, false);
}

function validateRuntimeReceiptAuthorityProofHash(proofPayload, proofBinding, proofHash) {
  const expected =
    proofPayload.length > 0
      ? sha256Bytes(proofPayload)
      : sha256Bytes(canonicalRuntimeAuthorityBytes(proofBinding, "authority_proof.binding"));
  if (expected.equals(Buffer.alloc(32)) || proofHash.equals(Buffer.alloc(32)) || !proofHash.equals(expected)) {
    throw invalidRuntime("runtime receipt proof facts are not canonical: authority_proof_hash_mismatch");
  }
}

function canonicalRuntimeReceiptState(value) {
  switch (requiredRuntimeString(value, "state")) {
    case "Accepted":
      return "ACCEPTED";
    case "Admitted":
      return "ADMITTED";
    case "Dispatched":
      return "DISPATCHED";
    case "Running":
      return "RUNNING";
    case "Completed":
      return "COMPLETED";
    case "Failed":
      return "FAILED";
    case "TimedOut":
      return "TIMED_OUT";
    case "Cancelled":
      return "CANCELLED";
    case "Unspecified":
      return "UNSPECIFIED";
    default:
      throw invalidRuntime(`unknown receipt state ${value}`);
  }
}

function canonicalRuntimeReceiptType(state) {
  switch (state) {
    case "ACCEPTED":
      return "accepted";
    case "ADMITTED":
      return "admitted";
    case "DISPATCHED":
      return "dispatched";
    case "RUNNING":
      return "running";
    case "COMPLETED":
      return "completed";
    case "FAILED":
      return "failed";
    case "TIMED_OUT":
      return "timed_out";
    case "CANCELLED":
      return "cancelled";
    default:
      throw invalidRuntime(`unsupported receipt lifecycle state ${state}`);
  }
}

function validateRuntimeReceiptCausalBinding(kind, binding) {
  const form = requiredRuntimeString(binding.form, "causal_binding.form");
  if (form !== kind) {
    throw invalidRuntime("runtime receipt causal_binding form does not match causal_binding_kind");
  }
  if (form === "none") {
    requireRuntimeReceiptExactKeys(binding, "causal_binding", ["form"]);
    return;
  }
  if (form === "scalar") {
    requireRuntimeReceiptExactKeys(binding, "causal_binding", ["form", "receipt"]);
    requireRuntimeReceiptRef(binding.receipt, "causal_binding.receipt");
    return;
  }
  if (form === "list") {
    requireRuntimeReceiptExactKeys(binding, "causal_binding", ["form", "prior"]);
    if (!Array.isArray(binding.prior) || binding.prior.length === 0) {
      throw invalidRuntime("causal_binding.prior must be a non-empty array");
    }
    binding.prior.forEach((receipt, index) => {
      requireRuntimeReceiptRef(receipt, `causal_binding.prior[${index}]`);
    });
    return;
  }
  if (form === "merkle") {
    requireRuntimeReceiptExactKeys(binding, "causal_binding", ["form", "root_hex", "proof_ura"]);
    runtimeReceiptHash(binding.root_hex, "causal_binding.root_hex", false);
    requiredRuntimeString(binding.proof_ura, "causal_binding.proof_ura");
    return;
  }
  throw invalidRuntime(`unsupported causal_binding form ${form}`);
}

function requireRuntimeReceiptRef(value, field) {
  const ref = objectValue(value, field);
  requireRuntimeReceiptExactKeys(ref, field, ["receipt_hash_hex", "receipt_ura"]);
  runtimeReceiptHash(ref.receipt_hash_hex, `${field}.receipt_hash_hex`, false);
  requiredRuntimeString(ref.receipt_ura, `${field}.receipt_ura`);
}

function requireRuntimeReceiptParents(value) {
  if (!Array.isArray(value)) {
    throw invalidRuntime("parent_receipts must be an array");
  }
  value.forEach((receipt, index) => {
    requireRuntimeReceiptRef(receipt, `parent_receipts[${index}]`);
  });
}

function requireRuntimeReceiptAgentBinding(value, field) {
  const binding = objectValue(value, field);
  requireRuntimeReceiptExactKeys(binding, field, ["ura", "profile"]);
  const out = {
    ura: requiredRuntimeText(binding.ura, `${field}.ura`),
    profile: requiredRuntimeText(binding.profile, `${field}.profile`),
  };
  validateRuntimeReceiptURAProfile(out.profile, `${field}.profile`);
  return out;
}

function requireRuntimeReceiptEntityRef(value, field) {
  const ref = objectValue(value, field);
  requireRuntimeReceiptExactKeys(ref, field, ["kind", "ura", "profile"]);
  if (!Number.isInteger(ref.kind) || ref.kind < 1 || ref.kind > 7) {
    throw invalidRuntime(`${field}.kind is not canonical`);
  }
  requiredRuntimeText(ref.ura, `${field}.ura`);
  validateRuntimeReceiptURAProfile(requiredRuntimeText(ref.profile, `${field}.profile`), `${field}.profile`);
}

function requireRuntimeReceiptAuthorityBinding(value, field) {
  const binding = objectValue(value, field);
  const kind = requiredRuntimeText(binding.kind, `${field}.kind`);
  if (kind === "self") {
    requireRuntimeReceiptExactKeys(binding, field, ["kind", "principal_ura"]);
    return { kind, principal_ura: requiredRuntimeText(binding.principal_ura, `${field}.principal_ura`) };
  }
  if (kind === "delegation") {
    requireRuntimeReceiptExactKeys(binding, field, [
      "kind",
      "issuer_ura",
      "subject_ura",
      "caller_ura",
      "audience",
      "scopes",
      "issued_at_ms",
      "expires_at_ms",
      "signature_base64",
    ]);
    return {
      kind,
      issuer_ura: requiredRuntimeText(binding.issuer_ura, `${field}.issuer_ura`),
      subject_ura: requiredRuntimeText(binding.subject_ura, `${field}.subject_ura`),
      caller_ura: requiredRuntimeText(binding.caller_ura, `${field}.caller_ura`),
      audience: requiredRuntimeText(binding.audience, `${field}.audience`),
      scopes: requiredRuntimeStringList(binding.scopes, `${field}.scopes`),
      issued_at_ms: requiredRuntimeNonNegativeInteger(binding.issued_at_ms, `${field}.issued_at_ms`),
      expires_at_ms: requiredRuntimeNonNegativeInteger(binding.expires_at_ms, `${field}.expires_at_ms`),
      signature_base64: requiredRuntimeBase64String(binding.signature_base64, `${field}.signature_base64`, 64),
    };
  }
  if (kind === "capability") {
    requireRuntimeReceiptExactKeys(binding, field, ["kind", "capability_ura"]);
    return { kind, capability_ura: requiredRuntimeText(binding.capability_ura, `${field}.capability_ura`) };
  }
  if (kind === "policy") {
    requireRuntimeReceiptExactKeys(binding, field, ["kind", "policy_ura"]);
    return { kind, policy_ura: requiredRuntimeText(binding.policy_ura, `${field}.policy_ura`) };
  }
  if (kind === "session") {
    requireRuntimeReceiptExactKeys(binding, field, [
      "kind",
      "issuer_ura",
      "subject_ura",
      "session_id",
      "scopes",
      "audiences",
      "issued_at_ms",
      "expires_at_ms",
      "signature_base64",
    ]);
    return {
      kind,
      issuer_ura: requiredRuntimeText(binding.issuer_ura, `${field}.issuer_ura`),
      subject_ura: requiredRuntimeText(binding.subject_ura, `${field}.subject_ura`),
      session_id: requiredRuntimeText(binding.session_id, `${field}.session_id`),
      scopes: requiredRuntimeStringList(binding.scopes, `${field}.scopes`),
      audiences: requiredRuntimeStringList(binding.audiences, `${field}.audiences`),
      issued_at_ms: requiredRuntimeNonNegativeInteger(binding.issued_at_ms, `${field}.issued_at_ms`),
      expires_at_ms: requiredRuntimeNonNegativeInteger(binding.expires_at_ms, `${field}.expires_at_ms`),
      signature_base64: requiredRuntimeBase64String(binding.signature_base64, `${field}.signature_base64`, 64),
    };
  }
  if (kind === "bootstrap") {
    requireRuntimeReceiptExactKeys(binding, field, ["kind", "principal_ura", "realm", "ability"]);
    return {
      kind,
      principal_ura: requiredRuntimeText(binding.principal_ura, `${field}.principal_ura`),
      realm: requiredRuntimeText(binding.realm, `${field}.realm`),
      ability: requiredRuntimeText(binding.ability, `${field}.ability`),
    };
  }
  throw invalidRuntime(`${field}.kind is not canonical: ${kind}`);
}

function requireRuntimeReceiptSignature(value, field) {
  const signature = objectValue(value, field);
  requireRuntimeReceiptExactKeys(signature, field, ["algorithm", "signature_base64", "key_id_hint"]);
  requiredRuntimeString(signature.algorithm, `${field}.algorithm`);
  validateRuntimeBase64(
    requiredRuntimeString(signature.signature_base64, `${field}.signature_base64`),
    `${field}.signature_base64`,
  );
  optionalRuntimeString(signature.key_id_hint, `${field}.key_id_hint`);
}

function requireRuntimeReceiptExactKeys(value, field, allowedKeys) {
  const allowed = new Set(allowedKeys);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw invalidRuntime(`${field} contains noncanonical field ${key}`);
    }
  }
}

function requireRuntimeReceiptRequiredKeys(value, field, requiredKeys) {
  for (const key of requiredKeys) {
    if (!Object.hasOwn(value, key)) {
      throw invalidRuntime(`runtime receipt summary is missing ${field}.${key}`);
    }
  }
}

function requireRuntimeReceiptUsage(value) {
  const usage = objectValue(value, "usage");
  requireRuntimeReceiptExactKeys(usage, "usage", [
    "tokens_in",
    "tokens_out",
    "duration_ms",
    "external_calls",
  ]);
  optionalRuntimeNonNegativeInteger(usage.tokens_in, "usage.tokens_in");
  optionalRuntimeNonNegativeInteger(usage.tokens_out, "usage.tokens_out");
  optionalRuntimeNonNegativeInteger(usage.duration_ms, "usage.duration_ms");
  optionalRuntimeNonNegativeInteger(usage.external_calls, "usage.external_calls");
}

function runtimeReceiptHash(value, field, allowZero) {
  const text = requiredRuntimeString(value, field);
  validateRuntimeHex(text, field, 64);
  if (!allowZero && /^0{64}$/i.test(text)) {
    throw invalidRuntime(`${field} must not be all-zero`);
  }
  return Buffer.from(text, "hex");
}

function canonicalRuntimeAuthorityBytes(binding, field) {
  const chunks = [];
  if (binding.kind === "self") {
    chunks.push(Buffer.from([0x01]), runtimeLengthPrefixedText(binding.principal_ura));
  } else if (binding.kind === "delegation") {
    chunks.push(
      Buffer.from([0x02]),
      runtimeLengthPrefixedText(binding.issuer_ura),
      runtimeLengthPrefixedText(binding.subject_ura),
      runtimeLengthPrefixedText(binding.caller_ura),
      runtimeLengthPrefixedText(binding.audience),
      runtimeU32(binding.scopes.length),
      ...binding.scopes.map(runtimeLengthPrefixedText),
      runtimeI64(binding.issued_at_ms),
      runtimeI64(binding.expires_at_ms),
      runtimeLengthPrefixedBytes(Buffer.from(binding.signature_base64, "base64")),
    );
  } else if (binding.kind === "capability") {
    chunks.push(Buffer.from([0x03]), runtimeLengthPrefixedText(binding.capability_ura));
  } else if (binding.kind === "policy") {
    chunks.push(Buffer.from([0x04]), runtimeLengthPrefixedText(binding.policy_ura));
  } else if (binding.kind === "session") {
    chunks.push(
      Buffer.from([0x05]),
      runtimeLengthPrefixedText(binding.issuer_ura),
      runtimeLengthPrefixedText(binding.subject_ura),
      runtimeLengthPrefixedText(binding.session_id),
      runtimeU32(binding.scopes.length),
      ...binding.scopes.map(runtimeLengthPrefixedText),
      runtimeU32(binding.audiences.length),
      ...binding.audiences.map(runtimeLengthPrefixedText),
      runtimeI64(binding.issued_at_ms),
      runtimeI64(binding.expires_at_ms),
      runtimeLengthPrefixedBytes(Buffer.from(binding.signature_base64, "base64")),
    );
  } else if (binding.kind === "bootstrap") {
    chunks.push(
      Buffer.from([0x06]),
      runtimeLengthPrefixedText(binding.principal_ura),
      runtimeLengthPrefixedText(binding.realm),
      runtimeLengthPrefixedText(binding.ability),
    );
  } else {
    throw invalidRuntime(`${field}.kind is not canonical: ${binding.kind}`);
  }
  return Buffer.concat(chunks);
}

function runtimeLengthPrefixedText(value) {
  return runtimeLengthPrefixedBytes(Buffer.from(value, "utf8"));
}

function runtimeLengthPrefixedBytes(value) {
  return Buffer.concat([runtimeU32(value.length), value]);
}

function runtimeU32(value) {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value);
  return out;
}

function runtimeI64(value) {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt(value));
  return out;
}

function sha256Bytes(value) {
  return createHash("sha256").update(value).digest();
}

function requireRuntimeReceiptSameIdentity(left, right) {
  if (left.ura !== right.ura || left.profile !== right.profile) {
    throw invalidRuntime("runtime receipt authority_proof issuer does not match callee_binding");
  }
}

function validateRuntimeReceiptURAProfile(profile, field) {
  if (profile !== "axon-strict-v2") {
    throw invalidRuntime(`${field} is not canonical`);
  }
}

function requiredRuntimeStringList(value, field) {
  if (!Array.isArray(value) || value.length === 0) {
    throw invalidRuntime(`${field} must be a non-empty array`);
  }
  return value.map((item, index) => requiredRuntimeText(item, `${field}[${index}]`));
}

function requiredRuntimeBase64String(value, field, expectedLength) {
  const text = requiredRuntimeText(value, field);
  validateRuntimeBase64(text, field, expectedLength, false);
  return text;
}

function stableRuntimeObjectJSON(value) {
  if (Array.isArray(value)) {
    return `[${value.map(stableRuntimeObjectJSON).join(",")}]`;
  }
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableRuntimeObjectJSON(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
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

function requiredRuntimeNonNegativeInteger(value, field) {
  if (!Number.isSafeInteger(value) || value < 0) {
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

function requiredRuntimeStringAllowEmpty(value, field) {
  if (typeof value !== "string" || value !== value.trim()) {
    if (field === "authority_proof.proof_payload_base64") {
      throw invalidRuntime("runtime receipt summary is missing authority_proof.proof_payload_base64");
    }
    throw invalidRuntime(`runtime receipt summary is missing ${field}`);
  }
  return value;
}

function requiredRuntimeText(value, field) {
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidRuntime(`${field} must be a non-empty string`);
  }
  return value.trim();
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

function validateRuntimeBase64(value, field, expectedLength = null, allowEmpty = false) {
  let raw;
  try {
    raw = Buffer.from(value, "base64");
  } catch {
    throw invalidRuntime(`${field} must be base64`);
  }
  if ((raw.length === 0 && !allowEmpty) || raw.toString("base64") !== value) {
    throw invalidRuntime(`${field} must be base64`);
  }
  if (expectedLength !== null && raw.length !== expectedLength) {
    throw invalidRuntime(`${field} must decode to exactly ${expectedLength} bytes`);
  }
  return raw;
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
    throw invalidRuntimeError(`unknown runtime error code: ${text}`);
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

export function runtimeStateReadSubjectURA(realm, userID) {
  return buildRuntimeStateReadSubjectURA(realm, userID, {
    invalidInvocation,
    invalidRuntime,
  });
}

function receiptReadSubjectURA(calleeURA, authority) {
  if (authority instanceof DelegationProof) {
    return authority.subjectURA;
  }
  if (authority instanceof SessionAuthority) {
    const callee = parseCanonicalURA(calleeURA, "callee_ura");
    return runtimeStateReadSubjectURA(callee.realm, authority.sessionOwnerUserID);
  }
  throw historyError(
    ErrorCode.AUTHORITY_DENIED,
    "receipt read call context authority has an unsupported canonical type",
  );
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
    case ErrorCode.RUNTIME_OFFLINE:
    case ErrorCode.TRANSPORT:
      return ErrorClass.AVAILABILITY;
    case ErrorCode.PERMISSION_DENIED:
    case ErrorCode.HTTP_AUTH_DENIED:
    case ErrorCode.CALLER_IDENTITY_UNAVAILABLE:
      return ErrorClass.PERMISSION;
    case ErrorCode.ADMISSION_DENIED:
    case ErrorCode.SIGNATURE_DENIED:
    case ErrorCode.POLICY_DENIED:
    case ErrorCode.AUTHORITY_DENIED:
    case ErrorCode.AUTHORITY_SUBJECT_MISMATCH:
    case ErrorCode.EXECUTION_FAILED:
    case ErrorCode.ABILITY_FAILED:
    case ErrorCode.CALLER_SIGNER_UNAVAILABLE:
    case ErrorCode.RECEIPT_PROOF_FACTS_MISSING:
      return ErrorClass.ADMISSION;
    case ErrorCode.ABILITY_NOT_FOUND:
    case ErrorCode.ROUTE_UNAVAILABLE:
    case ErrorCode.NOT_FOUND:
    case ErrorCode.DESCRIPTOR_NOT_FOUND:
    case ErrorCode.DESCRIPTOR_OWNER_OFFLINE:
    case ErrorCode.DESCRIPTOR_MODE_UNSUPPORTED:
    case ErrorCode.DESCRIPTOR_STALE:
    case ErrorCode.RUNTIME_ROUTE_UNAVAILABLE:
    case ErrorCode.PROVIDER_UNAVAILABLE:
      return ErrorClass.ROUTING;
    case ErrorCode.TIMEOUT:
    case ErrorCode.INVOCATION_TIMEOUT:
      return ErrorClass.TIMEOUT;
    case ErrorCode.CANCELLED:
    case ErrorCode.INVOCATION_CANCELLED:
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
  throw invalidRuntimeError("retry must be never, safe, after_backoff, or unknown");
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
  throw invalidRuntimeError("JSON payload must be bytes or string");
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

function requiredBuilderPrincipalString(value, field) {
  const cleaned = requiredBuilderString(value, field);
  if (containsAllZeroPrincipal(cleaned)) {
    throw invalidInvocation(`${field} must not be all-zero`);
  }
  return cleaned;
}

function requiredHistoryPrincipalString(value, field) {
  if (typeof value !== "string" || value.trim() === "" || value.trim() !== value) {
    throw historyError(ErrorCode.INVALID_INVOCATION, `${field} is required`);
  }
  if (containsAllZeroPrincipal(value)) {
    throw historyError(ErrorCode.INVALID_INVOCATION, `${field} must not be all-zero`);
  }
  return value;
}

function optionalHistoryPrincipalString(value, field) {
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value !== "string" || value.trim() !== value) {
    throw historyError(ErrorCode.INVALID_INVOCATION, `${field} must be a string or null`);
  }
  if (value && containsAllZeroPrincipal(value)) {
    throw historyError(ErrorCode.INVALID_INVOCATION, `${field} must not be all-zero`);
  }
  return value;
}

function requiredWireString(value, field) {
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidRuntimeError(`${field} is required`);
  }
  return value;
}

function stringValue(value, field, allowEmpty = false) {
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value !== "string" || (!allowEmpty && value === "")) {
    throw invalidRuntimeError(`${field} must be a string`);
  }
  return value;
}

function objectValue(value, field) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw invalidRuntimeError(`${field} must be an object`);
  }
  return { ...value };
}

function nonNegativeInteger(value, field) {
  if (!Number.isInteger(value) || value < 0) {
    throw invalidRuntimeError(`${field} must be a non-negative integer`);
  }
  return value;
}

function booleanValue(value, field) {
  if (typeof value !== "boolean") {
    throw invalidRuntimeError(`${field} must be a boolean`);
  }
  return value;
}

function stringMap(value, field) {
  const map = objectValue(value, field);
  for (const item of Object.values(map)) {
    if (typeof item !== "string") {
      throw invalidRuntimeError(`${field} must map strings to strings`);
    }
  }
  return map;
}

function boolMap(value, field) {
  const map = objectValue(value, field);
  for (const item of Object.values(map)) {
    if (typeof item !== "boolean") {
      throw invalidRuntimeError(`${field} must map strings to booleans`);
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
  rejectAllZeroAuthorityFields({
    issuer_ura: proof.issuerURA,
    subject_ura: proof.subjectURA,
    caller_ura: proof.callerURA,
    audience: proof.audience,
  });
  if (proof.expiresAtMS <= proof.issuedAtMS) {
    throw invalidAuthority("delegation authority expires_at_ms must be greater than issued_at_ms");
  }
  if (proof.signature.length === 0) {
    throw invalidAuthority("delegation authority signature is required");
  }
}

function validateSessionAuthority(authority) {
  validateSessionAuthorityRequiredFacts(authority);
  rejectAllZeroAuthorityFields({
    issuer_ura: authority.issuerURA,
    session_owner_user_id: authority.sessionOwnerUserID,
    session_owner_ura: authority.sessionOwnerURA,
    creator_principal_id: authority.creatorPrincipalID,
    creator_principal_ura: authority.creatorPrincipalURA,
    callee_ura: authority.calleeURA,
    subject_ura: authority.subjectURA,
    audience: authority.audience,
  });
  if (authority.expiresAtMS <= authority.issuedAtMS) {
    throw invalidAuthority("session authority expires_at_ms must be greater than issued_at_ms");
  }
  if (authority.signature.length === 0) {
    throw invalidAuthority("session authority signature is required");
  }
  validateSessionAuthoritySubjectBinding(
    authority.subjectURA,
    authority.sessionOwnerUserID,
    authority.sessionID,
  );
}

function validateDelegationRequest(request) {
  rejectAllZeroAuthorityFields({
    issuer_ura: request.issuerURA,
    subject_ura: request.subjectURA,
    caller_ura: request.callerURA,
    audience: request.audience,
  });
  if (request.expiresAtMS <= request.issuedAtMS) {
    throw invalidAuthority("delegation authority expires_at_ms must be greater than issued_at_ms");
  }
  rejectAuthorityPrivateKeyMetadata(request.metadata);
}

function validateSessionAuthorityRequest(request) {
  validateSessionAuthorityRequiredFacts(request);
  rejectAllZeroAuthorityFields({
    issuer_ura: request.issuerURA,
    session_owner_user_id: request.sessionOwnerUserID,
    session_owner_ura: request.sessionOwnerURA,
    creator_principal_id: request.creatorPrincipalID,
    creator_principal_ura: request.creatorPrincipalURA,
    callee_ura: request.calleeURA,
    subject_ura: request.subjectURA,
    audience: request.audience,
  });
  if (request.expiresAtMS <= request.issuedAtMS) {
    throw invalidAuthority("session authority expires_at_ms must be greater than issued_at_ms");
  }
  rejectAuthorityPrivateKeyMetadata(request.metadata);
  validateSessionAuthoritySubjectBinding(
    request.subjectURA,
    request.sessionOwnerUserID,
    request.sessionID,
  );
}

function validateSessionAuthorityRequiredFacts(authority) {
  if (
    !String(authority.issuerURA ?? "").trim() ||
    !String(authority.sessionID ?? "").trim() ||
    !String(authority.sessionOwnerUserID ?? "").trim() ||
    !String(authority.creatorPrincipalID ?? "").trim() ||
    !String(authority.calleeURA ?? "").trim() ||
    !String(authority.subjectURA ?? "").trim() ||
    !String(authority.audience ?? "").trim()
  ) {
    throw invalidAuthority(
      "session authority must bind issuer, session id, owner, creator principal, callee, subject, and audience",
    );
  }
}

function normalizeSessionAuthorityPrincipals(fields) {
  let sessionOwnerUserID = String(fields.sessionOwnerUserID ?? "").trim();
  let sessionOwnerURA = String(fields.sessionOwnerURA ?? "").trim();
  let creatorPrincipalID = String(fields.creatorPrincipalID ?? "").trim();
  let creatorPrincipalURA = String(fields.creatorPrincipalURA ?? "").trim();
  const subjectURA = requiredAuthorityString(fields.subjectURA, "subject_ura");

  if (!sessionOwnerURA) {
    sessionOwnerURA = sessionOwnerURAFromSubject(subjectURA, sessionOwnerUserID);
  }
  if (sessionOwnerURA) {
    const ownerUserID = userIDFromUserURA(sessionOwnerURA, "session_owner_ura");
    if (sessionOwnerUserID && sessionOwnerUserID !== ownerUserID) {
      throw invalidAuthority("session_owner_user_id must match session_owner_ura user id");
    }
    sessionOwnerUserID = ownerUserID;
  }

  if (creatorPrincipalURA) {
    requireCanonicalURA(creatorPrincipalURA, "creator_principal_ura");
    if (creatorPrincipalID && creatorPrincipalID !== creatorPrincipalURA) {
      throw invalidAuthority("creator_principal_id must match creator_principal_ura");
    }
    creatorPrincipalID = creatorPrincipalURA;
  } else if (creatorPrincipalID.startsWith("easynet:///")) {
    try {
      requireCanonicalURA(creatorPrincipalID, "creator_principal_id");
      creatorPrincipalURA = creatorPrincipalID;
    } catch {
      creatorPrincipalURA = "";
    }
  }

  return {
    sessionOwnerUserID,
    sessionOwnerURA,
    creatorPrincipalID,
    creatorPrincipalURA,
    subjectURA,
  };
}

function sessionOwnerURAFromSubject(subjectURA, ownerUserID) {
  const owner = String(ownerUserID ?? "").trim();
  if (!owner) {
    return "";
  }
  const subject = canonicalAuthoritySubject(subjectURA);
  if (!subject || subject.ownerUserID !== owner || (subject.kind !== "user" && subject.kind !== "session")) {
    return "";
  }
  return `easynet:///r/${subject.realm}/user/${owner}`;
}

function userIDFromUserURA(raw, field) {
  const parsed = parseCanonicalURA(raw, field);
  if (!parsed.path.startsWith("user/")) {
    throw invalidAuthority(`${field} must be a canonical user URA`);
  }
  const userID = parsed.path.slice("user/".length).trim();
  if (!userID || userID.includes("/")) {
    throw invalidAuthority(`${field} must be a canonical user URA`);
  }
  return userID;
}

function requireCanonicalURA(raw, field) {
  parseCanonicalURA(raw, field);
  return raw;
}

function parseCanonicalURA(raw, field) {
  const value = requiredAuthorityString(raw, field);
  const realmPrefix = "easynet:///r/";
  if (!value.startsWith(realmPrefix)) {
    throw invalidAuthority(`${field} must be a canonical URA`);
  }
  const rest = value.slice(realmPrefix.length);
  const slash = rest.indexOf("/");
  if (slash <= 0 || slash === rest.length - 1) {
    throw invalidAuthority(`${field} must be a canonical URA`);
  }
  return {
    realm: rest.slice(0, slash),
    path: rest.slice(slash + 1),
  };
}

function validateSessionAuthoritySubjectBinding(subjectURA, sessionOwnerUserID, sessionID) {
  if (!canonicalSessionAuthorityID(sessionID)) {
    throw invalidAuthority("session authority session_id is not canonical");
  }
  const subject = canonicalAuthoritySubject(subjectURA);
  if (!subject || (subject.kind !== "user" && subject.kind !== "session")) {
    throw invalidAuthority("session authority subject_ura must be a canonical user or session subject");
  }
  const owner = String(sessionOwnerUserID ?? "").trim();
  if (subject.ownerUserID !== owner) {
    throw invalidAuthority("session authority user subject must match session_owner_user_id");
  }
  if (subject.kind === "session" && subject.sessionID !== String(sessionID ?? "").trim()) {
    throw invalidAuthority(
      "session authority subject_ura owner/session must match session_owner_user_id and session_id",
    );
  }
}

function canonicalSessionAuthorityID(sessionID) {
  return CANONICAL_SESSION_AUTHORITY_ID.test(String(sessionID ?? "").trim());
}

function canonicalAuthoritySubject(subjectURA) {
  const parsed = parseCanonicalURANullable(subjectURA);
  if (!parsed) {
    return null;
  }
  const { realm, path } = parsed;
  if (path.startsWith("user/")) {
    const ownerUserID = path.slice("user/".length).trim();
    if (!ownerUserID || ownerUserID.includes("/")) {
      return null;
    }
    return { kind: "user", realm, ownerUserID };
  }
  if (!path.startsWith("resource/user.")) {
    return null;
  }
  const resource = path.slice("resource/user.".length);
  const sessionMarker = "/session/";
  const sessionIndex = resource.indexOf(sessionMarker);
  if (sessionIndex <= 0) {
    return null;
  }
  const ownerUserID = resource.slice(0, sessionIndex).trim();
  const sessionID = resource.slice(sessionIndex + sessionMarker.length).trim();
  if (
    !ownerUserID ||
    ownerUserID.includes(".") ||
    ownerUserID.includes("/") ||
    !sessionID ||
    sessionID.includes("/")
  ) {
    return null;
  }
  return { kind: "session", realm, ownerUserID, sessionID };
}

function parseCanonicalURANullable(raw) {
  const value = String(raw ?? "").trim();
  const realmPrefix = "easynet:///r/";
  if (!value.startsWith(realmPrefix)) {
    return null;
  }
  const rest = value.slice(realmPrefix.length);
  const slash = rest.indexOf("/");
  if (slash <= 0 || slash === rest.length - 1) {
    return null;
  }
  return {
    realm: rest.slice(0, slash),
    path: rest.slice(slash + 1),
  };
}

function rejectAllZeroAuthorityFields(fields) {
  for (const [field, value] of Object.entries(fields)) {
    if (containsAllZeroPrincipal(String(value ?? ""))) {
      throw invalidAuthority(`${field} must not be all-zero`);
    }
  }
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

function validateInvocationAuthorityBinding(draft) {
  const authority = invocationAuthorityFromMetadata(draft.metadata);
  if (!authority) {
    return;
  }
  new InvocationAuthorityBindingValidator(draft, authority).validate();
}

function rejectGovernanceReadPublicInvocationDescriptor(calleeURA, descriptorRef) {
  const ability = RuntimeAbilityProjection.fromDescriptorRef(calleeURA, descriptorRef);
  const governanceAbility =
    runtimeGovernanceReadAbility(ability.publicName) ||
    runtimeGovernanceReadAbility(ability.intrinsicName);
  if (!governanceAbility) {
    return;
  }
  throw invalidInvocation(
    `runtime governance read ability \`${governanceAbility}\` is not a public invocation action; ` +
      "use RuntimeReceiptProvider, SessionHistoryOperations, or the canonical runtime catalogue provider path",
  );
}

function runtimeGovernanceReadAbility(value) {
  const clean = String(value ?? "").trim();
  const explicit = runtimeExplicitGovernanceReadAbility(clean) || clean;
  if (runtimeCatalogueReadAbility(explicit)) {
    return "meta.list_abilities";
  }
  if (runtimeReceiptReadAbility(explicit)) {
    return "invocation.history.list";
  }
  return "";
}

function runtimeExplicitGovernanceReadAbility(value) {
  for (const ability of RUNTIME_GOVERNANCE_READ_ABILITIES) {
    if (value === ability || value.endsWith(`.${ability}`)) {
      return ability;
    }
  }
  return "";
}

function runtimeGovernanceDescriptorProviderForAbility(value) {
  const clean = String(value ?? "").trim();
  if (runtimeCatalogueReadAbility(clean)) {
    return RUNTIME_ABILITY_DESCRIPTOR_PROVIDER;
  }
  if (runtimeReceiptReadAbility(clean)) {
    return RUNTIME_RECEIPT_HISTORY_PROVIDER;
  }
  return "";
}

function runtimeGovernanceDescriptorProviderForRequest(abilityName, provider) {
  const ability = requiredRuntimeText(abilityName, "ability name");
  const expectedProvider = runtimeGovernanceDescriptorProviderForAbility(ability);
  const requestedProvider = optionalRuntimeString(provider, "descriptor_provider") ?? "";
  if (!expectedProvider) {
    if (requestedProvider) {
      throw invalidInvocation(
        `descriptor_ref provider ${requestedProvider} cannot resolve non-governance ability ${ability}`,
      );
    }
    throw invalidInvocation(
      "runtime governance read requires a runtime governance ability",
    );
  }
  if (!requestedProvider) {
    return expectedProvider;
  }
  if (requestedProvider !== expectedProvider) {
    throw invalidInvocation(
      `descriptor_ref provider ${requestedProvider} cannot resolve ability ${ability}; use provider ${expectedProvider}`,
    );
  }
  return requestedProvider;
}

function runtimeCatalogueReadAbility(value) {
  return (
    value === "meta.list_abilities" ||
    value === "runtime.catalog.list" ||
    value.endsWith(".meta.list_abilities") ||
    value.includes(".runtime.catalog.")
  );
}

function runtimeReceiptReadAbility(value) {
  return (
    value.startsWith("invocation.history.") ||
    value.startsWith("invocation.trace.") ||
    value.includes(".invocation.history.") ||
    value.includes(".invocation.trace.")
  );
}

class InvocationAuthorityBindingValidator {
  constructor(draft, authority) {
    this.draft = draft;
    this.authority = authority;
    this.ability = RuntimeAbilityProjection.fromInvocation(draft);
    this.details = {
      caller_ura: draft.callerURA,
      callee_ura: draft.calleeURA,
      subject_ura: draft.subjectURA,
      descriptor_ref: draft.descriptorRef,
      authority_session_subject: authority.subjectURA,
    };
  }

  validate() {
    if (this.authority instanceof DelegationProof) {
      this.validateDelegation();
      return;
    }
    this.validateSession();
  }

  validateDelegation() {
    this.require(
      this.authority.callerURA.trim() === this.draft.callerURA.trim(),
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority caller does not match invocation caller_ura",
    );
    this.require(
      this.authority.subjectURA.trim() === this.draft.subjectURA.trim(),
      ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
      "delegation authority subject does not match invocation subject_ura",
    );
    this.require(
      authorityAudienceAdmits(this.authority.audience, this.draft.calleeURA),
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority audience does not admit invocation callee_ura",
    );
    this.require(
      authorityScopesAdmit(this.authority.scopes, this.ability),
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority scopes do not admit invocation ability",
    );
  }

  validateSession() {
    this.require(
      this.authority.issuerURA.trim() === this.draft.callerURA.trim(),
      ErrorCode.AUTHORITY_DENIED,
      "session authority issuer does not match invocation caller_ura",
    );
    this.require(
      this.authority.calleeURA.trim() === this.draft.calleeURA.trim(),
      ErrorCode.AUTHORITY_DENIED,
      "session authority callee does not match invocation callee_ura",
    );
    this.require(
      sessionAuthorityAdmitsSubject(this.authority, this.draft.subjectURA),
      ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
      "session authority subject does not admit invocation subject_ura",
    );
    this.require(
      authorityAudienceAdmits(this.authority.audience, this.draft.calleeURA),
      ErrorCode.AUTHORITY_DENIED,
      "session authority audience does not admit invocation callee_ura",
    );
    this.require(
      authorityListAdmits(this.authority.allowedActions, "invoke"),
      ErrorCode.AUTHORITY_DENIED,
      "session authority allowed_actions do not admit invoke",
    );
    this.require(
      authorityScopesAdmit(this.authority.allowedFollowupAbilities, this.ability),
      ErrorCode.AUTHORITY_DENIED,
      "session authority allowed_followup_abilities do not admit invocation ability",
    );
    this.require(
      authorityScopesAdmit(this.authority.scopes, this.ability),
      ErrorCode.AUTHORITY_DENIED,
      "session authority scopes do not admit invocation ability",
    );
  }

  require(condition, code, message) {
    if (!condition) {
      throw authorityBindingError(code, message, this.details);
    }
  }
}

function invocationAuthorityFromMetadata(metadata) {
  const value = objectValue(metadata ?? {}, "metadata");
  const delegation = authorityMetadataValue(value, DELEGATION_METADATA_KEY);
  if (delegation) {
    return DelegationProof.fromMetadata(delegation);
  }
  const session = authorityMetadataValue(value, SESSION_AUTHORITY_METADATA_KEY);
  if (session) {
    return SessionAuthority.fromMetadata(session);
  }
  return null;
}

function validateSessionHistoryRequest(request) {
  if (!(request instanceof ReceiptListRequest)) {
    throw historyError(ErrorCode.INVALID_INVOCATION, "Receipt list request is required");
  }
  validateSessionHistoryRuntimeCall(request.call);
  validateSessionHistoryFilterBinding(request.call, request.filter);
}

function validateSessionHistoryRuntimeCall(call) {
  if (!(call instanceof RuntimeCallContext)) {
    throw historyError(ErrorCode.INVALID_INVOCATION, "runtime call context is required");
  }
  try {
    validateRuntimeAbilityCallContext(call);
  } catch (error) {
    if (error instanceof SDKError) {
      throw historyError(ErrorCode.INVALID_INVOCATION, error.message, runtimeCallDetails(call), error);
    }
    throw error;
  }
  if (
    !isRuntimeStateReadSubjectURA(call.subjectURA) &&
    !isRuntimeOwnerReadSubjectURA(call.subjectURA, call.calleeURA)
  ) {
    throw historyError(
      ErrorCode.INVALID_INVOCATION,
      "session history call.subject_ura must be a user-owned runtime-state read subject or the callee runtime-owner subject",
      runtimeCallDetails(call),
    );
  }
  const authority = runtimeCallAuthority(call);
  if (authority === null) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session history requires runtime authority bound to the receipt query tuple",
      runtimeCallDetails(call),
    );
  }
  validateSessionHistoryAuthorityBinding(authority, call);
}

function validateSessionHistoryFilterBinding(call, filter) {
  if (filter === null) {
    return;
  }
  if (!(filter instanceof ReceiptFilter)) {
    throw historyError(
      ErrorCode.INVALID_INVOCATION,
      "Receipt filter is required",
      runtimeCallDetails(call),
    );
  }
  const callerURA = call.callerURA.trim();
  const calleeURA = call.calleeURA.trim();
  const details = runtimeCallDetails(call);
  if (filter.callerURA && filter.callerURA.trim() !== callerURA) {
    details.filter_caller_ura = filter.callerURA;
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "receipt filter caller_ura does not match receipt query caller_ura",
      details,
    );
  }
  if (filter.calleeURA && filter.calleeURA.trim() !== calleeURA) {
    details.filter_callee_ura = filter.calleeURA;
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "receipt filter callee_ura does not match receipt query callee_ura",
      details,
    );
  }
}

function runtimeCallAuthority(call) {
  const metadata = objectValue(call.metadata ?? {}, "metadata");
  validateAuthorityMetadata(metadata);
  const rawPresent = Boolean(
    authorityMetadataValue(metadata, DELEGATION_METADATA_KEY) ||
      authorityMetadataValue(metadata, SESSION_AUTHORITY_METADATA_KEY),
  );
  if (call.authority !== null) {
    if (rawPresent) {
      throw historyError(
        ErrorCode.INVALID_INVOCATION,
        "runtime call authority must be supplied once as a typed authority or metadata, not both",
        runtimeCallDetails(call),
      );
    }
    return call.authority;
  }
  return invocationAuthorityFromMetadata(metadata);
}

function normalizeRuntimeCallAuthority(value) {
  if (value === null || value === undefined) {
    return null;
  }
  if (value instanceof DelegationProof || value instanceof SessionAuthority) {
    return value;
  }
  if (typeof value === "object") {
    const object = objectValue(value, "authority");
    if (object.kind === "delegation" || object.key === DELEGATION_METADATA_KEY) {
      return object.value ? DelegationProof.fromMetadata(object.value) : new DelegationProof(object);
    }
    if (object.kind === "session_authority" || object.key === SESSION_AUTHORITY_METADATA_KEY) {
      return object.value ? SessionAuthority.fromMetadata(object.value) : new SessionAuthority(object);
    }
  }
  throw historyError(ErrorCode.AUTHORITY_DENIED, "runtime call authority has an unsupported canonical type");
}

function validateSessionHistoryAuthorityBinding(authority, call) {
  const callerURA = call.callerURA.trim();
  const calleeURA = call.calleeURA.trim();
  const subjectURA = call.subjectURA.trim();
  const details = runtimeCallDetails(call);
  if (authority instanceof DelegationProof) {
    validateSessionHistoryDelegationBinding(authority, callerURA, calleeURA, subjectURA, details);
    return;
  }
  if (authority instanceof SessionAuthority) {
    validateSessionHistorySessionBinding(authority, callerURA, calleeURA, subjectURA, details);
    return;
  }
  throw historyError(ErrorCode.AUTHORITY_DENIED, "runtime call authority has an unsupported canonical type");
}

function validateSessionHistoryDelegationBinding(proof, callerURA, calleeURA, subjectURA, details) {
  if (proof.callerURA.trim() !== callerURA) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority caller does not match receipt query caller_ura",
      details,
    );
  }
  if (proof.subjectURA.trim() !== subjectURA) {
    throw historyError(
      ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
      "delegation authority subject does not match receipt query subject_ura",
      details,
    );
  }
  if (!authorityAudienceAdmits(proof.audience, calleeURA)) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority audience does not admit receipt query callee_ura",
      details,
    );
  }
  if (!authorityScopeMatchesList(proof.scopes, "invocation.history.list")) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "delegation authority scopes do not admit invocation.history.list",
      details,
    );
  }
}

function validateSessionHistorySessionBinding(authority, callerURA, calleeURA, subjectURA, details) {
  details.authority_session_subject = authority.subjectURA;
  if (authority.issuerURA.trim() !== callerURA) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session authority issuer does not match receipt query caller_ura",
      details,
    );
  }
  if (authority.calleeURA.trim() !== calleeURA) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session authority callee does not match receipt query callee_ura",
      details,
    );
  }
  if (!authorityAudienceAdmits(authority.audience, calleeURA)) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session authority audience does not admit receipt query callee_ura",
      details,
    );
  }
  if (!isRuntimeStateReadSubjectURA(subjectURA)) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session authority cannot authorize a runtime-owner receipt history subject; use a delegation authority or a user-owned runtime-state read subject",
      details,
    );
  }
  if (!sessionAuthorityAdmitsSubject(authority, subjectURA)) {
    throw historyError(
      ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
      "session authority subject does not admit receipt query subject_ura",
      details,
    );
  }
  if (!authorityScopeMatchesList(authority.scopes, "invocation.history.list")) {
    throw historyError(
      ErrorCode.AUTHORITY_DENIED,
      "session authority scopes do not admit invocation.history.list",
      details,
    );
  }
}

function runtimeCallDetails(call) {
  return {
    caller_ura: call.callerURA,
    callee_ura: call.calleeURA,
    subject_ura: call.subjectURA,
  };
}

function sessionAuthorityAdmitsSubject(authority, subjectURA) {
  const subject = subjectURA.trim();
  if (!canonicalSessionAuthorityID(authority.sessionID)) {
    return false;
  }
  if (authority.subjectURA.trim() === subject) {
    return true;
  }
  const resource = canonicalResourceSubject(subject);
  if (!resource) {
    return false;
  }
  if (
    resource.resourcePath.startsWith("session/") &&
    !canonicalSessionAuthorityID(resource.resourcePath.slice("session/".length))
  ) {
    return false;
  }
  const ownerUserID = authority.sessionOwnerUserID.trim();
  if (!ownerUserID) {
    return false;
  }
  if (resource.ownerID === `user.${ownerUserID}`) {
    return true;
  }
  if (!resource.ownerID.startsWith("agent.")) {
    return false;
  }
  const rest = resource.ownerID.slice("agent.".length);
  const dot = rest.indexOf(".");
  return dot > 0 && rest.slice(0, dot) === ownerUserID;
}

function authorityAudienceAdmits(audience, calleeURA) {
  const pattern = audience.trim();
  const callee = calleeURA.trim();
  return pattern === "*" || pattern === callee || (pattern.endsWith("/") && callee.startsWith(pattern));
}

function authorityScopesAdmit(patterns, ability) {
  return patterns.some(
    (pattern) =>
      authorityScopeMatches(pattern, ability.publicName) ||
      authorityScopeMatches(pattern, ability.abilityURA),
  );
}

function authorityListAdmits(patterns, value) {
  return patterns.some((pattern) => authorityScopeMatches(pattern, value));
}

function authorityScopeMatchesList(patterns, value) {
  return Array.isArray(patterns) && patterns.some((pattern) => authorityScopeMatches(pattern, value));
}

function authorityScopeMatches(pattern, value) {
  const cleanPattern = String(pattern ?? "").trim();
  const cleanValue = String(value ?? "").trim();
  if (!cleanPattern || !cleanValue) {
    return false;
  }
  if (cleanPattern === "*") {
    return true;
  }
  if (cleanPattern.endsWith("*")) {
    const prefix = cleanPattern.slice(0, -1);
    return Boolean(prefix) && cleanValue.startsWith(prefix);
  }
  return cleanPattern === cleanValue;
}

class RuntimeAbilityProjection {
  constructor({ abilityURA, publicName, intrinsicName }) {
    this.abilityURA = abilityURA;
    this.publicName = publicName;
    this.intrinsicName = intrinsicName;
    Object.freeze(this);
  }

  static fromInvocation(draft) {
    return this.fromDescriptorRef(draft.calleeURA, draft.descriptorRef);
  }

  static fromDescriptorRef(calleeURA, descriptorRef) {
    const projection = this.descriptorAbilityProjection(descriptorRef);
    const publicName = this.publicAbilityName(calleeURA, projection.intrinsicName);
    return new RuntimeAbilityProjection({ ...projection, publicName });
  }

  static descriptorAbilityProjection(descriptorRef) {
    const clean = String(descriptorRef ?? "").trim();
    const hash = clean.indexOf("#");
    const bang = clean.indexOf("!");
    const limit = Math.min(
      ...[hash, bang].filter((index) => index >= 0),
      clean.length,
    );
    const withoutMode = clean.slice(0, limit);
    const version = withoutMode.lastIndexOf("@");
    const abilityURA = (version >= 0 ? withoutMode.slice(0, version) : withoutMode).trim();
    const parsed = parseCanonicalURANullable(abilityURA);
    const path = parsed?.path ?? "";
    const abilityPrefix = "ability/";
    if (!path.startsWith(abilityPrefix)) {
      throw invalidAuthority("descriptor_ref must contain a canonical Ability URA");
    }
    const intrinsicName = path.slice(abilityPrefix.length).trim();
    if (!intrinsicName || intrinsicName.includes("/")) {
      throw invalidAuthority("descriptor_ref must contain a canonical Ability URA");
    }
    return { abilityURA, intrinsicName };
  }

  static publicAbilityName(calleeURA, intrinsicName) {
    const clean = String(intrinsicName ?? "").trim();
    const owner = this.abilityOwnerPrefix(calleeURA);
    if (owner && clean.startsWith(`${owner}.`)) {
      return clean.slice(owner.length + 1);
    }
    return "";
  }

  static abilityOwnerPrefix(calleeURA) {
    const parsed = parseCanonicalURANullable(calleeURA);
    const path = parsed?.path ?? "";
    if (path.startsWith("device/")) {
      const deviceID = path.slice("device/".length).trim();
      if (deviceID && !deviceID.includes("/")) {
        return `device.${deviceID}`;
      }
    }
    if (path === "authority") {
      return "authority";
    }
    return "";
  }
}

function runtimeDescriptorRefRequest(request) {
  const value = objectValue(request, "descriptor ref request");
  rejectRuntimeFields(value, [
    "callee_ura",
    "ability",
    "call_mode",
    "caller_ura",
    "subject_ura",
    "provider",
  ]);
  const out = {
    callee_ura: requiredHistoryPrincipalString(value.callee_ura, "callee_ura"),
    ability: requiredRuntimeText(value.ability, "ability"),
    call_mode: requiredRuntimeText(value.call_mode, "call_mode"),
  };
  const caller = optionalRuntimeString(value.caller_ura, "caller_ura") ?? "";
  const subject = optionalRuntimeString(value.subject_ura, "subject_ura") ?? "";
  const provider = optionalRuntimeString(value.provider, "provider") ?? "";
  if (caller.trim()) out.caller_ura = caller.trim();
  if (subject.trim()) out.subject_ura = subject.trim();
  if (provider.trim()) out.provider = provider.trim();
  admitRuntimeDescriptorRefRequest(out);
  return out;
}

function admitRuntimeDescriptorRefRequest(request) {
  const expectedProvider = runtimeGovernanceDescriptorProviderForAbility(request.ability);
  if (!request.provider) {
    if (expectedProvider) {
      throw invalidRuntime(
        `descriptor_ref provider request for ability ${request.ability} requires provider ${expectedProvider}`,
      );
    }
    return;
  }
  if (
    request.provider !== RUNTIME_ABILITY_DESCRIPTOR_PROVIDER &&
    request.provider !== RUNTIME_RECEIPT_HISTORY_PROVIDER
  ) {
    throw invalidRuntime(`descriptor_ref request provider ${request.provider} is not supported`);
  }
  if (expectedProvider !== request.provider) {
    if (!expectedProvider) {
      throw invalidRuntime(
        `descriptor_ref provider ${request.provider} cannot resolve non-governance ability ${request.ability}`,
      );
    }
    throw invalidRuntime(
      `descriptor_ref provider ${request.provider} cannot resolve ability ${request.ability}; use provider ${expectedProvider}`,
    );
  }
  for (const field of ["caller_ura", "subject_ura"]) {
    if (!request[field]) {
      throw invalidRuntime(`descriptor_ref provider request requires ${field}`);
    }
    if (containsAllZeroPrincipal(request[field])) {
      throw invalidRuntime(`descriptor_ref provider request ${field} must not be all-zero`);
    }
  }
  admitRuntimeDescriptorRefProviderSubject(request);
}

function admitRuntimeDescriptorRefProviderSubject(request) {
  switch (request.provider) {
    case RUNTIME_ABILITY_DESCRIPTOR_PROVIDER:
      admitAbilityDescriptorProviderSubject(request.callee_ura, request.subject_ura);
      return;
    case RUNTIME_RECEIPT_HISTORY_PROVIDER:
      if (
        !isRuntimeStateReadSubjectURA(request.subject_ura) &&
        !isRuntimeOwnerReadSubjectURA(request.subject_ura, request.callee_ura)
      ) {
        throw invalidRuntime(
          "descriptor_ref provider receipt_history subject_ura must be a runtime governance read subject",
        );
      }
      return;
    default:
      throw invalidRuntime(`descriptor_ref request provider ${request.provider} is not supported`);
  }
}

function admitAbilityDescriptorProviderSubject(calleeURA, subjectURA) {
  const callee = parseCanonicalURANullable(calleeURA);
  if (!callee) {
    throw invalidRuntime("descriptor_ref provider ability_descriptor callee_ura must be canonical");
  }
  const subject = parseCanonicalURANullable(subjectURA);
  if (!subject) {
    throw invalidRuntime("descriptor_ref provider ability_descriptor subject_ura must be canonical");
  }
  if (subject.path !== "authority") {
    throw invalidRuntime("descriptor_ref provider ability_descriptor subject_ura must be an Authority URA");
  }
  if (subject.realm !== callee.realm || subjectURA !== `easynet:///r/${callee.realm}/authority`) {
    throw invalidRuntime(
      "descriptor_ref provider ability_descriptor subject_ura must be the callee realm authority subject",
    );
  }
}

function runtimeDescriptorRefResponse(raw) {
  const text = decodeText(raw).trim();
  if (text.startsWith("{")) {
    const value = parseJSON(text, "descriptor ref response");
    return requiredRuntimeText(value.descriptor_ref, "descriptor_ref");
  }
  return requiredRuntimeText(text, "descriptor_ref");
}

function runtimeAbilityDispatchPolicy(options = {}) {
  const value = objectValue(options ?? {}, "runtime ability dispatch policy");
  return {
    allowGovernanceRead: value.allow_governance_read === true,
    subjectPolicy: optionalRuntimeString(value.subject_policy, "subject_policy") ?? "descriptor_bound",
    descriptorProvider: optionalRuntimeString(value.descriptor_provider, "descriptor_provider") ?? "",
  };
}

function runtimeAbilityGovernanceReadPolicy(provider) {
  return {
    allowGovernanceRead: true,
    subjectPolicy: provider === RUNTIME_ABILITY_DESCRIPTOR_PROVIDER
      ? "runtime_owner"
      : "descriptor_bound",
    descriptorProvider: requiredRuntimeText(provider, "descriptor_provider"),
  };
}

function validateRuntimeAbilityCallContext(call) {
  if (!(call instanceof RuntimeCallContext)) {
    throw invalidInvocation("runtime call context is required");
  }
  if (!call.nonceBase64) {
    throw invalidInvocation("nonce_base64 is required");
  }
  if (call.causalContext === null) {
    throw invalidInvocation("causal_context is required");
  }
}

function runtimeAbilitySubjectURA(call, policy) {
  switch (policy.subjectPolicy) {
    case "descriptor_bound":
      return call.subjectURA.trim();
    case "runtime_owner":
      return call.calleeURA.trim();
    default:
      throw invalidInvocation("runtime ability subject policy is unsupported");
  }
}

function runtimeAbilityDescriptorResolutionSubjectURA(call, subjectURA, policy) {
  if (policy.descriptorProvider === RUNTIME_ABILITY_DESCRIPTOR_PROVIDER) {
    const callee = parseCanonicalURANullable(call.calleeURA);
    if (!callee) {
      throw invalidInvocation("callee_ura must be canonical for runtime catalogue reads");
    }
    return `easynet:///r/${callee.realm}/authority`;
  }
  if (policy.subjectPolicy === "runtime_owner") {
    return subjectURA.trim();
  }
  return call.subjectURA.trim();
}

function runtimeAbilityMetadata(call, ability, subjectURA) {
  const metadata = { ...objectValue(call.metadata ?? {}, "metadata") };
  if (call.authority !== null) {
    Object.assign(metadata, call.authority.metadata().toMetadata());
  }
  validateAuthorityMetadata(metadata);
  const draft = {
    callerURA: call.callerURA,
    calleeURA: call.calleeURA,
    subjectURA,
    descriptorRef: ability.abilityURA,
    metadata,
  };
  validateInvocationAuthorityBinding(draft);
  return metadata;
}

function runtimeAbilityObjectOutput(result) {
  const value = objectValue(result, "invocation result");
  if (value.ok !== true) {
    throw new SDKError({
      code: ErrorCode.ABILITY_FAILED,
      stage: "runtime",
      retry: RetryHint.NEVER,
      message: "runtime ability invocation failed",
      details: value,
    });
  }
  const output = value.output ?? {};
  return objectValue(output, "invocation output");
}

function validateAbilityDescriptorProjection(descriptor) {
  const ability = parseCanonicalURANullable(descriptor.abilityURA);
  if (!ability || !ability.path.startsWith("ability/")) {
    throw invalidRuntime("ability descriptor ability_ura must be canonical");
  }
  const owner = parseCanonicalURANullable(descriptor.ownerURA);
  if (!owner) {
    throw invalidRuntime("ability descriptor owner_ura must be canonical");
  }
  const projected = RuntimeAbilityProjection.fromDescriptorRef(
    descriptor.ownerURA,
    descriptor.descriptorRef,
  );
  if (projected.abilityURA !== descriptor.abilityURA) {
    throw invalidRuntime("ability descriptor descriptor_ref does not bind ability_ura");
  }
}

function receiptListArguments(request) {
  const args = {};
  if (request.filter !== null) {
    Object.assign(args, request.filter.toJSON());
  }
  args.limit = request.limit;
  if (request.cursor) {
    args.cursor = request.cursor;
  }
  return args;
}

function historyOutputRecords(output) {
  const records = output.records ?? [];
  if (!Array.isArray(records)) {
    throw invalidHistory("records must be an array");
  }
  return records.map((record, index) => objectValue(record, `records[${index}]`));
}

function receiptLedgerSource(output) {
  const source = output.source;
  if (typeof source === "string" && source.trim()) {
    return source.trim();
  }
  const ledgerURA = output.ledger_ura;
  if (typeof ledgerURA === "string" && ledgerURA.trim()) {
    return ledgerURA.trim();
  }
  return "invocation.history.list";
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
  return terminalToken(value.kind);
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

function invalidHealth(message, details = {}) {
  return invalidProfile(HEALTH_PROFILE, "decode", message, details);
}

function invalidAuthority(message, details = {}) {
  return invalidProfile(AUTHORITY_PROFILE, "authority", message, details);
}

function authorityBindingError(code, message, details = {}) {
  return new SDKError({
    code,
    stage: "authorize",
    retry: RetryHint.NEVER,
    source: AUTHORITY_PROFILE,
    message,
    details: profileErrorDetails(AUTHORITY_PROFILE, details),
  });
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

function invalidHistory(message, details = {}) {
  return historyError(ErrorCode.INVALID_INVOCATION, message, details);
}

function historyError(code, message, details = {}, cause = undefined) {
  return new SDKError({
    code,
    stage: "history",
    retry: RetryHint.NEVER,
    message,
    details,
    cause,
  });
}

function invalidRuntimeError(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "decode",
    retry: RetryHint.NEVER,
    message,
  });
}
