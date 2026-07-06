export type RetryHintValue = "never" | "safe" | "after_backoff" | "unknown";

export declare const ErrorCode: Readonly<Record<string, string>>;
export declare const RetryHint: Readonly<{
  NEVER: "never";
  SAFE: "safe";
  AFTER_BACKOFF: "after_backoff";
  UNKNOWN: "unknown";
}>;
export declare const DEFAULT_DIRECTORY_PAGE_SIZE: 50;
export declare const MAX_DIRECTORY_PAGE_SIZE: 500;
export declare const DIRECTORY_IDENTITY_PROFILE: "directory_identity";
export declare const DEFAULT_DIRECTORY_PAGE_SIZE: 50;
export declare const MAX_DIRECTORY_PAGE_SIZE: 500;
export declare const DIRECTORY_IDENTITY_PROFILE: "directory_identity";

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

export interface IdentityTransport {
  projectDescriptorRef(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  buildDescriptorRef?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  ownerAbilityURA?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  resourceURA?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class IdentityClient {
  constructor(transport: IdentityTransport);
  projectDescriptorRef(request: Record<string, unknown>): Promise<Record<string, unknown>>;
  buildDescriptorRef(request: Record<string, unknown>): Promise<Record<string, unknown>>;
  canonicalAbilityDescriptorRef(value: string, descriptorVersion?: string): Promise<string>;
  abilityURAFromDescriptorRef(descriptorRef: string): Promise<string>;
  ownerAbilityURA(ownerURA: string, abilityName: string): Promise<string>;
  ownerAbilityDescriptorRef(ownerURA: string, abilityName: string, descriptorVersion: string): Promise<string>;
  resourceURA(ownerURA: string, path: string): Promise<string>;
  close(): Promise<void>;
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
  resolve(query: Record<string, unknown>): Promise<Record<string, unknown>>;
  listDevices(query: Record<string, unknown>): Promise<Record<string, unknown>>;
  listAgents(query: Record<string, unknown>): Promise<Record<string, unknown>>;
  listAbilities(query: Record<string, unknown>): Promise<Record<string, unknown>>;
  buildDirectorySubscriptionInvocation(request: Record<string, unknown>): Promise<Record<string, unknown>>;
  subscribeDirectory(request: Record<string, unknown>): Promise<StreamHandle>;
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

export class RuntimeClient {
  constructor(transport: RuntimeTransport);
  newInvocation(): InvocationBuilder;
  invoke(draft: InvocationDraft): Promise<Record<string, unknown>>;
  prepare(draft: InvocationDraft, options?: Record<string, unknown>): Promise<Record<string, unknown>>;
  submitSigned(signed: Record<string, unknown>): Promise<Record<string, unknown>>;
  invokeStream(draft: InvocationDraft): Promise<StreamHandle>;
  openBidi(draft: InvocationDraft, streams?: Array<Record<string, unknown>>): Promise<BidiSession>;
  close(): Promise<void>;
}

export class StreamHandle {
  terminal: boolean;
  constructor(transport: StreamTransport, open: Record<string, unknown>);
  receive(options?: ReceiveOptions): Promise<Record<string, unknown>>;
  events(options?: AsyncIterationOptions): AsyncIterableIterator<Record<string, unknown>>;
  [Symbol.asyncIterator](): AsyncIterableIterator<Record<string, unknown>>;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}

export class BidiSession {
  terminal: boolean;
  constructor(transport: BidiTransport, open: Record<string, unknown>);
  send(frame: Record<string, unknown>, options?: ReceiveOptions): Promise<void>;
  receive(options?: ReceiveOptions): Promise<Record<string, unknown>>;
  frames(options?: AsyncIterationOptions): AsyncIterableIterator<Record<string, unknown>>;
  [Symbol.asyncIterator](): AsyncIterableIterator<Record<string, unknown>>;
  closeSend(): Promise<void>;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}
