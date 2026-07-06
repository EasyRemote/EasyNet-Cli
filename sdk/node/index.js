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
    return parseJSON(raw, "prepared invocation");
  }

  async submitSigned(signed) {
    const transport = this.requireOpen();
    if (typeof transport.submitSigned !== "function") {
      throw invalidSDK("runtime submit-signed transport function is required");
    }
    if (!signed || typeof signed !== "object" || Array.isArray(signed)) {
      throw invalidInvocation("signed invocation is required");
    }
    const raw = await transport.submitSigned(Buffer.from(JSON.stringify(signed)));
    return parseJSON(raw, "invocation handle");
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

export class StreamHandle {
  constructor(transport, open) {
    if (!transport || typeof transport.receive !== "function") {
      throw invalidSDK("stream transport is required");
    }
    this.transport = transport;
    this.open = objectValue(open, "stream open");
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
    return event;
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
}

export class BidiSession {
  constructor(transport, open) {
    if (!transport || typeof transport.send !== "function" || typeof transport.receive !== "function") {
      throw invalidSDK("bidi transport is required");
    }
    this.transport = transport;
    this.open = objectValue(open, "bidi open");
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
    await withAbortSignal(
      this.transport.send(Buffer.from(JSON.stringify(frame))),
      options.signal,
      () => this.cancel(options.cancelReason ?? abortReason(options.signal)),
    );
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
    return frame;
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

function normalizeErrorCode(code) {
  const text = requiredWireString(code, "code");
  if (!ERROR_CODES.has(text)) {
    throw invalidDaemonError(`unknown daemon error code: ${text}`);
  }
  return text;
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

function invalidDirectory(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "directory",
    retry: RetryHint.NEVER,
    source: DIRECTORY_IDENTITY_PROFILE,
    message,
  });
}

function invalidReceipt(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "receipt",
    retry: RetryHint.NEVER,
    source: RECEIPT_PROFILE,
    message,
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

function invalidDaemonError(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "decode",
    retry: RetryHint.NEVER,
    message,
  });
}
