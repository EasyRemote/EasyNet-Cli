export type RetryHintValue = "never" | "safe" | "after_backoff" | "unknown";
export type ErrorClassValue =
  | "validation"
  | "handle"
  | "lifecycle"
  | "availability"
  | "permission"
  | "admission"
  | "routing"
  | "timeout"
  | "cancellation"
  | "protocol"
  | "version"
  | "control"
  | "unsupported"
  | "generic";

export declare const ErrorCode: Readonly<Record<string, string>>;
export declare const ErrorClass: Readonly<{
  VALIDATION: "validation";
  HANDLE: "handle";
  LIFECYCLE: "lifecycle";
  AVAILABILITY: "availability";
  PERMISSION: "permission";
  ADMISSION: "admission";
  ROUTING: "routing";
  TIMEOUT: "timeout";
  CANCELLATION: "cancellation";
  PROTOCOL: "protocol";
  VERSION: "version";
  CONTROL: "control";
  UNSUPPORTED: "unsupported";
  GENERIC: "generic";
}>;
export declare const RetryHint: Readonly<{
  NEVER: "never";
  SAFE: "safe";
  AFTER_BACKOFF: "after_backoff";
  UNKNOWN: "unknown";
}>;
export declare const DEFAULT_DIRECTORY_PAGE_SIZE: 50;
export declare const MAX_DIRECTORY_PAGE_SIZE: 500;
export declare const DIRECTORY_IDENTITY_PROFILE: "directory_identity";
export declare const RECEIPT_PROFILE: "receipt";
export declare const PUBLICATION_PROFILE: "publication";
export declare const DEFAULT_PUBLISHED_ABILITY_PAGE_SIZE: 50;
export declare const MAX_PUBLISHED_ABILITY_PAGE_SIZE: 500;
export declare const HOST_BINDING_PROFILE: "host_binding";
export declare const HEALTH_PROFILE: "health";
export declare const EVENTS_PROFILE: "events";
export declare const SURFACE_PROFILE: "surface";
export declare const COMPATIBILITY_PROFILE: "compatibility";
export declare const MAX_STREAM_BUFFERED_EVENTS: 1024;
export declare const MAX_BIDI_BUFFERED_FRAMES: 1024;
export declare const DEFAULT_EVENT_PAGE_SIZE: 50;
export declare const MAX_EVENT_PAGE_SIZE: 500;
export declare const MIN_EVENT_HEARTBEAT_INTERVAL_MS: 1000;
export declare const MAX_EVENT_HEARTBEAT_INTERVAL_MS: 300000;
export declare const DEFAULT_SURFACE_PAGE_SIZE: 50;
export declare const MAX_SURFACE_PAGE_SIZE: 500;
export declare const HOST_STREAM_FRAME_SCHEMA: "host-stream-frame.schema.json";
export declare const HOST_STREAM_HASH_ALGORITHM: "sha256(prev_hash || seq_be || canonical_json(value))";
export declare const HOST_STREAM_EMPTY_OUTPUT_HASH: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

export declare function profileSourceRef(profile: string): string;
export declare function profileErrorDetails(profile: string, details?: Record<string, unknown>): Record<string, unknown>;

export interface SDKErrorOptions {
  code: string;
  stage: string;
  retry?: RetryHintValue;
  retryable?: boolean;
  message: string;
  source?: string;
  invocationId?: string;
  receiptURA?: string;
  details?: Record<string, unknown>;
  cause?: unknown;
}

export class SDKError extends Error {
  code: string;
  stage: string;
  retry: RetryHintValue;
  retryable: boolean;
  source: string;
  invocationId: string;
  receiptURA: string;
  details: Record<string, unknown>;
  constructor(options: SDKErrorOptions);
  static fromJSON(raw: Uint8Array | string): SDKError | null;
  errorClass(): ErrorClassValue;
  profile(): string;
  sourceRef(): string;
}

export interface DiscoveryTransport {
  featureDiscovery(): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export interface FeatureSet {
  abiVersion: number;
  sdkVersion: string;
  profiles: Record<string, string>;
  symbols: Record<string, boolean>;
  axonPB: boolean;
  version(): { abiVersion: number; sdkVersion: string };
}

export class Client {
  constructor(transport: DiscoveryTransport);
  featureDiscovery(): Promise<FeatureSet>;
  requireABI(expected: number): Promise<FeatureSet>;
  close(): Promise<void>;
}

export interface RuntimeHealthFields {
  api_ready: boolean;
  daemon_ready: boolean;
  invocation_ready: boolean;
  directory_ready: boolean;
  trust_ready: boolean;
  runtime_ready: boolean;
  version?: string | null;
  abi_version?: number | null;
  mismatch?: Record<string, unknown> | null;
  diagnostics?: string[];
}

export class RuntimeHealth {
  apiReady: boolean;
  daemonReady: boolean;
  invocationReady: boolean;
  directoryReady: boolean;
  trustReady: boolean;
  runtimeReady: boolean;
  version: string | null;
  abiVersion: number | null;
  mismatch: Record<string, unknown> | null;
  diagnostics: string[];
  constructor(fields: RuntimeHealthFields);
  static fromJSON(raw: Uint8Array | string): RuntimeHealth;
  apiAlive(): boolean;
  ready(): boolean;
  toJSON(): Required<RuntimeHealthFields>;
}

export interface DiagnosticCheckFields {
  name: string;
  ready: boolean;
  message?: string | null;
}

export class DiagnosticCheck {
  name: string;
  ready: boolean;
  message: string | null;
  constructor(fields: DiagnosticCheckFields);
  toJSON(): Required<DiagnosticCheckFields>;
}

export interface DiagnosticsReportFields {
  profile: "health";
  kind: "diagnostics_report";
  state: string;
  ready: boolean;
  version: string;
  abi_version: number;
  control_endpoint: string;
  invocation_endpoint?: string | null;
  checks: DiagnosticCheckFields[];
  diagnostics?: string[];
}

export class DiagnosticsReport {
  profile: "health";
  kind: "diagnostics_report";
  state: string;
  ready: boolean;
  version: string;
  abiVersion: number;
  controlEndpoint: string;
  invocationEndpoint: string | null;
  checks: DiagnosticCheck[];
  diagnostics: string[];
  constructor(fields: DiagnosticsReportFields);
  static fromJSON(raw: Uint8Array | string): DiagnosticsReport;
  toJSON(): Required<DiagnosticsReportFields>;
}

export interface HealthTransport {
  runtimeHealth(): Promise<Uint8Array | string> | Uint8Array | string;
  runtimeDiagnostics?(): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class HealthClient {
  constructor(transport: HealthTransport);
  runtimeHealth(): Promise<RuntimeHealth>;
  diagnostics(): Promise<DiagnosticsReport>;
  close(): Promise<void>;
}

export interface IdentityTransport {
  projectDescriptorRef(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDescriptorRef?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  ownerAbilityURA?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  resourceURA?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export interface DescriptorRefRequest {
  descriptor_ref: string;
  metadata?: Record<string, unknown>;
}

export interface DescriptorRefBuildRequest {
  ability_ura: string;
  descriptor_version: string;
  metadata?: Record<string, unknown>;
}

export interface IdentityProjection {
  kind?: string;
  valid?: boolean;
  ura?: string;
  descriptor_ref?: string;
  ability_ura?: string;
  descriptor_version?: string;
  profile?: string;
  components?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export class IdentityClient {
  constructor(transport: IdentityTransport);
  projectDescriptorRef(request: DescriptorRefRequest): Promise<IdentityProjection>;
  buildDescriptorRef(request: DescriptorRefBuildRequest): Promise<IdentityProjection>;
  canonicalAbilityDescriptorRef(value: string, descriptorVersion?: string): Promise<string>;
  abilityURAFromDescriptorRef(descriptorRef: string): Promise<string>;
  ownerAbilityURA(ownerURA: string, abilityName: string): Promise<string>;
  ownerAbilityDescriptorRef(ownerURA: string, abilityName: string, descriptorVersion: string): Promise<string>;
  resourceURA(ownerURA: string, path: string): Promise<string>;
  close(): Promise<void>;
}

export interface DirectoryQueryBase {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  limit?: number;
  cursor?: string;
  metadata?: Record<string, unknown>;
}

export interface ResolveQuery extends DirectoryQueryBase {
  query_name?: string;
  ability_name?: string;
  qtype?: string;
  realm_hint?: string;
  peer_hub_urls?: string[];
}

export interface DeviceQuery extends DirectoryQueryBase {}
export interface AgentQuery extends DirectoryQueryBase {}

export interface AbilityQuery extends DirectoryQueryBase {
  scope?: string;
  owner_ura?: string;
  ability_ura?: string;
}

export interface DirectorySubscriptionCursor {
  stream: string;
  sequence: number;
  token?: string;
}

export interface DirectorySubscriptionRequest extends DirectoryQueryBase {
  stream?: "directory";
  realm?: string;
  owner_ura?: string;
  device_ura?: string;
  agent_ura?: string;
  ability_ura?: string;
  item_kind?: string;
  resume_cursor?: DirectorySubscriptionCursor;
  heartbeat_interval_ms?: number;
}

export interface DirectoryTransport {
  resolve(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listDevices?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listAgents?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listAbilities?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDirectorySubscriptionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  subscribeDirectory?(requestJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  close?(): Promise<void> | void;
}

export class DirectoryClient {
  constructor(transport: DirectoryTransport);
  resolve(query: ResolveQuery): Promise<Record<string, unknown>>;
  listDevices(query: DeviceQuery): Promise<Record<string, unknown>>;
  listAgents(query: AgentQuery): Promise<Record<string, unknown>>;
  listAbilities(query: AbilityQuery): Promise<Record<string, unknown>>;
  buildDirectorySubscriptionInvocation(request: DirectorySubscriptionRequest): Promise<Record<string, unknown>>;
  subscribeDirectory(request: DirectorySubscriptionRequest): Promise<StreamHandle>;
  close(): Promise<void>;
}

export type EventStreamKind = "directory" | "device" | "session" | "invocation";

export interface EventsCarrierBase {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface EventCursorFields {
  stream: EventStreamKind;
  sequence: number;
  token?: string;
}

export interface EventFilter {
  realm?: string;
  owner_ura?: string;
  device_ura?: string;
  agent_ura?: string;
  session_id?: string;
  invocation_id?: string;
}

export interface EventsSubscriptionRequest extends EventsCarrierBase {
  stream?: EventStreamKind;
  filter?: EventFilter;
  realm?: string;
  owner_ura?: string;
  device_ura?: string;
  agent_ura?: string;
  session_id?: string;
  session_ura?: string;
  invocation_id?: string;
  resume_cursor?: EventCursorFields;
  heartbeat_interval_ms?: number;
}

export type EventsDirectorySubscriptionRequest = EventsSubscriptionRequest;
export type EventsDeviceSubscriptionRequest = EventsSubscriptionRequest;
export type EventsSessionSubscriptionRequest = EventsSubscriptionRequest;
export type EventsInvocationSubscriptionRequest = EventsSubscriptionRequest;
export type DirectoryEventQuery = EventsDirectorySubscriptionRequest;
export type DeviceEventQuery = EventsDeviceSubscriptionRequest;
export type SessionEventQuery = EventsSessionSubscriptionRequest;
export type InvocationEventQuery = EventsInvocationSubscriptionRequest;

export interface EventsDeviceEventListRequest extends EventsCarrierBase {
  filter?: EventFilter;
  device_ura?: string;
  limit?: number;
  cursor?: string;
}

export interface EventProjectionInput {
  cursor: EventCursorFields;
  event: Record<string, unknown>;
  event_id?: string;
  resume_token?: string;
  tenant_ref?: unknown;
}

export interface EventDropReportInput {
  cursor: EventCursorFields;
  occurred_unix_ms: number;
  dropped_count: number;
  reconnect_after_ms?: number | null;
  reason?: string;
  event_id?: string;
  resume_token?: string;
  tenant_ref?: unknown;
}

export interface EventTerminalInput {
  cursor: EventCursorFields;
  occurred_unix_ms: number;
  reconnect_after_ms?: number | null;
  reason?: string;
  event_id?: string;
  resume_token?: string;
  tenant_ref?: unknown;
}

export interface EventFrameFields {
  profile: "events";
  stream: EventStreamKind;
  kind: string;
  event_id: string;
  cursor: EventCursorFields;
  resume_token: string;
  occurred_unix_ms: number;
  occurred_at: string;
  subject_ref: unknown;
  tenant_ref: unknown;
  payload: unknown;
  dropped_count: number;
  reconnect_after_ms: number | null;
  terminal: boolean;
  metadata: Record<string, unknown>;
}

export class EventCursor {
  stream: EventStreamKind;
  sequence: number;
  token: string;
  constructor(fields: EventCursorFields);
  static fromJSON(raw: Uint8Array | string): EventCursor;
  resumeToken(): string;
  toJSON(): Required<EventCursorFields>;
}

export class EventFrame {
  profile: "events";
  stream: EventStreamKind;
  kind: string;
  eventId: string;
  cursor: EventCursor;
  resumeToken: string;
  occurredUnixMS: number;
  occurredAt: string;
  subjectRef: unknown;
  tenantRef: unknown;
  payload: unknown;
  droppedCount: number;
  reconnectAfterMS: number | null;
  terminal: boolean;
  metadata: Record<string, unknown>;
  constructor(fields: EventFrameFields);
  static fromJSON(raw: Uint8Array | string): EventFrame;
  toJSON(): EventFrameFields;
}

export type DirectoryEvent = EventFrame;
export type DeviceEvent = EventFrame;
export type SessionEvent = EventFrame;
export type InvocationEvent = EventFrame;
export type EventDropReport = EventFrame;

export interface DeviceEventPageFields {
  profile: "events";
  stream: "device";
  item_kind: string;
  items: EventFrameFields[];
  next_cursor: string | null;
  has_more: boolean;
  limit: number;
  metadata: Record<string, unknown>;
}

export class DeviceEventPage {
  profile: "events";
  stream: "device";
  itemKind: string;
  items: DeviceEvent[];
  nextCursor: string | null;
  hasMore: boolean;
  limit: number;
  metadata: Record<string, unknown>;
  constructor(fields: DeviceEventPageFields);
  static fromJSON(raw: Uint8Array | string): DeviceEventPage;
  toJSON(): DeviceEventPageFields;
}

export interface EventStreamOpen {
  stream?: EventStreamKind;
  state?: string;
  stream_id?: string;
  resume_token?: string;
  metadata?: Record<string, unknown>;
  max_buffered_events?: number;
}

export class EventStream {
  stream: EventStreamKind;
  state: string;
  streamId: string;
  resumeToken: string;
  metadata: Record<string, unknown>;
  handle: StreamHandle;
  constructor(stream: EventStreamKind, handle: StreamHandle, open?: EventStreamOpen);
  static fromTransportResult(result: { transport: StreamTransport; open: Uint8Array | string }, stream: EventStreamKind): EventStream;
  receive(options?: ReceiveOptions): Promise<EventFrame>;
  events(options?: AsyncIterationOptions): AsyncIterableIterator<EventFrame>;
  [Symbol.asyncIterator](): AsyncIterableIterator<EventFrame>;
  terminalEvent(): EventFrame | null;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}

export interface EventTransport {
  buildDirectorySubscriptionInvocation(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDeviceSubscriptionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildSessionSubscriptionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildInvocationSubscriptionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  subscribeDirectory?(requestJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  subscribeDevices?(requestJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  subscribeSessions?(requestJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  subscribeInvocations?(requestJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  listDeviceEvents?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectDirectoryEvent?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectLiveEvent?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectDropReport?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectTerminal?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class EventClient {
  constructor(transport: EventTransport);
  buildDirectorySubscriptionInvocation(request: EventsDirectorySubscriptionRequest): Promise<InvocationDraft>;
  buildDeviceSubscriptionInvocation(request: EventsDeviceSubscriptionRequest): Promise<InvocationDraft>;
  buildSessionSubscriptionInvocation(request: EventsSessionSubscriptionRequest): Promise<InvocationDraft>;
  buildInvocationSubscriptionInvocation(request: EventsInvocationSubscriptionRequest): Promise<InvocationDraft>;
  subscribeDirectory(request: EventsDirectorySubscriptionRequest): Promise<EventStream>;
  subscribeDevices(request: EventsDeviceSubscriptionRequest): Promise<EventStream>;
  subscribeSessions(request: EventsSessionSubscriptionRequest): Promise<EventStream>;
  subscribeInvocations(request: EventsInvocationSubscriptionRequest): Promise<EventStream>;
  listDeviceEvents(request: EventsDeviceEventListRequest): Promise<DeviceEventPage>;
  projectDirectoryEvent(input: EventProjectionInput): Promise<DirectoryEvent>;
  projectLiveEvent(input: EventProjectionInput): Promise<EventFrame>;
  projectDropReport(input: EventDropReportInput): Promise<EventDropReport>;
  projectTerminal(input: EventTerminalInput): Promise<EventFrame>;
  close(): Promise<void>;
}

export interface SurfaceCarrierBase {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface SurfaceListPagesRequest extends SurfaceCarrierBase {
  limit?: number;
  cursor?: string;
}

export interface SurfaceCreatePageRequest extends SurfaceCarrierBase {
  project_id: string;
  folder: string;
  visibility?: "public" | "private";
}

export interface SurfaceDeletePageRequest extends SurfaceCarrierBase {
  project_id: string;
}

export interface SurfaceManifestRequest extends SurfaceCarrierBase {
  project_id: string;
}

export interface SurfaceHealthRequest extends SurfaceCarrierBase {
  project_id?: string;
  surface_ref?: string;
}

export type PageQuery = SurfaceListPagesRequest;
export type CreatePageRequest = SurfaceCreatePageRequest;
export type DeletePageRequest = SurfaceDeletePageRequest;
export type SurfaceStatusRequest = SurfaceHealthRequest;

export interface SurfacePageRecordFields {
  profile: "surface";
  kind: "page_record";
  page_id: string;
  owner_ura: string;
  surface_ref: string;
  public_ref?: string | null;
  status?: string | null;
  metadata: Record<string, unknown>;
}

export class SurfacePageRecord {
  profile: "surface";
  kind: "page_record";
  pageId: string;
  ownerURA: string;
  surfaceRef: string;
  publicRef: string | null;
  status: string | null;
  metadata: Record<string, unknown>;
  constructor(fields: SurfacePageRecordFields);
  static fromJSON(raw: Uint8Array | string): SurfacePageRecord;
  toJSON(): SurfacePageRecordFields;
}

export interface SurfacePagePageFields {
  profile: "surface";
  kind: "surface_page_page";
  item_kind: "page_record";
  items: SurfacePageRecordFields[];
  next_cursor: string | null;
  limit: number;
  source: "pages_read_model";
  metadata: Record<string, unknown>;
}

export class SurfacePagePage {
  profile: "surface";
  kind: "surface_page_page";
  itemKind: "page_record";
  items: SurfacePageRecord[];
  nextCursor: string | null;
  limit: number;
  source: "pages_read_model";
  metadata: Record<string, unknown>;
  constructor(fields: SurfacePagePageFields);
  static fromJSON(raw: Uint8Array | string): SurfacePagePage;
  toJSON(): SurfacePagePageFields;
}

export interface SurfaceManifestFields {
  profile: "surface";
  kind: "surface_manifest";
  page_id: string;
  owner_ura: string;
  surface_ref: string;
  public_ref: string;
  page: SurfacePageRecordFields;
  entrypoint: Record<string, unknown>;
  metadata: Record<string, unknown>;
}

export class SurfaceManifest {
  profile: "surface";
  kind: "surface_manifest";
  pageId: string;
  ownerURA: string;
  surfaceRef: string;
  publicRef: string;
  page: SurfacePageRecord;
  entrypoint: Record<string, unknown>;
  metadata: Record<string, unknown>;
  constructor(fields: SurfaceManifestFields);
  static fromJSON(raw: Uint8Array | string): SurfaceManifest;
  toJSON(): SurfaceManifestFields;
}

export interface SurfacePublicPageRefFields {
  profile: "surface";
  kind: "public_page_ref";
  page_id: string;
  owner_ura: string;
  surface_ref: string;
  public_ref: string;
  route_kind: string;
  metadata: Record<string, unknown>;
}

export class SurfacePublicPageRef {
  profile: "surface";
  kind: "public_page_ref";
  pageId: string;
  ownerURA: string;
  surfaceRef: string;
  publicRef: string;
  routeKind: string;
  metadata: Record<string, unknown>;
  constructor(fields: SurfacePublicPageRefFields);
  static fromJSON(raw: Uint8Array | string): SurfacePublicPageRef;
  toJSON(): SurfacePublicPageRefFields;
}

export interface SurfaceMutationResultFields {
  profile: "surface";
  kind: "surface_mutation_result";
  operation: "delete";
  page_id: string;
  removed: boolean;
  state: "deleted" | "unknown";
  metadata: Record<string, unknown>;
}

export class SurfaceMutationResult {
  profile: "surface";
  kind: "surface_mutation_result";
  operation: "delete";
  pageId: string;
  removed: boolean;
  state: "deleted" | "unknown";
  metadata: Record<string, unknown>;
  constructor(fields: SurfaceMutationResultFields);
  static fromJSON(raw: Uint8Array | string): SurfaceMutationResult;
  toJSON(): SurfaceMutationResultFields;
}

export interface SurfaceHealthCheckFields {
  name: string;
  state: string;
  ready: boolean;
  message?: string | null;
  latency_ms?: number;
  metadata?: Record<string, unknown>;
}

export class SurfaceHealthCheck {
  name: string;
  state: string;
  ready: boolean;
  message: string | null;
  latencyMS: number;
  metadata: Record<string, unknown>;
  constructor(fields: SurfaceHealthCheckFields);
  toJSON(): Required<SurfaceHealthCheckFields>;
}

export interface SurfaceHealthFields {
  profile: "surface";
  kind: "surface_health";
  state: string;
  ready: boolean;
  owner_ura: string;
  surface_ref: string;
  descriptor_ref: string;
  descriptor_version: string;
  page_count: number;
  checks: SurfaceHealthCheckFields[];
  metadata: Record<string, unknown>;
}

export class SurfaceHealth {
  profile: "surface";
  kind: "surface_health";
  state: string;
  ready: boolean;
  ownerURA: string;
  surfaceRef: string;
  descriptorRef: string;
  descriptorVersion: string;
  pageCount: number;
  checks: SurfaceHealthCheck[];
  metadata: Record<string, unknown>;
  constructor(fields: SurfaceHealthFields);
  static fromJSON(raw: Uint8Array | string): SurfaceHealth;
  toJSON(): SurfaceHealthFields;
}

export declare const SurfaceStatus: typeof SurfaceHealth;

export type SurfaceProjectionInput =
  | Uint8Array
  | string
  | Record<string, unknown>
  | SurfacePageRecord
  | SurfacePagePage
  | SurfaceManifest
  | SurfacePublicPageRef
  | SurfaceMutationResult
  | SurfaceHealth;

export interface SurfaceTransport {
  buildListPagesInvocation(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildCreatePageInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDeletePageInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildManifestInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildHealthInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listPages?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  createPage?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  deletePage?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  surfaceManifest?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  publicPageRef?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  surfaceHealth?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectPageRecord?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectPagePage?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectManifest?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectPublicPageRef?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectMutationResult?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectHealth?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class SurfaceClient {
  constructor(transport: SurfaceTransport);
  buildListPagesInvocation(request: SurfaceListPagesRequest): Promise<InvocationDraft>;
  buildCreatePageInvocation(request: SurfaceCreatePageRequest): Promise<InvocationDraft>;
  buildDeletePageInvocation(request: SurfaceDeletePageRequest): Promise<InvocationDraft>;
  buildManifestInvocation(request: SurfaceManifestRequest): Promise<InvocationDraft>;
  buildHealthInvocation(request: SurfaceHealthRequest): Promise<InvocationDraft>;
  listPages(request: SurfaceListPagesRequest): Promise<SurfacePagePage>;
  createPage(request: SurfaceCreatePageRequest): Promise<SurfacePageRecord>;
  deletePage(request: SurfaceDeletePageRequest): Promise<SurfaceMutationResult>;
  surfaceManifest(request: SurfaceManifestRequest): Promise<SurfaceManifest>;
  publicPageRef(request: { page: SurfacePageRecord | SurfacePageRecordFields }): Promise<SurfacePublicPageRef>;
  surfaceHealth(request: SurfaceHealthRequest): Promise<SurfaceHealth>;
  surfaceStatus(request: SurfaceStatusRequest): Promise<SurfaceHealth>;
  projectPageRecord(value: SurfaceProjectionInput): Promise<SurfacePageRecord>;
  projectPagePage(value: SurfaceProjectionInput): Promise<SurfacePagePage>;
  projectManifest(value: SurfaceProjectionInput): Promise<SurfaceManifest>;
  projectPublicPageRef(value: SurfaceProjectionInput): Promise<SurfacePublicPageRef>;
  projectMutationResult(value: SurfaceProjectionInput): Promise<SurfaceMutationResult>;
  projectHealth(value: SurfaceProjectionInput): Promise<SurfaceHealth>;
  projectStatus(value: SurfaceProjectionInput): Promise<SurfaceHealth>;
  close(): Promise<void>;
}

export interface CompatibilityCarrierBase {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  auth_token?: string | null;
  metadata?: Record<string, unknown>;
}

export interface CompatibilityListModelsRequest extends CompatibilityCarrierBase {}

export interface CompatibilityChatCompletionRequest extends CompatibilityCarrierBase {
  request: {
    model: string;
    messages: Record<string, unknown>[];
    stream?: false;
    [key: string]: unknown;
  };
}

export interface CompatibilityStreamChatCompletionRequest extends CompatibilityCarrierBase {
  request: {
    model: string;
    messages: Record<string, unknown>[];
    stream?: boolean;
    [key: string]: unknown;
  };
}

export interface CompatibilityFileUploadRequest extends CompatibilityCarrierBase {
  id?: string;
  file_id?: string;
  file_ref?: string;
  resource_ref?: string;
  resource_ura?: string;
  filename: string;
  purpose: string;
  owner_ura?: string;
  content_type?: string;
  content_hash?: string;
  bytes_b64?: string;
  bytes?: number;
  size_bytes?: number;
  created_at?: number;
  status?: string;
}

export interface CompatibilityFileRequest extends CompatibilityCarrierBase {
  id?: string;
  file_id?: string;
  file_ref?: string;
  resource_ref?: string;
  resource_ura?: string;
  filename?: string;
  purpose?: string;
  owner_ura?: string;
  content_type?: string;
  content_hash?: string;
  bytes?: number;
  size_bytes?: number;
  created_at?: number;
  created?: number;
  status?: string;
}

export interface CompatibilityFileDeleteRequest extends CompatibilityCarrierBase {
  id?: string;
  file_id?: string;
  file_ref?: string;
  resource_ref?: string;
  resource_ura?: string;
  content_hash?: string;
  deleted: true;
}

export interface CompatibilityModelFields {
  profile: "compatibility";
  kind: "model";
  id: string;
  object: "model";
  created: number;
  owned_by: string;
  ability_ref: string;
  metadata: Record<string, unknown>;
}

export class CompatibilityModel {
  profile: "compatibility";
  kind: "model";
  id: string;
  object: "model";
  created: number;
  ownedBy: string;
  abilityRef: string;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityModelFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityModel;
  toJSON(): CompatibilityModelFields;
}

export interface CompatibilityModelPageFields {
  profile: "compatibility";
  kind: "model_page";
  object: "list";
  data: CompatibilityModelFields[];
  next_cursor: string | null;
  metadata: Record<string, unknown>;
}

export class CompatibilityModelPage {
  profile: "compatibility";
  kind: "model_page";
  object: "list";
  data: CompatibilityModel[];
  nextCursor: string | null;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityModelPageFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityModelPage;
  toJSON(): CompatibilityModelPageFields;
}

export interface CompatibilityChatCompletionFields {
  profile: "compatibility";
  kind: "chat_completion";
  id: string;
  object: "chat.completion";
  created: number;
  model: string;
  choices: Record<string, unknown>[];
  usage: Record<string, unknown>;
  metadata: Record<string, unknown>;
}

export class CompatibilityChatCompletion {
  profile: "compatibility";
  kind: "chat_completion";
  id: string;
  object: "chat.completion";
  created: number;
  model: string;
  choices: Record<string, unknown>[];
  usage: Record<string, unknown>;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityChatCompletionFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityChatCompletion;
  toJSON(): CompatibilityChatCompletionFields;
}

export interface CompatibilityChatCompletionChunkFields {
  profile: "compatibility";
  kind: "chat_completion_chunk";
  id: string;
  object: "chat.completion.chunk";
  created: number;
  model: string;
  choices: Record<string, unknown>[];
  usage?: Record<string, unknown> | null;
  metadata: Record<string, unknown>;
}

export class CompatibilityChatCompletionChunk {
  profile: "compatibility";
  kind: "chat_completion_chunk";
  id: string;
  object: "chat.completion.chunk";
  created: number;
  model: string;
  choices: Record<string, unknown>[];
  usage: Record<string, unknown> | null;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityChatCompletionChunkFields);
  toJSON(): Required<CompatibilityChatCompletionChunkFields>;
}

export interface CompatibilityChatCompletionStreamFields {
  profile: "compatibility";
  kind: "chat_completion_stream";
  stream: true;
  items: CompatibilityChatCompletionChunkFields[];
  done_sentinel: "[DONE]";
  metadata: Record<string, unknown>;
}

export class CompatibilityChatCompletionStream {
  profile: "compatibility";
  kind: "chat_completion_stream";
  stream: true;
  items: CompatibilityChatCompletionChunk[];
  doneSentinel: "[DONE]";
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityChatCompletionStreamFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityChatCompletionStream;
  toJSON(): CompatibilityChatCompletionStreamFields;
}

export interface CompatibilityFileFields {
  profile: "compatibility";
  kind: "file";
  id: string;
  object: "file";
  bytes: number;
  created_at: number;
  filename: string;
  purpose: string;
  status: string;
  metadata: Record<string, unknown>;
}

export class CompatibilityFile {
  profile: "compatibility";
  kind: "file";
  id: string;
  object: "file";
  bytes: number;
  createdAt: number;
  filename: string;
  purpose: string;
  status: string;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityFileFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityFile;
  toJSON(): CompatibilityFileFields;
}

export interface CompatibilityFileDeleteResultFields {
  profile: "compatibility";
  kind: "file_delete_result";
  id: string;
  object: "file";
  deleted: true;
  metadata: Record<string, unknown>;
}

export class CompatibilityFileDeleteResult {
  profile: "compatibility";
  kind: "file_delete_result";
  id: string;
  object: "file";
  deleted: true;
  metadata: Record<string, unknown>;
  constructor(fields: CompatibilityFileDeleteResultFields);
  static fromJSON(raw: Uint8Array | string): CompatibilityFileDeleteResult;
  toJSON(): CompatibilityFileDeleteResultFields;
}

export type CompatibilityProjectionInput =
  | Uint8Array
  | string
  | Record<string, unknown>
  | CompatibilityModelPage
  | CompatibilityChatCompletion
  | CompatibilityChatCompletionStream
  | CompatibilityFile
  | CompatibilityFileDeleteResult;

export interface CompatibilityTransport {
  buildListModelsInvocation(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildChatCompletionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildStreamChatCompletionInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildFileUploadInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildFileRetrieveInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildFileDeleteInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listModels?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  chatCompletions?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  streamChatCompletions?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  uploadFile?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  getFile?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  deleteFile?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectModelPage?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectChatCompletion?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectChatStream?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectFileUpload?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectFile?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  projectFileDeleteResult?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class CompatibilityClient {
  constructor(transport: CompatibilityTransport);
  buildListModelsInvocation(request: CompatibilityListModelsRequest): Promise<InvocationDraft>;
  buildChatCompletionInvocation(request: CompatibilityChatCompletionRequest): Promise<InvocationDraft>;
  buildStreamChatCompletionInvocation(request: CompatibilityStreamChatCompletionRequest): Promise<InvocationDraft>;
  buildFileUploadInvocation(request: CompatibilityFileUploadRequest): Promise<InvocationDraft>;
  buildFileRetrieveInvocation(request: CompatibilityFileRequest): Promise<InvocationDraft>;
  buildFileDeleteInvocation(request: CompatibilityFileDeleteRequest): Promise<InvocationDraft>;
  listModels(request: CompatibilityListModelsRequest): Promise<CompatibilityModelPage>;
  chatCompletions(request: CompatibilityChatCompletionRequest): Promise<CompatibilityChatCompletion>;
  streamChatCompletions(request: CompatibilityStreamChatCompletionRequest): Promise<CompatibilityChatCompletionStream>;
  uploadFile(request: CompatibilityFileUploadRequest): Promise<CompatibilityFile>;
  getFile(request: CompatibilityFileRequest): Promise<CompatibilityFile>;
  deleteFile(request: CompatibilityFileDeleteRequest): Promise<CompatibilityFileDeleteResult>;
  projectModelPage(value: CompatibilityProjectionInput): Promise<CompatibilityModelPage>;
  projectChatCompletion(value: CompatibilityProjectionInput): Promise<CompatibilityChatCompletion>;
  projectChatStream(value: CompatibilityProjectionInput): Promise<CompatibilityChatCompletionStream>;
  projectFileUpload(value: CompatibilityProjectionInput): Promise<CompatibilityFile>;
  projectFile(value: CompatibilityProjectionInput): Promise<CompatibilityFile>;
  projectFileDeleteResult(value: CompatibilityProjectionInput): Promise<CompatibilityFileDeleteResult>;
  close(): Promise<void>;
}

export interface ReceiptTransport {
  fetch(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildFetchInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildListHistoryInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildGetHistoryInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildTraceInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listHistory?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  getHistory?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  getTrace?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  project?(receiptJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  verify?(receiptJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  verifyChain?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  causalRef?(receiptJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export interface ReceiptFetchRequest {
  caller_ura: string;
  callee_ura: string;
  descriptor_ref: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  invocation_ura?: string;
  request_id?: string;
  trace_id?: string;
  metadata?: Record<string, unknown>;
}

export interface ReceiptHistoryReadRequest {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  timeout_ms?: number;
  arguments?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface ReceiptRefFields {
  receipt_ura: string;
  receipt_hash_hex: string;
  invocation_id?: string;
  prev_receipt_hash_hex?: string;
  index?: number;
  metadata?: Record<string, unknown>;
}

export class ReceiptRef {
  receiptURA: string;
  receiptHashHex: string;
  invocationId: string;
  prevReceiptHashHex: string;
  index: number | null;
  metadata: Record<string, unknown>;
  constructor(fields: ReceiptRefFields);
  static fromJSON(raw: Uint8Array | string): ReceiptRef;
  toJSON(): ReceiptRefFields;
  toJSONString(): string;
  causalContext(client: ReceiptClient): Promise<Record<string, unknown>>;
}

export class ReceiptChain {
  receipts: ReceiptRef[];
  constructor(receipts: Array<ReceiptRef | ReceiptRefFields>);
  toJSON(): ReceiptRefFields[];
  verifyContinuity(client: ReceiptClient, metadata?: Record<string, unknown>): Promise<Record<string, unknown>>;
}

export class ReceiptClient {
  constructor(transport: ReceiptTransport);
  fetch(request: ReceiptFetchRequest): Promise<Record<string, unknown>>;
  buildFetchInvocation(request: ReceiptFetchRequest): Promise<InvocationDraft>;
  buildListHistoryInvocation(request: ReceiptHistoryReadRequest): Promise<InvocationDraft>;
  buildGetHistoryInvocation(request: ReceiptHistoryReadRequest): Promise<InvocationDraft>;
  buildTraceInvocation(request: ReceiptHistoryReadRequest): Promise<InvocationDraft>;
  listHistory(request: ReceiptHistoryReadRequest): Promise<Record<string, unknown>>;
  getHistory(request: ReceiptHistoryReadRequest): Promise<Record<string, unknown>>;
  getTrace(request: ReceiptHistoryReadRequest): Promise<Record<string, unknown>>;
  project(receiptJSON: Uint8Array | string | Record<string, unknown>): Promise<Record<string, unknown>>;
  verify(receiptJSON: Uint8Array | string | Record<string, unknown>): Promise<Record<string, unknown>>;
  verifyChain(request: { receipts: Array<ReceiptRef | ReceiptRefFields>; metadata?: Record<string, unknown> }): Promise<Record<string, unknown>>;
  causalRef(receiptJSON: Uint8Array | string | Record<string, unknown>): Promise<Record<string, unknown>>;
  causalContext(receiptJSON: Uint8Array | string | Record<string, unknown>): Promise<Record<string, unknown>>;
  close(): Promise<void>;
}

export interface PublicationTransport {
  buildResourceRef(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  validatePackage?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  deployAbility?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDeployInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  installPlugin?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  listAbilities?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  showAbility?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  enableAbilityImpl?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  disableAbilityImpl?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildUnpublishInvocation?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  unpublishAbility?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export interface LocalResourceRefRequest {
  path: string;
  capability: "list" | "stat" | "read" | "write";
}

export interface ResourceRef {
  resource_ura: string;
  owner_ura: string;
  namespace: string;
  capability: string;
  expires_unix_ms?: number;
  revision: string;
  display_path?: string;
}

export interface PackageValidationRequest {
  package_path?: string;
  manifest?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface PublicationCarrierBase {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  descriptor_version: string;
  nonce_base64: string;
  causal_context: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface AbilityDeployRequest extends PublicationCarrierBase {
  resource_ref: ResourceRef;
  node_id: string;
}

export interface PublishedAbilityQuery extends PublicationCarrierBase {
  limit?: number;
  cursor?: string;
  owner_ura?: string;
  ability_ura?: string;
}

export interface ShowAbilityRequest {
  descriptor_ref: string;
  caller_ura?: string;
  callee_ura?: string;
  subject_ura?: string;
  descriptor_version?: string;
  nonce_base64?: string;
  causal_context?: Record<string, unknown>;
  owner_ura?: string;
  metadata?: Record<string, unknown>;
}

export interface AbilityImplLifecycleRequest {
  impl_id: string;
  ability_ura: string;
  caller_ura?: string;
  callee_ura?: string;
  subject_ura?: string;
  descriptor_version?: string;
  nonce_base64?: string;
  causal_context?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface UnpublishAbilityRequest extends PublicationCarrierBase {
  ability_ura: string;
}

export interface PluginInstallRequest {
  source: string;
  metadata?: Record<string, unknown>;
}

export class PublicationClient {
  constructor(transport: PublicationTransport);
  buildLocalResourceRef(request: LocalResourceRefRequest): Promise<Record<string, unknown>>;
  validatePackage(request: PackageValidationRequest): Promise<Record<string, unknown>>;
  deployAbility(request: AbilityDeployRequest): Promise<Record<string, unknown>>;
  buildDeployInvocation(request: AbilityDeployRequest): Promise<InvocationDraft>;
  installPlugin(request: PluginInstallRequest): Promise<Record<string, unknown>>;
  listAbilities(request: PublishedAbilityQuery): Promise<Record<string, unknown>>;
  showAbility(request: ShowAbilityRequest): Promise<Record<string, unknown>>;
  enableAbilityImpl(request: AbilityImplLifecycleRequest): Promise<Record<string, unknown>>;
  disableAbilityImpl(request: AbilityImplLifecycleRequest): Promise<Record<string, unknown>>;
  buildUnpublishInvocation(request: UnpublishAbilityRequest): Promise<InvocationDraft>;
  unpublishAbility(request: UnpublishAbilityRequest): Promise<Record<string, unknown>>;
  close(): Promise<void>;
}

export interface HostStreamBindingRequest {
  binding_id: string;
  descriptor_ref: string;
  endpoint: string;
  frame_schema: "host-stream-frame.schema.json";
  cleanup?: Record<string, unknown> | null;
  timeout_ms?: number | null;
  readiness?: Record<string, unknown> | null;
  metadata?: Record<string, unknown> | null;
}

export interface HostStreamBinding {
  binding_id: string;
  descriptor_ref: string;
  endpoint: string;
  frame_schema: "host-stream-frame.schema.json";
  cleanup: Record<string, unknown>;
  timeout_ms: number | null;
  readiness: Record<string, unknown>;
  lifecycle: Record<string, unknown>;
  metadata: Record<string, unknown>;
}

export interface HostStreamEnvelope {
  request: {
    fn: string;
    args?: unknown;
    call_id: string;
    caller: string;
    parent_receipt?: unknown;
  };
}

export interface HostStreamTerminalSummary {
  output_hash: string;
  frames: number;
  metadata?: Record<string, unknown>;
}

export interface HostStreamHashStateFields {
  algorithm: string;
  output_hash: string;
  frames: number;
  last_seq: number | null;
  canonical_json?: string;
}

export class HostStreamHashState {
  algorithm: string;
  outputHash: string;
  frames: number;
  lastSeq: number | null;
  canonicalJSON: string;
  constructor(fields: HostStreamHashStateFields);
  static initial(): HostStreamHashState;
  static fromJSON(raw: Uint8Array | string): HostStreamHashState;
  toJSON(): HostStreamHashStateFields;
}

export interface HostBindingTransport {
  buildHostStreamBinding(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  decodeRequest(envelopeJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  encodeItem(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  encodeError(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  encodeTerminal(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  foldOutputHash(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export type HostBindingDescriptorRefCanonicalizer = (descriptorRef: string) => Promise<string> | string;

export class LocalHostBindingTransport {
  constructor(descriptorRefCanonicalizer?: HostBindingDescriptorRefCanonicalizer);
  buildHostStreamBinding(requestJSON: Uint8Array | string): Promise<string>;
  decodeRequest(envelopeJSON: Uint8Array | string): Promise<string>;
  encodeItem(requestJSON: Uint8Array | string): Promise<string>;
  encodeError(requestJSON: Uint8Array | string): Promise<string>;
  encodeTerminal(requestJSON: Uint8Array | string): Promise<string>;
  foldOutputHash(requestJSON: Uint8Array | string): Promise<string>;
  close(): Promise<void>;
}

export interface HostStreamLifecycleProvider {
  checkReadiness(binding: HostStreamBinding): Promise<Record<string, unknown>> | Record<string, unknown>;
  cleanup(binding: HostStreamBinding): Promise<Record<string, unknown>> | Record<string, unknown>;
}

export class HostBindingClient {
  constructor(transport: HostBindingTransport, lifecycleProvider?: HostStreamLifecycleProvider | null);
  buildHostStreamBinding(request: HostStreamBindingRequest): Promise<HostStreamBinding>;
  decodeRequest(envelope: HostStreamEnvelope): Promise<Record<string, unknown>>;
  encodeItem(seq: number, value: unknown): Promise<Record<string, unknown>>;
  encodeError(error: Error | SDKError | Record<string, unknown>): Promise<Record<string, unknown>>;
  encodeTerminal(summary: HostStreamTerminalSummary): Promise<Record<string, unknown>>;
  foldOutputHash(state: HostStreamHashState | HostStreamHashStateFields, seq: number, value: unknown): Promise<HostStreamHashState>;
  openLifecycle(binding: HostStreamBinding, provider?: HostStreamLifecycleProvider | null): HostStreamLifecycleController;
  checkReadiness(binding: HostStreamBinding, provider?: HostStreamLifecycleProvider | null): Promise<Record<string, unknown>>;
  cleanup(binding: HostStreamBinding, provider?: HostStreamLifecycleProvider | null): Promise<Record<string, unknown>>;
  close(): Promise<void>;
}

export class HostStreamLifecycleController {
  binding: HostStreamBinding;
  state: string;
  readiness: Record<string, unknown> | null;
  cleanupResult: Record<string, unknown> | null;
  constructor(binding: HostStreamBinding, provider: HostStreamLifecycleProvider);
  checkReadiness(): Promise<Record<string, unknown>>;
  cleanup(): Promise<Record<string, unknown>>;
  close(): void;
}

export interface InvocationDraftFields {
  callerURA: string;
  calleeURA: string;
  descriptorRef: string;
  subjectURA: string;
  nonceBase64: string;
  causalContext: Record<string, unknown>;
  contentType: string;
  args?: unknown;
  argumentsBase64?: string;
  metadata?: Record<string, unknown>;
  callerSignature?: Record<string, unknown> | null;
  hasArgs?: boolean;
}

export class InvocationDraft {
  callerURA: string;
  calleeURA: string;
  descriptorRef: string;
  subjectURA: string;
  nonceBase64: string;
  causalContext: Record<string, unknown>;
  contentType: string;
  args: unknown;
  argumentsBase64: string;
  metadata: Record<string, unknown>;
  callerSignature: Record<string, unknown> | null;
  hasArgs: boolean;
  constructor(fields: InvocationDraftFields);
  static fromJSON(raw: Uint8Array | string): InvocationDraft;
  toJSON(): Record<string, unknown>;
  toJSONString(): string;
}

export class InvocationBuilder {
  withCallerURA(value: string): this;
  withCalleeURA(value: string): this;
  withDescriptorRef(value: string): this;
  withSubjectURA(value: string): this;
  withNonceBase64(value: string): this;
  withCausalContext(value: Record<string, unknown>): this;
  withJSONArgs(value: unknown): this;
  withArgumentsBase64(value: string): this;
  withContentType(value: string): this;
  withMetadata(value: Record<string, unknown>): this;
  inspect(): InvocationDraft;
  build(): InvocationDraft;
}

export interface RuntimeTransport {
  invoke(draftJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  prepare?(draftJSON: Uint8Array, optionsJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  submitSigned?(signedJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  awaitHandle?(handleId: number): Promise<Uint8Array | string> | Uint8Array | string;
  cancelHandle?(handleId: number, reason: string): Promise<Uint8Array | string> | Uint8Array | string;
  handleEvents?(handleId: number): Promise<Uint8Array | string> | Uint8Array | string;
  freeHandle?(handleId: number): Promise<void> | void;
  openStream?(draftJSON: Uint8Array): Promise<{ transport: StreamTransport; open: Uint8Array | string }> | { transport: StreamTransport; open: Uint8Array | string };
  openBidi?(draftJSON: Uint8Array, streamsJSON: Uint8Array): Promise<{ transport: BidiTransport; open: Uint8Array | string }> | { transport: BidiTransport; open: Uint8Array | string };
  close?(): Promise<void> | void;
}

export interface StreamTransport {
  receive(): Promise<Uint8Array | string> | Uint8Array | string;
  cancel?(reason?: string): Promise<void> | void;
  close?(): Promise<void> | void;
}

export interface BidiTransport {
  send(frameJSON: Uint8Array): Promise<void> | void;
  receive(): Promise<Uint8Array | string> | Uint8Array | string;
  closeSend?(): Promise<void> | void;
  cancel?(reason?: string): Promise<void> | void;
  close?(): Promise<void> | void;
}

export interface ReceiveOptions {
  signal?: AbortSignal;
  cancelReason?: string;
}

export interface AsyncIterationOptions extends ReceiveOptions {
  closeOnReturn?: boolean;
}

export interface SignerPolicyFields {
  mode?: string | null;
  signer_id?: string | null;
  policy_ref?: string | null;
  expires_at_unix_ms?: number | null;
}

export class SignerPolicy {
  mode: string;
  signerId: string;
  policyRef: string;
  expiresAtUnixMS: number;
  constructor(fields?: SignerPolicyFields);
  toJSON(): Required<SignerPolicyFields>;
}

export interface SigningMaterialFields {
  algorithm?: string | null;
  canonical_bytes_base64: string;
  args_digest_hex: string;
  descriptor_ref: string;
  nonce_base64?: string | null;
  signed_fields?: string[];
  expires_at_unix_ms: number;
  signer_policy?: SignerPolicyFields | null;
}

export class SigningMaterial {
  algorithm: string;
  canonicalBytesBase64: string;
  argsDigestHex: string;
  descriptorRef: string;
  nonceBase64: string;
  signedFields: readonly string[];
  expiresAtUnixMS: number;
  signerPolicy: SignerPolicy | null;
  constructor(fields: SigningMaterialFields);
  toJSON(): SigningMaterialFields;
}

export interface InvocationSignatureFields {
  algorithm: string;
  signature_base64: string;
  key_id_hint?: string | null;
  signer_public_key_base64?: string | null;
}

export class InvocationSignature {
  algorithm: string;
  signatureBase64: string;
  keyIdHint: string;
  signerPublicKeyBase64: string;
  constructor(fields: InvocationSignatureFields);
  toJSON(): Required<InvocationSignatureFields>;
}

export interface PreparedInvocationFields {
  prepared_id?: string | null;
  request_id?: string | null;
  tuple: Record<string, unknown>;
  signing_material: SigningMaterialFields;
  descriptor_ref?: string | null;
  descriptor_hash_hex?: string | null;
  schema_hash_hex?: string | null;
  canonical_hash_hex?: string | null;
  expires_at_unix_ms?: number | null;
  submit_ready?: false | null;
}

export class PreparedInvocation {
  preparedId: string;
  requestId: string;
  tuple: InvocationDraft;
  signingMaterial: SigningMaterial;
  descriptorRef: string;
  descriptorHashHex: string;
  schemaHashHex: string;
  canonicalHashHex: string;
  expiresAtUnixMS: number;
  constructor(fields: PreparedInvocationFields);
  static fromJSON(raw: Uint8Array | string): PreparedInvocation;
  bindRuntime(runtime: RuntimeClient): this;
  submitReady(): false;
  signWithCallerSignature(signature: InvocationSignature | InvocationSignatureFields): SignedInvocation;
  toJSON(): Required<PreparedInvocationFields>;
}

export interface SignedInvocationFields {
  prepared: PreparedInvocation | PreparedInvocationFields;
  signature: InvocationSignature | InvocationSignatureFields;
  signer_id: string;
  policy?: SignerPolicy | SignerPolicyFields | null;
}

export class SignedInvocation {
  prepared: PreparedInvocation;
  signature: InvocationSignature;
  signerId: string;
  policy: SignerPolicy | null;
  constructor(fields: SignedInvocationFields);
  bindRuntime(runtime: RuntimeClient): this;
  submitReady(): boolean;
  submit(): Promise<InvocationHandle>;
  toJSON(): Record<string, unknown>;
}

export class RuntimeClient {
  constructor(transport: RuntimeTransport);
  newInvocation(): InvocationBuilder;
  invoke(draft: InvocationDraft): Promise<Record<string, unknown>>;
  prepare(draft: InvocationDraft, options?: Record<string, unknown>): Promise<PreparedInvocation>;
  submitSigned(signed: SignedInvocation): Promise<InvocationHandle>;
  awaitResult(handle: InvocationHandle): Promise<Record<string, unknown>>;
  cancel(handle: InvocationHandle, reason?: string): Promise<InvocationCancel>;
  events(handle: InvocationHandle): Promise<InvocationHandle>;
  closeHandle(handle: InvocationHandle): Promise<void>;
  invokeStream(draft: InvocationDraft): Promise<StreamHandle>;
  openBidi(draft: InvocationDraft, streams?: Array<Record<string, unknown>>): Promise<BidiSession>;
  close(): Promise<void>;
}

export interface InvocationHandleEventFields {
  sequence: number;
  kind: string;
  state: string;
  terminal: boolean;
  reason?: string | null;
  result?: Record<string, unknown> | null;
}

export class InvocationHandleEvent {
  sequence: number;
  kind: string;
  state: string;
  terminal: boolean;
  reason: string | null;
  result: Record<string, unknown> | null;
  constructor(fields: InvocationHandleEventFields);
  toJSON(): InvocationHandleEventFields;
}

export interface InvocationHandleFields {
  handle_id: number;
  state: string;
  terminal: boolean;
  events?: InvocationHandleEventFields[];
  result?: Record<string, unknown> | null;
}

export class InvocationHandle {
  handleId: number;
  state: string;
  terminal: boolean;
  events: InvocationHandleEvent[];
  result: Record<string, unknown> | null;
  constructor(fields: InvocationHandleFields);
  static fromJSON(raw: Uint8Array | string): InvocationHandle;
  bindRuntime(runtime: RuntimeClient): this;
  awaitResult(): Promise<Record<string, unknown>>;
  cancel(reason?: string): Promise<InvocationCancel>;
  refreshEvents(): Promise<InvocationHandle>;
  close(): Promise<void>;
  toJSON(): InvocationHandleFields;
}

export interface InvocationCancelFields {
  handle_id: number;
  cancelled: boolean;
  state: string;
  terminal: boolean;
}

export class InvocationCancel {
  handleId: number;
  cancelled: boolean;
  state: string;
  terminal: boolean;
  constructor(fields: InvocationCancelFields);
  static fromJSON(raw: Uint8Array | string): InvocationCancel;
  toJSON(): InvocationCancelFields;
}

export class StreamHandle {
  terminal: boolean;
  closed: boolean;
  open: Record<string, unknown>;
  maxBufferedEvents: number;
  retainedEvents: Array<Record<string, unknown>>;
  overflow: Record<string, unknown> | null;
  constructor(transport: StreamTransport, open: Record<string, unknown>);
  receive(options?: ReceiveOptions): Promise<Record<string, unknown>>;
  events(options?: AsyncIterationOptions): AsyncIterableIterator<Record<string, unknown>>;
  [Symbol.asyncIterator](): AsyncIterableIterator<Record<string, unknown>>;
  terminalEvent(): Record<string, unknown> | null;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}

export class BidiSession {
  terminal: boolean;
  closed: boolean;
  open: Record<string, unknown>;
  maxBufferedFrames: number;
  sentFrames: Array<Record<string, unknown>>;
  receivedFrames: Array<Record<string, unknown>>;
  overflow: Record<string, unknown> | null;
  constructor(transport: BidiTransport, open: Record<string, unknown>);
  send(frame: Record<string, unknown>, options?: ReceiveOptions): Promise<void>;
  receive(options?: ReceiveOptions): Promise<Record<string, unknown>>;
  frames(options?: AsyncIterationOptions): AsyncIterableIterator<Record<string, unknown>>;
  [Symbol.asyncIterator](): AsyncIterableIterator<Record<string, unknown>>;
  terminalFrame(): Record<string, unknown> | null;
  closeSend(): Promise<void>;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}
