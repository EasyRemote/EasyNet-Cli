import assert from "node:assert/strict";
import test from "node:test";

import {
  AdminClient,
  Client,
  CompatibilityClient,
  DEFAULT_DIRECTORY_PAGE_SIZE,
  DEFAULT_EVENT_PAGE_SIZE,
  DEFAULT_SURFACE_PAGE_SIZE,
  DirectoryClient,
  EventClient,
  ErrorClass,
  ErrorCode,
  HOST_STREAM_EMPTY_OUTPUT_HASH,
  HOST_STREAM_FRAME_SCHEMA,
  HOST_STREAM_HASH_ALGORITHM,
  HostBindingClient,
  HostStreamHashState,
  HealthClient,
  InvocationSignature,
  InvocationHandle,
  IdentityClient,
  InvocationBuilder,
  LocalHostBindingTransport,
  MAX_BIDI_BUFFERED_FRAMES,
  MAX_STREAM_BUFFERED_EVENTS,
  MissionClient,
  PreparedInvocation,
  PublicationClient,
  ReceiptChain,
  ReceiptClient,
  ReceiptRef,
  RuntimeClient,
  SDKError,
  SurfaceClient,
  SurfaceStatus,
  profileErrorDetails,
  profileSourceRef,
} from "../index.js";

const completeDraft = () =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef("opaque-descriptor-ref-from-identity-profile")
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs({})
    .withContentType("application/json");

const runtimeCoreDraft = () =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs({})
    .withContentType("application/json");

const directoryBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/device/dev-a",
  subject_ura: "easynet:///r/example/device/dev-a",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
});

const receiptFetch = () => ({
  ...directoryBase(),
  descriptor_ref: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  invocation_ura: "easynet:///r/example/resource/invocation.inv-1",
});

const receiptHash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const eventFrame = (overrides = {}) => ({
  profile: "events",
  stream: "directory",
  kind: "directory.agent_advertised",
  event_id: "evt-directory-8",
  cursor: { stream: "directory", sequence: 8, token: "directory:8" },
  resume_token: "directory:8",
  occurred_unix_ms: 1783100000123,
  occurred_at: "2026-07-03T17:33:20.123Z",
  subject_ref: {
    kind: "ura",
    ura: "easynet:///r/example/agent/alice.main",
    role: "agent",
  },
  tenant_ref: { kind: "realm", realm: "example" },
  payload: {
    type: "agent_advertised",
    agent_ura: "easynet:///r/example/agent/alice.main",
  },
  dropped_count: 0,
  reconnect_after_ms: null,
  terminal: false,
  metadata: {
    profile: "events",
    stream: "directory",
    carrier_owner: "daemon_sdk",
  },
  ...overrides,
});

const eventCarrier = () => ({
  ...directoryBase(),
  stream: "directory",
  realm: "example",
  agent_ura: "easynet:///r/example/agent/alice.main",
  resume_cursor: { stream: "directory", sequence: 7 },
  heartbeat_interval_ms: 30000,
  metadata: { request_id: "events-directory-subscribe-1" },
});

const missionBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/device/dev-a",
  subject_ura: "easynet:///r/example/device/dev-a",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
  metadata: { request_id: "mission-1" },
});

const missionID = "2026-07-04_010203_weather";

const missionStatus = () => ({
  profile: "mission",
  kind: "mission_status",
  mission_id: missionID,
  state: "partial",
  terminal: true,
  partial_failures: 1,
  cancelled: false,
  parent_invocation_id: null,
  parent_receipt_ura: "easynet:///r/example/receipt/parent",
  parent_invocation: {
    caller: "easynet:///r/example/agent/alice.sdk",
    callee: "easynet:///r/example/device/dev-a",
    ability: "mission.run",
    subject: "easynet:///r/example/device/dev-a",
    causal_context: {
      form: "scalar",
      receipt_ura: "easynet:///r/example/receipt/parent",
      receipt_hash_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    },
  },
  child_invocations: [
    {
      step_id: "s1",
      request_id: "req-1",
      trace_id: missionID,
      ability: "observe.health",
      invocation_ura: "easynet:///r/example/invocation/req-1",
      caller_ura: "easynet:///r/example/device/dev-a",
      callee_ura: "easynet:///r/example/device/dev-a",
      subject_ura: "easynet:///r/example/device/dev-a",
      metadata_state: "receipt_backed",
      ledger_state: "completed",
      receipt: {
        receipt_ura: "easynet:///r/example/receipt/child",
        receipt_hash: "bbbb",
        head_receipt_hash: "bbbb",
      },
    },
  ],
  child_receipts: [
    {
      step_id: "s1",
      invocation_ura: "easynet:///r/example/invocation/req-1",
      receipt_ura: "easynet:///r/example/receipt/child",
      receipt_hash: "bbbb",
    },
  ],
  output_refs: [
    {
      kind: "run_dir",
      path: `/tmp/easynet/missions/runs/${missionID}`,
    },
  ],
  metadata: {
    profile: "mission",
    carrier_owner: "daemon_sdk",
    status_source: "mission_result",
    running: false,
    name: "weather",
    trace_id: missionID,
  },
});

const missionEvent = (overrides = {}) => ({
  profile: "mission",
  kind: "mission_event",
  mission_id: missionID,
  sequence: 4,
  event_type: "progress",
  occurred_unix_ms: 1783126923000,
  terminal: false,
  payload: {
    step_id: "s1",
    state: "running",
    message: "observe.health started",
  },
  receipt: {},
  metadata: {
    profile: "mission",
    carrier_owner: "daemon_sdk",
  },
  ...overrides,
});

const missionEventPage = () => ({
  profile: "mission",
  kind: "mission_event_page",
  mission_id: missionID,
  cursor_sequence: 4,
  next_cursor_sequence: 7,
  has_more: false,
  dropped_count: 0,
  events: [
    missionEvent(),
    missionEvent({
      sequence: 6,
      event_type: "completed",
      terminal: true,
      payload: {
        state: "partial",
        partial_failures: 1,
        steps_completed: 1,
        steps_failed: 1,
      },
      receipt: {
        receipt_ura: "easynet:///r/example/receipt/parent",
        receipt_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        head_receipt_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      },
      metadata: {
        profile: "mission",
        carrier_owner: "daemon_sdk",
        status_source: "mission_result",
      },
    }),
  ],
  metadata: {
    profile: "mission",
    carrier_owner: "daemon_sdk",
    status_source: "mission_event_log",
  },
});

const missionDraftJSON = (descriptorRef, args = {}) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs(args)
    .withContentType("application/json")
    .withMetadata({ profile: "mission" })
    .build()
    .toJSONString();

const adminBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/device/dev-a",
  subject_ura: "easynet:///r/example/device/dev-a",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
  metadata: { request_id: "admin-1" },
});

const gatewayStatus = () => ({
  profile: "admin_gateway",
  gateway_id: "device:example:dev-a",
  ready: true,
  state: "ready",
  process_live: true,
  control_ready: true,
  runtime_ready: true,
  directory_ready: true,
  trust_ready: true,
  public_listener_ready: false,
  listeners: [
    { kind: "control", endpoint: "/tmp/easynet-control.sock", ready: true, public: false },
    { kind: "invocation", endpoint: "/tmp/easynet-daemon.sock", ready: true, public: false },
  ],
  identity: { mode: "device", realm: "example", node_id: "dev-a" },
  metadata: { profile: "admin_gateway", source: "daemon_lifecycle_status" },
});

const adminAgentRecords = () => ({
  profile: "admin_gateway",
  kind: "agent_records",
  state: "ok",
  items: [
    {
      name: "codex",
      agent_ura: "easynet:///r/example/agent/alice.codex",
      owner_ura: "easynet:///r/example/user/alice",
      device_ura: null,
      state: "registered",
      runtime: "codex",
      model: "gpt-5",
      label: "primary",
      abilities: [],
      metadata: { profile: "admin_gateway", source: "agent.list" },
    },
  ],
  next_cursor: null,
  metadata: { profile: "admin_gateway", source: "agent.list", count: 1 },
});

const adminLifecycleResult = (overrides = {}) => ({
  profile: "admin_gateway",
  kind: "agent_lifecycle_result",
  operation: "agent.start",
  state: "ok",
  agent_ura: "easynet:///r/example/agent/alice.codex",
  ack: null,
  runtime_not_ready: false,
  runtime_catalog_not_ready: false,
  metadata: { profile: "admin_gateway", source: "agent_lifecycle" },
  ...overrides,
});

const pairingPreflight = () => ({
  profile: "admin_gateway",
  kind: "pairing_preflight",
  state: "requires_pairing",
  hub_ura: "easynet:///r/example/hub/main",
  device_ura: "easynet:///r/example/device/dev-a",
  pairing_required: true,
  trust_ready: false,
  scopes: ["invoke", "events"],
  metadata: { profile: "admin_gateway", source: "pairing.preflight" },
});

const pairingToken = () => ({
  profile: "admin_gateway",
  kind: "pairing_token",
  token_id: "pair-token-1",
  token: "pair-token-value",
  hub_ura: "easynet:///r/example/hub/main",
  device_ura: "easynet:///r/example/device/dev-a",
  state: "issued",
  expires_unix_ms: 1893456000000,
  scopes: ["invoke", "events"],
  metadata: { profile: "admin_gateway", source: "pairing.create" },
});

const deviceCredential = () => ({
  profile: "admin_gateway",
  kind: "device_credential",
  credential_id: "cred-dev-a",
  device_ura: "easynet:///r/example/device/dev-a",
  hub_ura: "easynet:///r/example/hub/main",
  state: "active",
  issued_unix_ms: 1767225600000,
  expires_unix_ms: 1893456000000,
  scopes: ["invoke", "events"],
  metadata: { profile: "admin_gateway", source: "pairing.validate" },
});

const deviceSession = () => ({
  profile: "admin_gateway",
  kind: "device_session",
  session_id: "dev-session-1",
  device_ura: "easynet:///r/example/device/dev-a",
  hub_ura: "easynet:///r/example/hub/main",
  state: "active",
  session_kind: "remote_desktop",
  created_unix_ms: 1767225600000,
  expires_unix_ms: 1893456000000,
  metadata: { profile: "admin_gateway", source: "session.create" },
});

const deviceSessionPage = () => ({
  profile: "admin_gateway",
  kind: "device_sessions",
  state: "ok",
  items: [deviceSession()],
  next_cursor: null,
  metadata: { profile: "admin_gateway", source: "session.list" },
});

const adminDraftJSON = (descriptorRef, args = {}) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs(args)
    .withContentType("application/json")
    .withMetadata({ profile: "admin_gateway" })
    .build()
    .toJSONString();

const surfaceBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/agent/alice.pages",
  subject_ura: "easynet:///r/example/agent/alice.pages",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
  metadata: { request_id: "surface-list-1" },
});

const surfacePageRecord = (overrides = {}) => ({
  profile: "surface",
  kind: "page_record",
  page_id: "docs",
  owner_ura: "easynet:///r/example/agent/alice.pages",
  surface_ref: "easynet:///r/example/resource/alice.docs",
  public_ref: "https://example/web/alice/docs/",
  status: "published",
  metadata: {
    profile: "surface",
    source_ability: "pages.get",
    project_id: "docs",
  },
  ...overrides,
});

const surfaceManifest = () => ({
  profile: "surface",
  kind: "surface_manifest",
  page_id: "docs",
  owner_ura: "easynet:///r/example/agent/alice.pages",
  surface_ref: "easynet:///r/example/resource/alice.docs",
  public_ref: "https://example/web/alice/docs/",
  page: surfacePageRecord(),
  entrypoint: {
    kind: "public_page_ref",
    href: "https://example/web/alice/docs/",
  },
  metadata: {
    profile: "surface",
    source_ability: "pages.get",
  },
});

const surfaceHealth = () => ({
  profile: "surface",
  kind: "surface_health",
  state: "ready",
  ready: true,
  owner_ura: "easynet:///r/example/agent/alice.pages",
  surface_ref: "easynet:///r/example/resource/alice.docs",
  descriptor_ref: "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
  descriptor_version: "1.0.0",
  page_count: 1,
  checks: [
    {
      name: "manifest",
      state: "ready",
      ready: true,
      message: null,
      latency_ms: 3,
      metadata: { source: "pages.get" },
    },
  ],
  metadata: {
    profile: "surface",
    source_ability: "pages.health",
    rendering_owner: "backend",
  },
});

const surfaceDraftJSON = (descriptorRef, args = {}) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/agent/alice.pages")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/agent/alice.pages")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs(args)
    .withContentType("application/json")
    .withMetadata({ profile: "surface" })
    .build()
    .toJSONString();

const compatibilityBase = () => ({
  caller_ura: "easynet:///r/example/agent/alice.sdk",
  callee_ura: "easynet:///r/example/device/dev-a",
  subject_ura: "easynet:///r/example/device/dev-a",
  descriptor_version: "1.0.0",
  nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA==",
  causal_context: { form: "none" },
  auth_token: "tok_example",
  metadata: { request_id: "compatibility-1" },
});

const compatibilityAbilityURA = "easynet:///r/example/ability/alice.codex.chat";

const compatibilityChatRequest = (overrides = {}) => ({
  ...compatibilityBase(),
  request: {
    model: compatibilityAbilityURA,
    messages: [{ role: "user", content: "reply with: ok" }],
    temperature: 0.2,
    ...overrides,
  },
});

const compatibilityFileRequest = (overrides = {}) => ({
  ...compatibilityBase(),
  id: "file-easynet-docs-1",
  file_ref: "easynet:///r/example/resource/alice.files/prompt.jsonl",
  owner_ura: "easynet:///r/example/agent/alice.sdk",
  filename: "prompt.jsonl",
  purpose: "batch",
  content_type: "application/jsonl",
  content_hash: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  size_bytes: 19,
  created_at: 1783094400,
  status: "processed",
  ...overrides,
});

const compatibilityModelPage = () => ({
  profile: "compatibility",
  kind: "model_page",
  object: "list",
  data: [
    {
      profile: "compatibility",
      kind: "model",
      id: compatibilityAbilityURA,
      object: "model",
      created: 1783094400,
      owned_by: "easynet:///r/example/agent/alice.sdk",
      ability_ref: compatibilityAbilityURA,
      metadata: { profile: "compatibility", source: "openai.list_models" },
    },
  ],
  next_cursor: null,
  metadata: { profile: "compatibility", source: "openai.list_models" },
});

const compatibilityChatCompletion = () => ({
  profile: "compatibility",
  kind: "chat_completion",
  id: "chatcmpl-easynet-1",
  object: "chat.completion",
  created: 1783094401,
  model: compatibilityAbilityURA,
  choices: [{ index: 0, message: { role: "assistant", content: "ok" }, finish_reason: "stop" }],
  usage: { prompt_tokens: 5, completion_tokens: 1, total_tokens: 6 },
  metadata: { profile: "compatibility", source: "openai.chat_completions" },
});

const compatibilityChatStream = () => ({
  profile: "compatibility",
  kind: "chat_completion_stream",
  stream: true,
  items: [
    {
      profile: "compatibility",
      kind: "chat_completion_chunk",
      id: "chatcmpl-easynet-1",
      object: "chat.completion.chunk",
      created: 1783094401,
      model: compatibilityAbilityURA,
      choices: [{ index: 0, delta: { content: "ok" }, finish_reason: null }],
      usage: null,
      metadata: { profile: "compatibility", sequence: 1 },
    },
  ],
  done_sentinel: "[DONE]",
  metadata: { profile: "compatibility", source: "openai.chat_completions.stream" },
});

const compatibilityFile = () => ({
  profile: "compatibility",
  kind: "file",
  id: "file-easynet-docs-1",
  object: "file",
  bytes: 19,
  created_at: 1783094400,
  filename: "prompt.jsonl",
  purpose: "batch",
  status: "processed",
  metadata: {
    profile: "compatibility",
    source: "compatibility.file",
    file_ref: "easynet:///r/example/resource/alice.files/prompt.jsonl",
  },
});

const compatibilityFileDeleteResult = () => ({
  profile: "compatibility",
  kind: "file_delete_result",
  id: "file-easynet-docs-1",
  object: "file",
  deleted: true,
  metadata: { profile: "compatibility", source: "compatibility.file.delete" },
});

const compatibilityDraftJSON = (descriptorRef, args = {}) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs(args)
    .withContentType("application/json")
    .withMetadata({ profile: "compatibility" })
    .build()
    .toJSONString();

const publicationResourceRef = () => ({
  resource_ura: "easynet:///r/example/resource/fs.local.pkg",
  owner_ura: "easynet:///r/example/device/dev-a",
  namespace: "fs",
  display_path: "/tmp/easynet/pkg",
  capability: "read",
  expires_unix_ms: 0,
  revision: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
});

const publicationDeploy = () => ({
  ...directoryBase(),
  resource_ref: publicationResourceRef(),
  node_id: "local",
  metadata: { request_id: "deploy-1" },
});

const publicationDraftJSON = (descriptorRef) =>
  new InvocationBuilder()
    .withCallerURA("easynet:///r/example/agent/alice.sdk")
    .withCalleeURA("easynet:///r/example/device/dev-a")
    .withDescriptorRef(descriptorRef)
    .withSubjectURA("easynet:///r/example/device/dev-a")
    .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
    .withCausalContext({ form: "none" })
    .withJSONArgs({ ok: true })
    .withContentType("application/json")
    .withMetadata({ profile: "publication" })
    .build()
    .toJSONString();

const hostBindingRequest = () => ({
  binding_id: "binding-weather-1",
  descriptor_ref: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
  endpoint: "/tmp/easynet-weather.sock",
  frame_schema: HOST_STREAM_FRAME_SCHEMA,
  cleanup: { mode: "unlink_socket" },
  timeout_ms: 30000,
});

const preparedInvocationJSON = (overrides = {}) => ({
  prepared_id: "prepared-example-1",
  tuple: runtimeCoreDraft().build().toJSON(),
  signing_material: {
    algorithm: "ed25519",
    canonical_bytes_base64: "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=",
    args_digest_hex: "0000000000000000000000000000000000000000000000000000000000000000",
    descriptor_ref: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
    expires_at_unix_ms: 1783000000000,
  },
  submit_ready: false,
  ...overrides,
});

const callerSignature = () =>
  new InvocationSignature({
    algorithm: "ed25519",
    signature_base64: "c2lnbmF0dXJl",
    key_id_hint: "signer-alice-key-1",
  });

test("feature discovery decodes canonical Runtime Core facts", async () => {
  const client = new Client({
    featureDiscovery: () =>
      JSON.stringify({
        abi_version: 4,
        sdk_version: "0.91.30",
        profiles: { runtime_core: "seam" },
        symbols: { runtime_health: true },
        axon_pb: false,
      }),
  });

  const features = await client.requireABI(4);
  assert.equal(features.abiVersion, 4);
  assert.equal(features.version().sdkVersion, "0.91.30");
  assert.equal(features.symbols.runtime_health, true);

  await assert.rejects(
    () => client.requireABI(5),
    (error) => error instanceof SDKError && error.code === ErrorCode.VERSION_MISMATCH,
  );
});

test("HealthClient decodes runtime health and diagnostics DTOs", async () => {
  const calls = [];
  const client = new HealthClient({
    runtimeHealth: () => {
      calls.push("health");
      return JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: true,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: true,
        version: "0.1.0",
        abi_version: 4,
        mismatch: null,
        diagnostics: [],
      });
    },
    runtimeDiagnostics: () => {
      calls.push("diagnostics");
      return JSON.stringify({
        profile: "health",
        kind: "diagnostics_report",
        state: "Running",
        ready: true,
        version: "0.91.30",
        abi_version: 4,
        control_endpoint: "/tmp/easynet/control.json",
        invocation_endpoint: "/tmp/easynet/daemon.sock",
        checks: [{ name: "runtime", ready: true, message: null }],
        diagnostics: [],
      });
    },
  });

  const health = await client.runtimeHealth();
  const diagnostics = await client.diagnostics();

  assert.equal(health.apiAlive(), true);
  assert.equal(health.ready(), true);
  assert.equal(health.abiVersion, 4);
  assert.equal(health.toJSON().runtime_ready, true);
  assert.equal(diagnostics.profile, "health");
  assert.equal(diagnostics.kind, "diagnostics_report");
  assert.equal(diagnostics.checks.length, 1);
  assert.deepEqual(calls, ["health", "diagnostics"]);
});

test("HealthClient preserves API liveness separate from runtime readiness", async () => {
  const client = new HealthClient({
    runtimeHealth: () =>
      JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: false,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: false,
        diagnostics: ["invocation endpoint unavailable"],
      }),
  });

  const health = await client.runtimeHealth();

  assert.equal(health.apiAlive(), true);
  assert.equal(health.ready(), false);
  assert.equal(health.invocationReady, false);
  assert.deepEqual(health.diagnostics, ["invocation endpoint unavailable"]);
  await assert.rejects(
    () => client.diagnostics(),
    (error) => error instanceof SDKError && error.code === ErrorCode.NOT_IMPLEMENTED,
  );
});

test("HealthClient rejects malformed payloads and wraps transport failure", async () => {
  await assert.rejects(
    () =>
      new HealthClient({
        runtimeHealth: () =>
          JSON.stringify({
            api_ready: true,
            daemon_ready: true,
            invocation_ready: true,
            directory_ready: true,
            trust_ready: true,
            runtime_ready: true,
            abi_version: true,
          }),
      }).runtimeHealth(),
    (error) =>
      error instanceof SDKError &&
      error.code === ErrorCode.INVALID_ARGUMENT &&
      error.source === "health",
  );

  const down = new Error("daemon unavailable");
  await assert.rejects(
    () =>
      new HealthClient({
        runtimeHealth: () => {
          throw down;
        },
      }).runtimeHealth(),
    (error) =>
      error instanceof SDKError &&
      error.code === ErrorCode.ROUTE_UNAVAILABLE &&
      error.cause === down,
  );
});

test("HealthClient closes transport and rejects closed use", async () => {
  const calls = [];
  const client = new HealthClient({
    runtimeHealth: () =>
      JSON.stringify({
        api_ready: true,
        daemon_ready: true,
        invocation_ready: true,
        directory_ready: true,
        trust_ready: true,
        runtime_ready: true,
        diagnostics: [],
      }),
    close: () => {
      calls.push("close");
    },
  });

  await client.close();
  await client.close();
  assert.deepEqual(calls, ["close"]);
  await assert.rejects(
    () => client.runtimeHealth(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("InvocationBuilder validates tuple completeness without descriptor grammar", () => {
  const builder = completeDraft();
  const inspected = builder.inspect();
  assert.equal(inspected.descriptorRef, "opaque-descriptor-ref-from-identity-profile");

  const draft = builder.build();
  assert.equal(draft.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(draft.toJSON().descriptor_ref, "opaque-descriptor-ref-from-identity-profile");
  assert.throws(
    () => builder.inspect(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_HANDLE,
  );

  assert.throws(
    () =>
      new InvocationBuilder()
        .withCallerURA("easynet:///r/example/agent/alice.sdk")
        .withCalleeURA("easynet:///r/example/device/dev-a")
        .withDescriptorRef("descriptor")
        .withSubjectURA("easynet:///r/example/device/dev-a")
        .withNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==")
        .withCausalContext({ form: "none" })
        .withContentType("application/json")
        .build(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("RuntimeClient delegates through injected transport and rejects closed use", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: (draftJSON) => {
      seen.push(JSON.parse(Buffer.from(draftJSON).toString("utf8")));
      return JSON.stringify({ ok: true, terminal_state: "Completed" });
    },
    close: () => {
      seen.push({ closed: true });
    },
  });

  const result = await runtime.invoke(completeDraft().build());
  assert.equal(result.ok, true);
  assert.equal(seen[0].caller_ura, "easynet:///r/example/agent/alice.sdk");

  await runtime.close();
  await runtime.close();
  assert.deepEqual(seen.at(-1), { closed: true });
  await assert.rejects(
    () => runtime.invoke(completeDraft().build()),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("RuntimeClient.prepare returns daemon-provided canonical signing material", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    prepare: (draftJSON, optionsJSON) => {
      seen.push({
        draft: JSON.parse(Buffer.from(draftJSON).toString("utf8")),
        options: JSON.parse(Buffer.from(optionsJSON).toString("utf8")),
      });
      return JSON.stringify(preparedInvocationJSON());
    },
  });

  const prepared = await runtime.prepare(runtimeCoreDraft().build(), { deadline_unix_ms: 1783000000000 });

  assert.equal(prepared instanceof PreparedInvocation, true);
  assert.equal(prepared.submitReady(), false);
  assert.equal(prepared.preparedId, "prepared-example-1");
  assert.equal(prepared.signingMaterial.algorithm, "ed25519");
  assert.equal(prepared.signingMaterial.canonicalBytesBase64, "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=");
  assert.equal(
    prepared.signingMaterial.descriptorRef,
    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  );
  assert.equal(seen[0].draft.descriptor_ref, prepared.signingMaterial.descriptorRef);
  assert.deepEqual(seen[0].options, { deadline_unix_ms: 1783000000000 });
});

test("PreparedInvocation enforces non-submit-ready canonical material boundaries", () => {
  assert.throws(
    () => new PreparedInvocation(preparedInvocationJSON({ submit_ready: true })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () => new PreparedInvocation(preparedInvocationJSON({ submit_ready: "false" })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );

  assert.throws(
    () =>
      new PreparedInvocation(
        preparedInvocationJSON({
          signing_material: {
            ...preparedInvocationJSON().signing_material,
            descriptor_ref: "easynet:///r/example/ability/other@1.0.0",
          },
        }),
      ),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );

  assert.throws(
    () =>
      new PreparedInvocation(
        preparedInvocationJSON({
          canonical_hash_hex: "0000000000000000000000000000000000000000000000000000000000000000",
        }),
      ),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("PreparedInvocation signs with caller signature without rewriting daemon material", () => {
  const prepared = new PreparedInvocation(preparedInvocationJSON());
  const signed = prepared.signWithCallerSignature(callerSignature());
  const encoded = signed.toJSON();

  assert.equal(signed.submitReady(), true);
  assert.equal(encoded.signer_id, "signer-alice-key-1");
  assert.equal(encoded.signature.algorithm, "ed25519");
  assert.equal(encoded.signature.signature_base64, "c2lnbmF0dXJl");
  assert.equal(encoded.prepared.canonical_bytes_base64, prepared.signingMaterial.canonicalBytesBase64);
  assert.equal(encoded.prepared.descriptor_ref, prepared.descriptorRef);
});

test("RuntimeClient.submitSigned rejects prepared invocations before transport", async () => {
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    submitSigned: () => {
      throw new Error("transport must not receive prepared invocation");
    },
  });

  await assert.rejects(
    () => runtime.submitSigned(new PreparedInvocation(preparedInvocationJSON())),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("RuntimeClient submits signed envelopes and observes invocation handles", async () => {
  const seen = [];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    prepare: (draftJSON) => {
      seen.push(["prepare", JSON.parse(Buffer.from(draftJSON).toString("utf8"))]);
      return JSON.stringify(preparedInvocationJSON());
    },
    submitSigned: (signedJSON) => {
      seen.push(["submit", JSON.parse(Buffer.from(signedJSON).toString("utf8"))]);
      return JSON.stringify({
        handle_id: 7,
        state: "Submitted",
        terminal: false,
        events: [{ sequence: 1, kind: "submitted", state: "Submitted", terminal: false }],
        result: null,
      });
    },
    awaitHandle: (handleId) => {
      seen.push(["await", handleId]);
      return JSON.stringify({
        ok: true,
        terminal_state: "Completed",
        output_json: {},
        receipt: null,
        error: null,
      });
    },
    cancelHandle: (handleId, reason) => {
      seen.push(["cancel", handleId, reason]);
      return JSON.stringify({
        handle_id: 7,
        cancelled: false,
        state: "Completed",
        terminal: true,
      });
    },
    handleEvents: (handleId) => {
      seen.push(["events", handleId]);
      return JSON.stringify({
        handle_id: 7,
        state: "Completed",
        terminal: true,
        events: [
          {
            sequence: 2,
            kind: "terminal",
            state: "Completed",
            terminal: true,
            result: { ok: true },
          },
        ],
        result: { ok: true },
      });
    },
    freeHandle: (handleId) => {
      seen.push(["free", handleId]);
    },
  });

  const prepared = await runtime.prepare(runtimeCoreDraft().build());
  const signed = prepared.signWithCallerSignature(callerSignature());
  const handle = await runtime.submitSigned(signed);
  const result = await handle.awaitResult();
  const cancel = await handle.cancel("after terminal");
  const refreshed = await handle.refreshEvents();
  await handle.close();

  assert.equal(handle.handleId, 7);
  assert.equal(handle.state, "Submitted");
  assert.equal(handle.events[0].sequence, 1);
  assert.equal(result.terminal_state, "Completed");
  assert.equal(cancel.state, "Completed");
  assert.equal(cancel.cancelled, false);
  assert.equal(refreshed.terminal, true);
  assert.equal(refreshed.events.length, 1);
  assert.equal(refreshed.events[0].terminal, true);
  assert.equal(seen[0][0], "prepare");
  assert.equal(seen[1][0], "submit");
  assert.equal(seen[1][1].signature.signature_base64, "c2lnbmF0dXJl");
  assert.equal(seen[1][1].prepared.canonical_bytes_base64, "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=");
  assert.deepEqual(seen.slice(2), [
    ["await", 7],
    ["cancel", 7, "after terminal"],
    ["events", 7],
    ["free", 7],
  ]);
});

test("InvocationHandle rejects legacy aliases and terminal drift", () => {
  assert.throws(
    () =>
      new InvocationHandle({
        handleId: 7,
        state: "Submitted",
        terminal: false,
      }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () =>
      new InvocationHandle({
        handle_id: 7,
        state: "Completed",
        terminal: true,
        events: [
          { sequence: 1, kind: "terminal", state: "Completed", terminal: true },
          { sequence: 2, kind: "cancelled", state: "Cancelled", terminal: true },
        ],
      }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () =>
      new InvocationHandle({
        handle_id: 7,
        state: "Submitted",
        terminal: false,
        events: [{ sequence: 1, kind: "terminal", state: "Completed", terminal: true }],
      }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("typed daemon error JSON decodes canonical schema values", () => {
  const error = SDKError.fromJSON(
    JSON.stringify({
      code: "DAEMON_OFFLINE",
      stage: "transport",
      message: "daemon offline",
      retry: "safe",
      details: { profile: "runtime_core" },
    }),
  );

  assert.equal(error.code, ErrorCode.DAEMON_OFFLINE);
  assert.equal(error.retryable, true);
  assert.equal(error.details.profile, "runtime_core");
  assert.equal(error.errorClass(), ErrorClass.AVAILABILITY);
  assert.equal(error.profile(), "runtime_core");
  assert.equal(error.sourceRef(), "");
});

test("typed daemon error JSON rejects legacy code aliases", () => {
  for (const code of ["InvalidArgument", "DaemonDown", "DAEMON_DOWN", "VersionIncompatible"]) {
    assert.throws(
      () =>
        SDKError.fromJSON(
          JSON.stringify({
            code,
            stage: "transport",
            message: "legacy code",
            retry: "never",
            details: {},
          }),
        ),
      (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
    );
  }
});

test("profile errors expose stable source refs without schema changes", async () => {
  assert.equal(profileSourceRef(" publication "), "node_sdk.profile.publication");
  assert.deepEqual(
    profileErrorDetails("publication", {
      source_ref: "custom.source",
      reason: "resource_ref_namespace_reserved",
    }),
    {
      profile: "publication",
      source_ref: "custom.source",
      reason: "resource_ref_namespace_reserved",
    },
  );

  const publication = new PublicationClient({
    buildResourceRef: () => JSON.stringify(publicationResourceRef()),
  });

  await assert.rejects(
    () => publication.buildLocalResourceRef({ path: "relative/pkg", capability: "read" }),
    (error) => {
      assert.equal(error instanceof SDKError, true);
      assert.equal(error.code, ErrorCode.INVALID_ARGUMENT);
      assert.equal(error.errorClass(), ErrorClass.VALIDATION);
      assert.equal(error.profile(), "publication");
      assert.equal(error.sourceRef(), "node_sdk.profile.publication");
      assert.equal(error.details.profile, "publication");
      assert.equal(error.details.source_ref, "node_sdk.profile.publication");
      assert.equal(error.details.reason, "resource_ref_path_must_be_absolute");
      assert.equal(error.details.operation, "build_local_resource_ref");
      return true;
    },
  );
});

test("StreamHandle exposes async iteration with terminal close", async () => {
  const closed = [];
  const events = [
    { frame_type: "data", value: 1 },
    { frame_type: "terminal", terminal: true, state: "Completed" },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openStream: () => ({
      open: JSON.stringify({ stream_id: "stream-1", state: "Open" }),
      transport: {
        receive: () => JSON.stringify(events.shift()),
        close: () => {
          closed.push("stream-1");
        },
      },
    }),
  });

  const stream = await runtime.invokeStream(completeDraft().build());
  const seen = [];
  for await (const event of stream) {
    seen.push(event);
  }

  assert.deepEqual(seen.map((event) => event.frame_type), ["data", "terminal"]);
  assert.equal(stream.terminal, true);
  assert.deepEqual(closed, ["stream-1"]);
  await assert.rejects(
    () => stream.receive(),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("StreamHandle keeps retained event history bounded with typed overflow", async () => {
  const events = [
    { frame_type: "data", value: 1 },
    { frame_type: "data", value: 2 },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openStream: () => ({
      open: JSON.stringify({
        stream_id: "stream-bounded",
        state: "Open",
        max_buffered_events: 1,
      }),
      transport: {
        receive: () => JSON.stringify(events.shift()),
      },
    }),
  });

  const stream = await runtime.invokeStream(completeDraft().build());
  const first = await stream.receive();
  const overflow = await stream.receive();

  assert.equal(MAX_STREAM_BUFFERED_EVENTS, 1024);
  assert.equal(stream.maxBufferedEvents, 1);
  assert.deepEqual(stream.retainedEvents, [first]);
  assert.equal(stream.retainedEvents.length, 1);
  assert.equal(overflow.terminal, true);
  assert.equal(overflow.error.code, ErrorCode.ADMISSION_DENIED);
  assert.equal(overflow.error.retry, "after_backoff");
  assert.equal(overflow.error.details.reason, "callback_queue_overflow");
  assert.equal(overflow.error.details.wire_code, "RESOURCE_EXHAUSTED");
  assert.equal(stream.terminal, true);
  assert.deepEqual(stream.terminalEvent(), overflow);
});

test("StreamHandle AbortSignal cancellation calls transport cancel", async () => {
  const cancelled = [];
  const controller = new AbortController();
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openStream: () => ({
      open: JSON.stringify({ stream_id: "stream-2", state: "Open" }),
      transport: {
        receive: () => new Promise(() => {}),
        cancel: (reason) => {
          cancelled.push(reason);
        },
      },
    }),
  });

  const stream = await runtime.invokeStream(completeDraft().build());
  const pending = stream.receive({ signal: controller.signal, cancelReason: "operator cancelled" });
  controller.abort("ignored by explicit reason");

  await assert.rejects(
    pending,
    (error) => error instanceof SDKError && error.code === ErrorCode.CANCELLED,
  );
  assert.deepEqual(cancelled, ["operator cancelled"]);
  assert.equal(stream.closed, true);
});

test("BidiSession exposes async iteration and AbortSignal cancellation", async () => {
  const closed = [];
  const cancelled = [];
  const frames = [
    { frame_type: "data", payload: { ok: true } },
    { frame_type: "done", terminal: true, state: "Closed" },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openBidi: () => ({
      open: JSON.stringify({ session_id: "bidi-1", state: "Open" }),
      transport: {
        send: () => {},
        receive: () => (frames.length > 0 ? JSON.stringify(frames.shift()) : new Promise(() => {})),
        close: () => {
          closed.push("bidi-1");
        },
        cancel: (reason) => {
          cancelled.push(reason);
        },
      },
    }),
  });

  const bidi = await runtime.openBidi(completeDraft().build());
  await bidi.send({ frame_type: "data", payload: { hello: true } });
  const seen = [];
  for await (const frame of bidi.frames()) {
    seen.push(frame);
  }
  assert.deepEqual(seen.map((frame) => frame.frame_type), ["data", "done"]);
  assert.deepEqual(closed, ["bidi-1"]);

  const aborted = await runtime.openBidi(completeDraft().build());
  const controller = new AbortController();
  const pending = aborted.receive({ signal: controller.signal });
  controller.abort("stop bidi");
  await assert.rejects(
    pending,
    (error) => error instanceof SDKError && error.code === ErrorCode.CANCELLED,
  );
  assert.deepEqual(cancelled, ["stop bidi"]);
});

test("BidiSession keeps retained frame history bounded with typed overflow", async () => {
  const receiveFrames = [
    { frame_type: "data", payload: { one: true } },
    { frame_type: "data", payload: { two: true } },
  ];
  const runtime = new RuntimeClient({
    invoke: () => JSON.stringify({ ok: true }),
    openBidi: () => ({
      open: JSON.stringify({
        session_id: "bidi-bounded",
        state: "Open",
        max_buffered_frames: 1,
      }),
      transport: {
        send: () => {},
        receive: () => JSON.stringify(receiveFrames.shift()),
      },
    }),
  });

  const receiveBounded = await runtime.openBidi(completeDraft().build());
  const first = await receiveBounded.receive();
  const overflow = await receiveBounded.receive();

  assert.equal(MAX_BIDI_BUFFERED_FRAMES, 1024);
  assert.equal(receiveBounded.maxBufferedFrames, 1);
  assert.deepEqual(receiveBounded.receivedFrames, [first]);
  assert.equal(overflow.terminal, true);
  assert.equal(overflow.error.code, ErrorCode.ADMISSION_DENIED);
  assert.equal(overflow.error.retry, "after_backoff");
  assert.equal(overflow.error.details.reason, "callback_queue_overflow");
  assert.equal(overflow.error.details.wire_code, "RESOURCE_EXHAUSTED");
  assert.deepEqual(receiveBounded.terminalFrame(), overflow);

  const sendBounded = await runtime.openBidi(completeDraft().build());
  await sendBounded.send({ frame_type: "data", payload: { one: true } });
  await assert.rejects(
    () => sendBounded.send({ frame_type: "data", payload: { two: true } }),
    (error) =>
      error instanceof SDKError &&
      error.code === ErrorCode.ADMISSION_DENIED &&
      error.retry === "after_backoff" &&
      error.details.reason === "callback_queue_overflow",
  );
  assert.equal(sendBounded.sentFrames.length, 1);
  assert.equal(sendBounded.terminal, true);
  assert.equal(sendBounded.overflow.error.details.direction, "send");
});

test("IdentityClient delegates DescriptorRef and URA projections without local grammar", async () => {
  const seen = [];
  const identity = new IdentityClient({
    projectDescriptorRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project", request });
      return JSON.stringify({
        kind: "descriptor_ref",
        valid: true,
        descriptor_ref: request.descriptor_ref,
        ability_ura: "easynet:///r/example/ability/device.dev-a.observe.health",
        descriptor_version: "1.0.0",
        profile: "directory_identity",
        components: {},
        metadata: {},
      });
    },
    buildDescriptorRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build", request });
      return JSON.stringify({
        kind: "descriptor_ref",
        valid: true,
        descriptor_ref: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
        ability_ura: request.ability_ura,
        descriptor_version: request.descriptor_version,
        profile: "directory_identity",
        components: {},
        metadata: {},
      });
    },
    ownerAbilityURA: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "owner", request });
      return JSON.stringify({
        ability_ura: "easynet:///r/example/ability/device.dev-a.observe.health",
      });
    },
    resourceURA: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resource", request });
      return JSON.stringify({
        resource_ura: "easynet:///r/example/resource/alice.docs",
      });
    },
  });

  const projection = await identity.projectDescriptorRef({
    descriptor_ref: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  });
  assert.equal(
    projection.descriptor_ref,
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );

  const ability = await identity.abilityURAFromDescriptorRef(
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );
  assert.equal(ability, "easynet:///r/example/ability/device.dev-a.observe.health");

  const canonical = await identity.canonicalAbilityDescriptorRef(
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );
  assert.equal(
    canonical,
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );

  const descriptor = await identity.ownerAbilityDescriptorRef(
    "easynet:///r/example/device/dev-a",
    "observe.health",
    "1.0.0",
  );
  assert.equal(descriptor, "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0");

  const resource = await identity.resourceURA("easynet:///r/example/agent/alice.sdk", "docs");
  assert.equal(resource, "easynet:///r/example/resource/alice.docs");
  assert.deepEqual(seen.map((item) => item.method), [
    "project",
    "project",
    "project",
    "owner",
    "build",
    "resource",
  ]);
  assert.equal(
    seen[0].request.descriptor_ref,
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );
  assert.equal(
    seen[4].request.ability_ura,
    "easynet:///r/example/ability/device.dev-a.observe.health",
  );
  assert.equal(seen[4].request.descriptor_version, "1.0.0");

  await assert.rejects(
    () => identity.projectDescriptorRef({ descriptor_ref: " descriptor " }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("DirectoryClient delegates bounded read-model pages without fanout", async () => {
  const seen = [];
  const directory = new DirectoryClient({
    resolve: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resolve", request });
      return JSON.stringify({ kind: "resolved_ref", profile: "directory_identity" });
    },
    listDevices: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "devices", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "device_page",
        items: [],
        next_cursor: "",
        metadata: {},
      });
    },
    listAgents: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "agents", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "agent_page",
        items: [],
        next_cursor: "",
        metadata: {},
      });
    },
    listAbilities: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "abilities", request });
      return JSON.stringify({
        profile: "directory_identity",
        kind: "ability_page",
        items: [],
        next_cursor: "",
        metadata: {},
      });
    },
  });

  await directory.resolve({ ...directoryBase(), query_name: "dev-a", ability_name: "observe.health" });
  const devices = await directory.listDevices(directoryBase());
  const agents = await directory.listAgents({ ...directoryBase(), limit: 10 });
  const abilities = await directory.listAbilities({
    ...directoryBase(),
    limit: 25,
    scope: "owner",
    owner_ura: "easynet:///r/example/device/dev-a",
  });

  assert.equal(devices.kind, "device_page");
  assert.equal(agents.kind, "agent_page");
  assert.equal(abilities.kind, "ability_page");
  assert.equal(seen[1].request.limit, DEFAULT_DIRECTORY_PAGE_SIZE);
  assert.equal(seen[2].request.limit, 10);
  assert.equal(seen[3].request.limit, 25);
  assert.equal(seen[3].request.owner_ura, "easynet:///r/example/device/dev-a");

  await assert.rejects(
    () => directory.listDevices({ ...directoryBase(), limit: 501 }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => directory.resolve({ ...directoryBase() }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("DirectoryClient exposes subscription as StreamHandle seam", async () => {
  const events = [
    { kind: "directory_event", phase: "live" },
    { kind: "directory_event", phase: "terminal", terminal: true },
  ];
  const directory = new DirectoryClient({
    resolve: () => JSON.stringify({ kind: "resolved_ref" }),
    buildDirectorySubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.stream, "directory");
      return completeDraft().build().toJSONString();
    },
    subscribeDirectory: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      assert.equal(request.stream, "directory");
      return {
        open: JSON.stringify({ stream: "directory", state: "Live" }),
        transport: {
          receive: () => JSON.stringify(events.shift()),
          close: () => {},
        },
      };
    },
  });

  const carrier = await directory.buildDirectorySubscriptionInvocation(directoryBase());
  assert.equal(carrier.descriptor_ref, "opaque-descriptor-ref-from-identity-profile");

  const stream = await directory.subscribeDirectory(directoryBase());
  const phases = [];
  for await (const event of stream.events()) {
    phases.push(event.phase);
  }
  assert.deepEqual(phases, ["live", "terminal"]);
});

test("EventClient delegates event carriers, projections, history, and streams", async () => {
  const seen = [];
  const streamEvents = [
    eventFrame(),
    eventFrame({
      kind: "directory.terminal",
      event_id: "evt-directory-9",
      cursor: { stream: "directory", sequence: 9, token: "directory:9" },
      resume_token: "directory:9",
      payload: { reason: "client_closed" },
      terminal: true,
    }),
  ];
  const events = new EventClient({
    buildDirectorySubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "directory_carrier", request });
      return completeDraft().build().toJSONString();
    },
    buildDeviceSubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "device_carrier", request });
      return completeDraft().build().toJSONString();
    },
    buildSessionSubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "session_carrier", request });
      return completeDraft().build().toJSONString();
    },
    buildInvocationSubscriptionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "invocation_carrier", request });
      return completeDraft().build().toJSONString();
    },
    subscribeDirectory: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "directory_stream", request });
      return {
        open: JSON.stringify({
          stream: "directory",
          state: "Live",
          metadata: { profile: "events" },
          max_buffered_events: 4,
        }),
        transport: {
          receive: () => JSON.stringify(streamEvents.shift()),
          close: () => {},
        },
      };
    },
    listDeviceEvents: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "device_history", request });
      return JSON.stringify({
        profile: "events",
        stream: "device",
        item_kind: "device_event",
        items: [
          eventFrame({
            stream: "device",
            kind: "device.status_changed",
            event_id: "evt-device-8",
            cursor: { stream: "device", sequence: 8, token: "device:8" },
            resume_token: "device:8",
            subject_ref: {
              kind: "ura",
              ura: "easynet:///r/example/device/dev-a",
              role: "device",
            },
            payload: { state: "online" },
          }),
        ],
        next_cursor: null,
        has_more: false,
        limit: request.limit,
        metadata: { profile: "events", source: "device_event_history" },
      });
    },
    projectDirectoryEvent: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_directory", request });
      return JSON.stringify(eventFrame({ cursor: request.cursor, payload: request.event }));
    },
    projectLiveEvent: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_live", request });
      return JSON.stringify(eventFrame({
        stream: request.cursor.stream,
        kind: `${request.cursor.stream}.live`,
        cursor: request.cursor,
        resume_token: request.cursor.token,
        payload: request.event,
      }));
    },
    projectDropReport: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_drop", request });
      return JSON.stringify(eventFrame({
        kind: "directory.drop_report",
        event_id: "evt-directory-10",
        cursor: request.cursor,
        resume_token: request.cursor.token,
        payload: { reason: request.reason, dropped_count: request.dropped_count },
        dropped_count: request.dropped_count,
        reconnect_after_ms: request.reconnect_after_ms,
      }));
    },
    projectTerminal: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_terminal", request });
      return JSON.stringify(eventFrame({
        kind: "directory.terminal",
        event_id: "evt-directory-11",
        cursor: request.cursor,
        resume_token: request.cursor.token,
        payload: { reason: request.reason },
        terminal: true,
      }));
    },
  });

  const directoryCarrier = await events.buildDirectorySubscriptionInvocation(eventCarrier());
  const deviceCarrier = await events.buildDeviceSubscriptionInvocation({
    ...directoryBase(),
    stream: "device",
    filter: { device_ura: "easynet:///r/example/device/dev-a" },
    device_ura: "easynet:///r/example/device/dev-a",
    resume_cursor: { stream: "device", sequence: 2 },
    heartbeat_interval_ms: 30000,
  });
  const sessionCarrier = await events.buildSessionSubscriptionInvocation({
    ...directoryBase(),
    stream: "session",
    session_id: "run-1",
    resume_cursor: { stream: "session", sequence: 4 },
  });
  const invocationCarrier = await events.buildInvocationSubscriptionInvocation({
    ...directoryBase(),
    stream: "invocation",
    filter: { invocation_id: "inv-1" },
    invocation_id: "inv-1",
    resume_cursor: { stream: "invocation", sequence: 9 },
  });
  const page = await events.listDeviceEvents({
    ...directoryBase(),
    filter: { device_ura: "easynet:///r/example/device/dev-a" },
    device_ura: "easynet:///r/example/device/dev-a",
  });
  const projected = await events.projectDirectoryEvent({
    cursor: { stream: "directory", sequence: 8 },
    event: { type: "agent_advertised" },
  });
  const live = await events.projectLiveEvent({
    cursor: { stream: "device", sequence: 8 },
    event: { state: "online" },
  });
  const drop = await events.projectDropReport({
    cursor: { stream: "directory", sequence: 10 },
    occurred_unix_ms: 1783100000123,
    dropped_count: 4,
    reconnect_after_ms: 1000,
    reason: "consumer_lagged",
  });
  const terminal = await events.projectTerminal({
    cursor: { stream: "directory", sequence: 11 },
    occurred_unix_ms: 1783100000123,
    reason: "client_closed",
  });
  const stream = await events.subscribeDirectory(eventCarrier());
  const streamKinds = [];
  for await (const frame of stream.events()) {
    streamKinds.push(frame.kind);
  }

  assert.equal(directoryCarrier.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(deviceCarrier.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(sessionCarrier.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(invocationCarrier.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(page.limit, DEFAULT_EVENT_PAGE_SIZE);
  assert.equal(page.items[0].stream, "device");
  assert.equal(projected.cursor.resumeToken(), "directory:8");
  assert.equal(live.stream, "device");
  assert.equal(drop.droppedCount, 4);
  assert.equal(terminal.terminal, true);
  assert.deepEqual(streamKinds, ["directory.agent_advertised", "directory.terminal"]);
  assert.equal(seen[0].request.stream, "directory");
  assert.equal(seen[0].request.resume_cursor.token, "directory:7");
  assert.equal(seen[1].request.filter.device_ura, "easynet:///r/example/device/dev-a");
  assert.equal(seen[2].request.session_id, "run-1");
  assert.equal(seen[3].request.invocation_id, "inv-1");
  assert.equal(seen[4].request.limit, DEFAULT_EVENT_PAGE_SIZE);

  await assert.rejects(
    () => events.buildSessionSubscriptionInvocation({
      ...directoryBase(),
      stream: "session",
      session_ura: "easynet:///r/example/resource/session.run-1",
    }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("MissionClient delegates carriers, status projections, and event streams without execution policy", async () => {
  const seen = [];
  const streamFrames = [missionEvent(), missionEvent({ sequence: 5, event_type: "completed", terminal: true })];
  const mission = new MissionClient({
    buildRunEALInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_run", request });
      return missionDraftJSON("easynet:///r/example/ability/device.dev-a.mission.run@1.0.0", {
        source: request.source,
        label: request.label,
      });
    },
    buildRunFileInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_run_file", request });
      return missionDraftJSON("easynet:///r/example/ability/device.dev-a.mission.run@1.0.0", {
        path: request.path,
        label: request.label,
      });
    },
    buildTrackInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_track", request });
      return missionDraftJSON("easynet:///r/example/ability/device.dev-a.mission.track@1.0.0", {
        run_id: request.mission_id,
      });
    },
    buildCancelInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_cancel", request });
      return missionDraftJSON("easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0", {
        run_id: request.mission_id,
      });
    },
    buildEventsInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_events", request });
      return missionDraftJSON("easynet:///r/example/ability/device.dev-a.mission.events@1.0.0", {
        run_id: request.mission_id,
        cursor_sequence: request.cursor_sequence,
        limit: request.limit,
      });
    },
    runEAL: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "run", request });
      return JSON.stringify(missionStatus());
    },
    runFile: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "run_file", request });
      return JSON.stringify(missionStatus());
    },
    track: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "track", request });
      return JSON.stringify(missionStatus());
    },
    cancel: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "cancel", request });
      return JSON.stringify({ ...missionStatus(), state: "cancelled", cancelled: true });
    },
    events: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "events", request });
      return JSON.stringify(missionEventPage());
    },
    openEventStream: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "stream", request });
      return {
        open: JSON.stringify({ stream_id: "mission-stream-1", state: "Open" }),
        transport: {
          receive: () => JSON.stringify(streamFrames.shift()),
          close: () => {},
        },
      };
    },
    projectStatus: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_status", request });
      return JSON.stringify(request);
    },
    projectEvents: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_events", request });
      return JSON.stringify(request);
    },
  });

  const runDraft = await mission.buildRunEALInvocation({
    ...missionBase(),
    source: "mission weather\nlet r = local.observe_health()",
    label: "weather",
  });
  const fileDraft = await mission.buildRunFileInvocation({
    ...missionBase(),
    path: "/tmp/easynet-sdk-demo.eal",
    label: "file-weather",
  });
  const trackDraft = await mission.buildTrackInvocation({ ...missionBase(), mission_id: missionID });
  const cancelDraft = await mission.buildCancelInvocation({ ...missionBase(), mission_id: missionID });
  const eventsDraft = await mission.buildEventsInvocation({
    ...missionBase(),
    mission_id: missionID,
    cursor_sequence: 4,
    limit: 100,
  });
  const run = await mission.runEAL({
    ...missionBase(),
    source: "mission weather\nlet r = local.observe_health()",
    label: "weather",
  });
  const runFile = await mission.runFile({
    ...missionBase(),
    path: "/tmp/easynet-sdk-demo.eal",
    label: "file-weather",
  });
  const tracked = await mission.track({ ...missionBase(), mission_id: missionID });
  const cancelled = await mission.cancel({ ...missionBase(), mission_id: missionID });
  const page = await mission.events({ ...missionBase(), mission_id: missionID, cursor_sequence: 4, limit: 100 });
  const stream = await mission.openEventStream({
    ...missionBase(),
    mission_id: missionID,
    cursor_sequence: 4,
    limit: 100,
  });
  const streamed = [];
  for await (const event of stream.events()) {
    streamed.push(event);
  }
  const projectedStatus = await mission.projectStatus(tracked);
  const projectedEvents = await mission.projectEvents(page.toJSON());

  assert.equal(runDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0");
  assert.equal(fileDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0");
  assert.equal(trackDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0");
  assert.equal(cancelDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0");
  assert.equal(eventsDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0");
  assert.equal(run.status.missionID, missionID);
  assert.equal(runFile.status.outputRefs[0].kind, "run_dir");
  assert.equal(tracked.childReceipts[0].receipt_ura, "easynet:///r/example/receipt/child");
  assert.equal(cancelled.cancelled, true);
  assert.equal(page.events.length, 2);
  assert.equal(page.events[1].terminal, true);
  assert.equal(streamed.length, 2);
  assert.equal(streamed[1].terminal, true);
  assert.equal(projectedStatus.state, "partial");
  assert.equal(projectedEvents.nextCursorSequence, 7);
  assert.equal(seen[0].request.source, "mission weather\nlet r = local.observe_health()");
  assert.equal(seen[1].request.path, "/tmp/easynet-sdk-demo.eal");
  assert.equal(seen[4].request.limit, 100);
  assert.equal(seen.at(-1).method, "project_events");

  await assert.rejects(
    () => mission.buildTrackInvocation({ ...missionBase(), mission_id: "../bad" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => mission.buildRunFileInvocation({ ...missionBase(), path: "relative.eal" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => new MissionClient({
      buildRunEALInvocation: () => completeDraft().build().toJSONString(),
      projectStatus: () => JSON.stringify({ ...missionStatus(), parent_receipt_ura: null }),
    }).projectStatus({
      ...missionStatus(),
      parent_receipt_ura: null,
    }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("AdminClient delegates gateway carriers and projections without backend onboarding policy", async () => {
  const seen = [];
  const admin = new AdminClient({
    buildAgentListInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_agent_list", request });
      return adminDraftJSON("easynet:///r/example/ability/device.dev-a.agent.list@1.0.0");
    },
    buildAgentStartInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_agent_start", request });
      return adminDraftJSON("easynet:///r/example/ability/device.dev-a.agent.start@1.0.0", {
        name: request.name,
        agent_type: request.agent_type,
        model: request.model,
        label: request.label,
      });
    },
    buildAgentStopInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_agent_stop", request });
      return adminDraftJSON("easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0", {
        name: request.name,
      });
    },
    buildAgentRefreshInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_agent_refresh", request });
      return adminDraftJSON("easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0", {
        name: request.name,
      });
    },
    buildSessionListInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_session_list", request });
      return adminDraftJSON("easynet:///r/example/ability/device.dev-a.session.list@1.0.0", {
        include_terminated: request.include_terminated,
      });
    },
    gatewayStatus: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "gateway_status", request });
      return JSON.stringify(gatewayStatus());
    },
    listAgents: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list_agents", request });
      return JSON.stringify(adminAgentRecords());
    },
    startAgent: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "start_agent", request });
      return JSON.stringify(adminLifecycleResult());
    },
    stopAgent: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "stop_agent", request });
      return JSON.stringify(adminLifecycleResult({ operation: "agent.stop" }));
    },
    refreshAgent: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "refresh_agent", request });
      return JSON.stringify(adminLifecycleResult({ operation: "agent.refresh" }));
    },
    listSessions: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list_sessions", request });
      return JSON.stringify(deviceSessionPage());
    },
    pairingPreflight: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "pairing_preflight", request });
      return JSON.stringify(pairingPreflight());
    },
    createPairing: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "create_pairing", request });
      return JSON.stringify(pairingToken());
    },
    validatePairing: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "validate_pairing", request });
      return JSON.stringify(deviceCredential());
    },
    createDeviceSession: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "create_device_session", request });
      return JSON.stringify(deviceSession());
    },
    deleteDeviceSession: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "delete_device_session", request });
      return JSON.stringify(adminLifecycleResult({
        kind: "device_admin_result",
        operation: "session.delete",
        state: "deleted",
        agent_ura: null,
        device_ura: "easynet:///r/example/device/dev-a",
        ack: true,
      }));
    },
    projectGatewayStatus: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectAgentRecords: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectAgentLifecycleResult: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectPairingPreflight: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectPairingToken: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectDeviceCredential: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectDeviceSession: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectDeviceSessionPage: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
    projectDeviceAdminResult: (requestJSON) => JSON.stringify(JSON.parse(Buffer.from(requestJSON).toString("utf8"))),
  });

  const startRequest = {
    ...adminBase(),
    name: "codex",
    agent_type: "codex",
    model: "gpt-5",
    label: "primary",
  };
  const stopRequest = { ...adminBase(), name: "codex" };
  const pairingRequest = {
    ...adminBase(),
    hub_ura: "easynet:///r/example/hub/main",
    device_ura: "easynet:///r/example/device/dev-a",
  };

  const agentListDraft = await admin.buildAgentListInvocation(adminBase());
  const agentStartDraft = await admin.buildAgentStartInvocation(startRequest);
  const agentStopDraft = await admin.buildAgentStopInvocation(stopRequest);
  const agentRefreshDraft = await admin.buildAgentRefreshInvocation(stopRequest);
  const sessionListDraft = await admin.buildSessionListInvocation({ ...adminBase(), include_terminated: false });
  const status = await admin.gatewayStatus({ require_public_listener: false });
  const agents = await admin.listAgents(adminBase());
  const start = await admin.startAgent(startRequest);
  const stop = await admin.stopAgent(stopRequest);
  const refresh = await admin.refreshAgent(stopRequest);
  const sessions = await admin.listSessions({ ...adminBase(), include_terminated: false });
  const preflight = await admin.pairingPreflight({ ...pairingRequest, requested_scopes: ["invoke", "events"] });
  const token = await admin.createPairing({
    ...pairingRequest,
    expires_unix_ms: 1893456000000,
    scopes: ["invoke", "events"],
  });
  const credential = await admin.validatePairing({
    ...adminBase(),
    token: "pair-token-value",
    device_ura: "easynet:///r/example/device/dev-a",
  });
  const createdSession = await admin.createDeviceSession({
    ...pairingRequest,
    session_kind: "remote_desktop",
    expires_unix_ms: 1893456000000,
  });
  const listedSessions = await admin.listDeviceSessions({ ...adminBase(), include_terminated: false });
  const deletedSession = await admin.deleteDeviceSession({
    ...adminBase(),
    session_id: "dev-session-1",
    reason: "done",
  });
  const projectedStatus = await admin.projectGatewayStatus(status);
  const projectedAgents = await admin.projectAgentRecords(agents);
  const projectedLifecycle = await admin.projectAgentLifecycleResult(start);
  const projectedPreflight = await admin.projectPairingPreflight(preflight);
  const projectedToken = await admin.projectPairingToken(token);
  const projectedCredential = await admin.projectDeviceCredential(credential);
  const projectedSession = await admin.projectDeviceSession(createdSession);
  const projectedSessionPage = await admin.projectDeviceSessionPage(sessions);
  const projectedDelete = await admin.projectDeviceAdminResult(deletedSession);

  assert.equal(agentListDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0");
  assert.equal(agentStartDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0");
  assert.equal(agentStopDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0");
  assert.equal(agentRefreshDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0");
  assert.equal(sessionListDraft.descriptorRef, "easynet:///r/example/ability/device.dev-a.session.list@1.0.0");
  assert.equal(status.ready, true);
  assert.equal(status.publicListenerReady, false);
  assert.equal(agents.items[0].name, "codex");
  assert.equal(start.operation, "agent.start");
  assert.equal(stop.operation, "agent.stop");
  assert.equal(refresh.operation, "agent.refresh");
  assert.equal(sessions.items[0].sessionKind, "remote_desktop");
  assert.equal(preflight.pairingRequired, true);
  assert.equal(token.tokenID, "pair-token-1");
  assert.equal(credential.credentialID, "cred-dev-a");
  assert.equal(createdSession.sessionID, "dev-session-1");
  assert.equal(listedSessions.items.length, 1);
  assert.equal(deletedSession.state, "deleted");
  assert.equal(projectedStatus.gatewayID, "device:example:dev-a");
  assert.equal(projectedAgents.items[0].runtime, "codex");
  assert.equal(projectedLifecycle.agentURA, "easynet:///r/example/agent/alice.codex");
  assert.equal(projectedPreflight.trustReady, false);
  assert.equal(projectedToken.scopes[0], "invoke");
  assert.equal(projectedCredential.state, "active");
  assert.equal(projectedSession.hubURA, "easynet:///r/example/hub/main");
  assert.equal(projectedSessionPage.items[0].deviceURA, "easynet:///r/example/device/dev-a");
  assert.equal(projectedDelete.ack, true);
  assert.equal(seen[1].request.name, "codex");
  assert.equal(seen[4].request.include_terminated, false);
  assert.equal(seen[12].request.expires_unix_ms, 1893456000000);

  await assert.rejects(
    () => admin.buildAgentStartInvocation({ ...startRequest, name: "system" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => admin.buildAgentStartInvocation({ ...adminBase(), name: "codex" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("ReceiptClient delegates fetch, projection, and causal refs without verification claims", async () => {
  const seen = [];
  const receipt = new ReceiptClient({
    fetch: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "fetch", request });
      return JSON.stringify({
        receipt_ura: request.invocation_ura,
        invocation_id: "inv-1",
        state: "completed",
        verified: false,
        metadata: { profile: "receipt" },
      });
    },
    project: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "project", request });
      return JSON.stringify({
        receipt_ura: request.receipt_ura,
        invocation_id: request.invocation_id,
        state: "completed",
        verified: false,
        metadata: {},
      });
    },
    verify: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "verify", request });
      return JSON.stringify({
        verified: false,
        receipt_ura: request.receipt_ura,
        method: "provider_required",
        reason: "summary_only",
        metadata: {},
      });
    },
    causalRef: (receiptJSON) => {
      const request = JSON.parse(Buffer.from(receiptJSON).toString("utf8"));
      seen.push({ method: "causal", request });
      return JSON.stringify({
        causal_ref: `receipt:${request.receipt_ura}`,
        receipt_ura: request.receipt_ura,
        receipt_hash_hex: request.receipt_hash_hex,
        causal_context: {
          form: "receipt",
          receipt_ura: request.receipt_ura,
          receipt_hash_hex: request.receipt_hash_hex,
        },
        verified: false,
        metadata: {},
      });
    },
  });

  const fetched = await receipt.fetch(receiptFetch());
  assert.equal(fetched.verified, false);
  assert.equal(
    seen[0].request.descriptor_ref,
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );

  const ref = new ReceiptRef({
    receipt_ura: "easynet:///r/example/resource/receipt.inv-1",
    receipt_hash_hex: receiptHash,
    invocation_id: "inv-1",
  });
  const projected = await receipt.project(ref.toJSON());
  const verified = await receipt.verify(ref.toJSON());
  const causal = await ref.causalContext(receipt);

  assert.equal(projected.verified, false);
  assert.equal(verified.verified, false);
  assert.equal(causal.receipt_hash_hex, receiptHash);
  assert.deepEqual(seen.map((item) => item.method), ["fetch", "project", "verify", "causal"]);

  await assert.rejects(
    () => receipt.fetch({ ...receiptFetch(), descriptor_ref: "" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("ReceiptClient delegates carriers, history, and chain verification", async () => {
  const seen = [];
  const draftJSON = completeDraft().build().toJSONString();
  const receipt = new ReceiptClient({
    fetch: () => JSON.stringify({ verified: false, metadata: {} }),
    buildFetchInvocation: (requestJSON) => {
      seen.push({ method: "build_fetch", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return draftJSON;
    },
    buildListHistoryInvocation: (requestJSON) => {
      seen.push({ method: "build_list", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return draftJSON;
    },
    listHistory: (requestJSON) => {
      seen.push({ method: "list", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return JSON.stringify({ profile: "receipt", items: [] });
    },
    getTrace: (requestJSON) => {
      seen.push({ method: "trace", request: JSON.parse(Buffer.from(requestJSON).toString("utf8")) });
      return JSON.stringify({ profile: "receipt", trace: [] });
    },
    verifyChain: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "chain", request });
      return JSON.stringify({
        verified: false,
        continuous: true,
        method: "provider_projection",
        receipt_count: request.receipts.length,
        metadata: request.metadata ?? {},
      });
    },
  });

  const built = await receipt.buildFetchInvocation(receiptFetch());
  await receipt.buildListHistoryInvocation({ ...directoryBase(), arguments: { limit: 1 } });
  const listed = await receipt.listHistory({ ...directoryBase(), arguments: { limit: 1 } });
  const trace = await receipt.getTrace({ ...directoryBase(), arguments: { invocation_id: "inv-1" } });
  const chain = new ReceiptChain([
    { receipt_ura: "easynet:///r/example/resource/receipt.inv-1", receipt_hash_hex: receiptHash },
  ]);
  const verification = await chain.verifyContinuity(receipt, { source: "test" });

  assert.equal(built.descriptorRef, "opaque-descriptor-ref-from-identity-profile");
  assert.equal(
    seen[0].request.descriptor_ref,
    "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
  );
  assert.equal(listed.profile, "receipt");
  assert.equal(trace.profile, "receipt");
  assert.equal(verification.receipt_count, 1);
  assert.equal(seen.at(-1).request.receipts[0].receipt_hash_hex, receiptHash);
});

test("ReceiptRef rejects fabricated or malformed receipt anchors", () => {
  assert.throws(
    () => new ReceiptRef({ receipt_hash_hex: receiptHash }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  assert.throws(
    () => new ReceiptRef({ receipt_ura: "easynet:///r/example/resource/receipt.inv-1", receipt_hash_hex: "abc" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("SurfaceClient delegates page carriers and daemon projections without rendering policy", async () => {
  const seen = [];
  const surface = new SurfaceClient({
    buildListPagesInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_list", request });
      return surfaceDraftJSON("easynet:///r/example/ability/alice.pages.pages.list@1.0.0");
    },
    buildCreatePageInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_create", request });
      return surfaceDraftJSON(
        "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0",
        { project_id: request.project_id, folder: request.folder, visibility: request.visibility },
      );
    },
    buildDeletePageInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_delete", request });
      return surfaceDraftJSON(
        "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0",
        { project_id: request.project_id },
      );
    },
    buildManifestInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_manifest", request });
      return surfaceDraftJSON(
        "easynet:///r/example/ability/alice.pages.pages.get@1.0.0",
        { project_id: request.project_id },
      );
    },
    buildHealthInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_health", request });
      return surfaceDraftJSON(
        "easynet:///r/example/ability/alice.pages.pages.health@1.0.0",
        { surface_ref: request.surface_ref },
      );
    },
    listPages: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list", request });
      return JSON.stringify({
        profile: "surface",
        kind: "surface_page_page",
        item_kind: "page_record",
        items: [surfacePageRecord()],
        next_cursor: null,
        limit: request.limit,
        source: "pages_read_model",
        metadata: {
          profile: "surface",
          source_ability: "pages.list",
        },
      });
    },
    createPage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "create", request });
      return JSON.stringify(surfacePageRecord({ page_id: request.project_id }));
    },
    deletePage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "delete", request });
      return JSON.stringify({
        profile: "surface",
        kind: "surface_mutation_result",
        operation: "delete",
        page_id: request.project_id,
        removed: true,
        state: "deleted",
        metadata: { profile: "surface", source_ability: "pages.unpublish" },
      });
    },
    surfaceManifest: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "manifest", request });
      return JSON.stringify(surfaceManifest());
    },
    publicPageRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "public_ref", request });
      return JSON.stringify({
        profile: "surface",
        kind: "public_page_ref",
        page_id: request.page.page_id,
        owner_ura: request.page.owner_ura,
        surface_ref: request.page.surface_ref,
        public_ref: request.page.public_ref,
        route_kind: "hub_web",
        metadata: { profile: "surface", source_ability: "pages.get" },
      });
    },
    surfaceHealth: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "health", request });
      return JSON.stringify(surfaceHealth());
    },
    projectPagePage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_page", request });
      return JSON.stringify(request);
    },
    projectManifest: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_manifest", request });
      return JSON.stringify(request);
    },
    projectHealth: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_health", request });
      return JSON.stringify(request);
    },
  });

  const listDraft = await surface.buildListPagesInvocation({ ...surfaceBase(), limit: 50 });
  const createDraft = await surface.buildCreatePageInvocation({
    ...surfaceBase(),
    project_id: "docs",
    folder: "/tmp/easynet-pages-docs",
    visibility: "public",
  });
  const deleteDraft = await surface.buildDeletePageInvocation({
    ...surfaceBase(),
    project_id: "docs",
  });
  const manifestDraft = await surface.buildManifestInvocation({
    ...surfaceBase(),
    project_id: "docs",
  });
  const healthDraft = await surface.buildHealthInvocation({
    ...surfaceBase(),
    surface_ref: "easynet:///r/example/resource/alice.docs",
  });
  const page = await surface.listPages(surfaceBase());
  const record = await surface.createPage({
    ...surfaceBase(),
    project_id: "docs",
    folder: "/tmp/easynet-pages-docs",
    visibility: "public",
  });
  const mutation = await surface.deletePage({ ...surfaceBase(), project_id: "docs" });
  const manifest = await surface.surfaceManifest({ ...surfaceBase(), project_id: "docs" });
  const ref = await surface.publicPageRef({ page: record });
  const health = await surface.surfaceHealth({
    ...surfaceBase(),
    surface_ref: "easynet:///r/example/resource/alice.docs",
  });
  const status = await surface.surfaceStatus({
    ...surfaceBase(),
    surface_ref: "easynet:///r/example/resource/alice.docs",
  });
  const projectedPage = await surface.projectPagePage(page);
  const projectedManifest = await surface.projectManifest(manifest.toJSON());
  const projectedHealth = await surface.projectStatus(health);

  assert.equal(listDraft.descriptorRef, "easynet:///r/example/ability/alice.pages.pages.list@1.0.0");
  assert.equal(createDraft.descriptorRef, "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0");
  assert.equal(deleteDraft.descriptorRef, "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0");
  assert.equal(manifestDraft.descriptorRef, "easynet:///r/example/ability/alice.pages.pages.get@1.0.0");
  assert.equal(healthDraft.descriptorRef, "easynet:///r/example/ability/alice.pages.pages.health@1.0.0");
  assert.equal(page.limit, DEFAULT_SURFACE_PAGE_SIZE);
  assert.equal(page.items[0].pageId, "docs");
  assert.equal(record.surfaceRef, "easynet:///r/example/resource/alice.docs");
  assert.equal(mutation.state, "deleted");
  assert.equal(manifest.page.pageId, "docs");
  assert.equal(ref.routeKind, "hub_web");
  assert.equal(health.ready, true);
  assert.equal(status instanceof SurfaceStatus, true);
  assert.equal(projectedPage.source, "pages_read_model");
  assert.equal(projectedManifest.kind, "surface_manifest");
  assert.equal(projectedHealth.descriptorVersion, "1.0.0");
  assert.equal(seen[0].request.limit, 50);
  assert.equal(seen[1].request.folder, "/tmp/easynet-pages-docs");
  assert.equal(seen[4].request.surface_ref, "easynet:///r/example/resource/alice.docs");
  assert.equal(seen[5].request.limit, DEFAULT_SURFACE_PAGE_SIZE);

  await assert.rejects(
    () => surface.buildCreatePageInvocation({
      ...surfaceBase(),
      project_id: "../docs",
      folder: "/tmp/easynet-pages-docs",
    }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => surface.buildCreatePageInvocation({
      ...surfaceBase(),
      project_id: "docs",
      folder: "relative",
    }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => surface.listPages({ ...surfaceBase(), limit: 501 }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("CompatibilityClient delegates OpenAI-compatible carriers and daemon projections without product HTTP policy", async () => {
  const seen = [];
  const compatibility = new CompatibilityClient({
    buildListModelsInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_list_models", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.list_models@1.0.0");
    },
    buildChatCompletionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_chat", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.chat_completions@1.0.0", request.request);
    },
    buildStreamChatCompletionInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_stream_chat", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.chat_completions.stream@1.0.0", request.request);
    },
    buildFileUploadInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_file_upload", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.files.upload@1.0.0", request);
    },
    buildFileRetrieveInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_file_retrieve", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.files.retrieve@1.0.0", request);
    },
    buildFileDeleteInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "build_file_delete", request });
      return compatibilityDraftJSON("easynet:///r/example/ability/openai.files.delete@1.0.0", request);
    },
    listModels: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list_models", request });
      return JSON.stringify(compatibilityModelPage());
    },
    chatCompletions: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "chat", request });
      return JSON.stringify(compatibilityChatCompletion());
    },
    streamChatCompletions: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "stream_chat", request });
      return JSON.stringify(compatibilityChatStream());
    },
    uploadFile: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "file_upload", request });
      return JSON.stringify(compatibilityFile());
    },
    getFile: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "file_get", request });
      return JSON.stringify(compatibilityFile());
    },
    deleteFile: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "file_delete", request });
      return JSON.stringify(compatibilityFileDeleteResult());
    },
    projectModelPage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_model_page", request });
      return JSON.stringify(request);
    },
    projectChatCompletion: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_chat", request });
      return JSON.stringify(request);
    },
    projectChatStream: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_stream", request });
      return JSON.stringify(request);
    },
    projectFileUpload: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_file_upload", request });
      return JSON.stringify(compatibilityFile());
    },
    projectFile: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_file", request });
      return JSON.stringify(request);
    },
    projectFileDeleteResult: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "project_file_delete", request });
      return JSON.stringify(request);
    },
  });

  const listDraft = await compatibility.buildListModelsInvocation(compatibilityBase());
  const chatDraft = await compatibility.buildChatCompletionInvocation(compatibilityChatRequest());
  const streamDraft = await compatibility.buildStreamChatCompletionInvocation(compatibilityChatRequest());
  const uploadDraft = await compatibility.buildFileUploadInvocation(compatibilityFileRequest());
  const retrieveDraft = await compatibility.buildFileRetrieveInvocation({
    ...compatibilityBase(),
    id: "file-easynet-docs-1",
  });
  const deleteDraft = await compatibility.buildFileDeleteInvocation({
    ...compatibilityBase(),
    id: "file-easynet-docs-1",
    deleted: true,
  });
  const models = await compatibility.listModels(compatibilityBase());
  const completion = await compatibility.chatCompletions(compatibilityChatRequest());
  const stream = await compatibility.streamChatCompletions(compatibilityChatRequest());
  const uploaded = await compatibility.uploadFile(compatibilityFileRequest());
  const file = await compatibility.getFile({ ...compatibilityBase(), id: "file-easynet-docs-1" });
  const deleted = await compatibility.deleteFile({
    ...compatibilityBase(),
    id: "file-easynet-docs-1",
    deleted: true,
  });
  const projectedModels = await compatibility.projectModelPage(models);
  const projectedCompletion = await compatibility.projectChatCompletion(completion.toJSON());
  const projectedStream = await compatibility.projectChatStream(stream);
  const projectedUpload = await compatibility.projectFileUpload(compatibilityFileRequest());
  const projectedFile = await compatibility.projectFile(file.toJSON());
  const projectedDelete = await compatibility.projectFileDeleteResult(deleted);

  assert.equal(listDraft.descriptorRef, "easynet:///r/example/ability/openai.list_models@1.0.0");
  assert.equal(chatDraft.descriptorRef, "easynet:///r/example/ability/openai.chat_completions@1.0.0");
  assert.equal(streamDraft.descriptorRef, "easynet:///r/example/ability/openai.chat_completions.stream@1.0.0");
  assert.equal(uploadDraft.descriptorRef, "easynet:///r/example/ability/openai.files.upload@1.0.0");
  assert.equal(retrieveDraft.descriptorRef, "easynet:///r/example/ability/openai.files.retrieve@1.0.0");
  assert.equal(deleteDraft.descriptorRef, "easynet:///r/example/ability/openai.files.delete@1.0.0");
  assert.equal(models.data[0].id, compatibilityAbilityURA);
  assert.equal(completion.object, "chat.completion");
  assert.equal(stream.stream, true);
  assert.equal(stream.items[0].object, "chat.completion.chunk");
  assert.equal(uploaded.id, "file-easynet-docs-1");
  assert.equal(file.createdAt, 1783094400);
  assert.equal(deleted.deleted, true);
  assert.equal(projectedModels.object, "list");
  assert.equal(projectedCompletion.model, compatibilityAbilityURA);
  assert.equal(projectedStream.doneSentinel, "[DONE]");
  assert.equal(projectedUpload.status, "processed");
  assert.equal(projectedFile.metadata.file_ref, "easynet:///r/example/resource/alice.files/prompt.jsonl");
  assert.equal(projectedDelete.id, "file-easynet-docs-1");
  assert.equal(seen[2].request.request.stream, true);
  assert.equal(seen[3].request.file_ref, "easynet:///r/example/resource/alice.files/prompt.jsonl");
  assert.deepEqual(
    seen
      .filter((entry) => entry.method.startsWith("project_"))
      .map((entry) => entry.method),
    [
      "project_model_page",
      "project_chat",
      "project_stream",
      "project_file_upload",
      "project_file",
      "project_file_delete",
    ],
  );

  await assert.rejects(
    () => compatibility.buildChatCompletionInvocation(compatibilityChatRequest({ stream: true })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () => compatibility.buildChatCompletionInvocation(compatibilityChatRequest({ model: "gpt-4o" })),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("PublicationClient delegates resource, package, deploy, and unpublish carriers", async () => {
  const seen = [];
  const publication = new PublicationClient({
    buildResourceRef: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "resource", request });
      return JSON.stringify(publicationResourceRef());
    },
    validatePackage: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "validate", request });
      return JSON.stringify({
        profile: "publication",
        kind: "package_validation",
        valid: true,
        package_path: request.package_path,
        manifest_path: `${request.package_path}/ability.json`,
        manifest_hash: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        manifest: {
          name: "weather",
          namespace: "er",
          wire_key: "er.weather",
          descriptor_version: "1.0.0",
          description: "",
          exec_kind: "host_stream",
          timeout_seconds: null,
          input_schema: {},
          output_schema: null,
        },
        errors: [],
        metadata: {},
      });
    },
    buildDeployInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "deploy_carrier", request });
      return publicationDraftJSON("easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0");
    },
    buildUnpublishInvocation: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "unpublish_carrier", request });
      return publicationDraftJSON("easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0");
    },
  });

  const ref = await publication.buildLocalResourceRef({
    path: "/tmp/easynet/pkg",
    capability: "read",
  });
  const validation = await publication.validatePackage({ package_path: "/tmp/easynet/pkg" });
  const deploy = await publication.buildDeployInvocation(publicationDeploy());
  const unpublish = await publication.buildUnpublishInvocation({
    ...directoryBase(),
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
    metadata: { request_id: "unpublish-1" },
  });

  assert.equal(ref.resource_ura, "easynet:///r/example/resource/fs.local.pkg");
  assert.equal(validation.valid, true);
  assert.equal(deploy.descriptorRef, "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0");
  assert.equal(unpublish.descriptorRef, "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0");
  assert.deepEqual(seen.map((item) => item.method), [
    "resource",
    "validate",
    "deploy_carrier",
    "unpublish_carrier",
  ]);
  assert.equal(seen[2].request.resource_ref.resource_ura, ref.resource_ura);
});

test("PublicationClient delegates read models and lifecycle projections", async () => {
  const seen = [];
  const publication = new PublicationClient({
    buildResourceRef: () => JSON.stringify(publicationResourceRef()),
    listAbilities: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "list", request });
      return JSON.stringify({
        profile: "publication",
        kind: "published_ability_page",
        item_kind: "published_ability",
        items: [],
        next_cursor: null,
        limit: request.limit,
        source: "daemon_read_model",
        metadata: {},
      });
    },
    showAbility: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "show", request });
      return JSON.stringify({
        descriptor: { descriptor_ref: request.descriptor_ref },
        implementation: {},
        metadata: {},
      });
    },
    installPlugin: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "install", request });
      return JSON.stringify({
        profile: "publication",
        kind: "plugin_install_result",
        source: request.source,
        install_id: "install-1",
        status: "installed",
        metadata: {},
      });
    },
    enableAbilityImpl: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "enable", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_impl_enabled",
        status: "enabled",
        metadata: {},
      });
    },
    disableAbilityImpl: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "disable", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_impl_disabled",
        status: "disabled",
        metadata: {},
      });
    },
    unpublishAbility: (requestJSON) => {
      const request = JSON.parse(Buffer.from(requestJSON).toString("utf8"));
      seen.push({ method: "unpublish", request });
      return JSON.stringify({
        profile: "publication",
        kind: "ability_unpublished",
        status: "unpublished",
        metadata: {},
      });
    },
  });

  const page = await publication.listAbilities({ ...directoryBase() });
  const ability = await publication.showAbility({
    descriptor_ref: "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
  });
  const installed = await publication.installPlugin({ source: "/tmp/easynet/plugin" });
  const enabled = await publication.enableAbilityImpl({
    impl_id: "impl-1",
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });
  const disabled = await publication.disableAbilityImpl({
    impl_id: "impl-1",
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });
  const unpublished = await publication.unpublishAbility({
    ...directoryBase(),
    ability_ura: "easynet:///r/example/ability/device.dev-a.er.weather",
  });

  assert.equal(page.limit, 50);
  assert.equal(ability.descriptor.descriptor_ref, "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0");
  assert.equal(installed.status, "installed");
  assert.equal(enabled.status, "enabled");
  assert.equal(disabled.status, "disabled");
  assert.equal(unpublished.status, "unpublished");
  assert.deepEqual(seen.map((item) => item.method), [
    "list",
    "show",
    "install",
    "enable",
    "disable",
    "unpublish",
  ]);
});

test("PublicationClient rejects incomplete carriers and local resource fabrication", async () => {
  const publication = new PublicationClient({
    buildResourceRef: () => JSON.stringify(publicationResourceRef()),
  });

  await assert.rejects(
    () => publication.buildLocalResourceRef({ path: "relative/pkg", capability: "read" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
  await assert.rejects(
    () =>
      publication.buildDeployInvocation({
        ...publicationDeploy(),
        resource_ref: { ...publicationResourceRef(), namespace: "system" },
      }),
    (error) => error instanceof SDKError && error.source === "publication",
  );
  await assert.rejects(
    () => publication.buildUnpublishInvocation({ ...directoryBase() }),
    (error) => error instanceof SDKError && error.source === "publication",
  );

  await publication.close();
  await publication.close();
  await assert.rejects(
    () => publication.buildLocalResourceRef({ path: "/tmp/easynet/pkg", capability: "read" }),
    (error) => error instanceof SDKError && error.code === ErrorCode.INVALID_ARGUMENT,
  );
});

test("HostBindingClient local transport builds binding and codec frames", async () => {
  const calls = [];
  const transport = new LocalHostBindingTransport((descriptorRef) => {
    calls.push(`canonical:${descriptorRef}`);
    return descriptorRef;
  });
  const client = new HostBindingClient(transport);

  const binding = await client.buildHostStreamBinding(hostBindingRequest());
  const request = await client.decodeRequest({
    request: {
      fn: "weather.stream",
      args: { city: "Singapore" },
      call_id: "call-weather-1",
      caller: "easynet:///r/example/user/alice",
    },
  });
  const item = await client.encodeItem(0, { token: "hello" });
  const error = await client.encodeError(
    new SDKError({
      code: ErrorCode.GENERIC,
      stage: "host_binding",
      retry: "never",
      message: "boom",
      details: {},
    }),
  );
  const plainError = await client.encodeError(new Error("plain boom"));
  const terminal = await client.encodeTerminal({
    output_hash: "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15",
    frames: 1,
  });

  assert.equal(binding.binding_id, "binding-weather-1");
  assert.equal(binding.lifecycle.frame_contract_owner, "daemon_sdk");
  assert.equal(binding.metadata.hash_algorithm, HOST_STREAM_HASH_ALGORITHM);
  assert.equal(request.function, "weather.stream");
  assert.deepEqual(request.args, { city: "Singapore" });
  assert.equal(item.frame_type, "item");
  assert.equal(item.seq, 0);
  assert.equal(item.output_hash, null);
  assert.equal(error.frame_type, "error");
  assert.equal(error.error.code, ErrorCode.GENERIC);
  assert.equal(plainError.error.code, ErrorCode.GENERIC);
  assert.equal(plainError.error.message, "plain boom");
  assert.equal(terminal.frame_type, "terminal");
  assert.equal(terminal.output_hash, terminal.terminal.output_hash);
  assert.deepEqual(calls, ["canonical:easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0"]);
});

test("HostBindingClient folds output hash and rejects corrupted state", async () => {
  const client = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
  );
  const initial = HostStreamHashState.initial();
  assert.equal(initial.outputHash, HOST_STREAM_EMPTY_OUTPUT_HASH);

  const folded = await client.foldOutputHash(initial, 0, { token: "hello" });
  assert.equal(folded.outputHash, "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15");
  assert.equal(folded.canonicalJSON, "{\"token\":\"hello\"}");
  assert.equal(folded.lastSeq, 0);

  await assert.rejects(
    () => client.foldOutputHash(initial, 2, { token: "skip" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  assert.throws(
    () =>
      new HostStreamHashState({
        algorithm: HOST_STREAM_HASH_ALGORITHM,
        output_hash: HOST_STREAM_EMPTY_OUTPUT_HASH,
        frames: 0,
        last_seq: 0,
      }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  assert.throws(
    () =>
      new HostStreamHashState({
        algorithm: HOST_STREAM_HASH_ALGORITHM,
        output_hash: folded.toJSON().output_hash,
        frames: 3,
        last_seq: 0,
      }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
});

test("HostBinding lifecycle provider is explicit and cleanup is idempotent", async () => {
  const providerCalls = [];
  const provider = {
    checkReadiness(binding) {
      providerCalls.push(`readiness:${binding.binding_id}`);
      return { state: "ready", checked: true, endpoint_ready: true };
    },
    cleanup(binding) {
      providerCalls.push(`cleanup:${binding.binding_id}`);
      return { mode: "unlink_socket", cleaned: true };
    },
  };
  const client = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
    provider,
  );
  const binding = await client.buildHostStreamBinding(hostBindingRequest());
  const lifecycle = client.openLifecycle(binding);
  const readiness = await lifecycle.checkReadiness();
  const cleanup = await lifecycle.cleanup();
  const cleanupAgain = await lifecycle.cleanup();

  assert.equal(readiness.state, "ready");
  assert.equal(cleanup.mode, "unlink_socket");
  assert.equal(cleanupAgain, cleanup);
  assert.equal(lifecycle.state, "cleaned");
  assert.deepEqual(providerCalls, ["readiness:binding-weather-1", "cleanup:binding-weather-1"]);
  lifecycle.close();
  assert.equal(lifecycle.state, "closed");
});

test("HostBinding rejects descriptor, endpoint, schema, and hash drift", async () => {
  const client = new HostBindingClient(new LocalHostBindingTransport());
  await assert.rejects(
    () => client.buildHostStreamBinding(hostBindingRequest()),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  const canonicalClient = new HostBindingClient(
    new LocalHostBindingTransport((descriptorRef) => descriptorRef),
  );
  await assert.rejects(
    () => canonicalClient.buildHostStreamBinding({ ...hostBindingRequest(), endpoint: "relative.sock" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  await assert.rejects(
    () => canonicalClient.buildHostStreamBinding({ ...hostBindingRequest(), frame_schema: "drift.schema.json" }),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
  await assert.rejects(
    () =>
      canonicalClient.foldOutputHash(
        {
          algorithm: HOST_STREAM_HASH_ALGORITHM,
          output_hash: "sha256:ABC",
          frames: 0,
          last_seq: null,
        },
        0,
        { token: "hello" },
      ),
    (error) => error instanceof SDKError && error.source === "host_binding",
  );
});
