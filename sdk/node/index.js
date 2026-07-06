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
  return ["terminal", "closed", "completed", "failed", "cancelled", "canceled", "timedout", "done"].includes(token);
}

function invalidSDK(message) {
  return new SDKError({
    code: ErrorCode.INVALID_ARGUMENT,
    stage: "sdk",
    retry: RetryHint.NEVER,
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
