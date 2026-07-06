export type RetryHintValue = "never" | "safe" | "after_backoff" | "unknown";

export declare const ErrorCode: Readonly<Record<string, string>>;
export declare const RetryHint: Readonly<{
  NEVER: "never";
  SAFE: "safe";
  AFTER_BACKOFF: "after_backoff";
  UNKNOWN: "unknown";
}>;

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
  constructor(transport: StreamTransport, open: Record<string, unknown>);
  receive(): Promise<Record<string, unknown>>;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}

export class BidiSession {
  constructor(transport: BidiTransport, open: Record<string, unknown>);
  send(frame: Record<string, unknown>): Promise<void>;
  receive(): Promise<Record<string, unknown>>;
  closeSend(): Promise<void>;
  cancel(reason?: string): Promise<void>;
  close(): Promise<void>;
}
