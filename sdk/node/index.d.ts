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
export declare const HEALTH_PROFILE: "health";
export declare const AUTHORITY_PROFILE: "authority";
export declare const DELEGATION_METADATA_KEY: "x-runtime-delegation";
export declare const SESSION_AUTHORITY_METADATA_KEY: "x-runtime-session-authority";
export declare const MAX_STREAM_BUFFERED_EVENTS: 1024;
export declare const MAX_BIDI_BUFFERED_FRAMES: 1024;
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

export interface AuthorityMetadataFields {
  kind: "delegation" | "session_authority";
  key: "x-runtime-delegation" | "x-runtime-session-authority";
  value: string;
}

export class AuthorityMetadata {
  kind: string;
  key: string;
  value: string;
  constructor(fields: AuthorityMetadataFields);
  toMetadata(): Record<string, string>;
  mergeInto(metadata?: Record<string, unknown>): Record<string, unknown>;
}

export interface DelegationProofFields {
  issuer_ura: string;
  subject_ura: string;
  caller_ura: string;
  audience: string;
  scopes: string[];
  issued_at_ms: number;
  expires_at_ms: number;
  signature_base64: string;
  metadata_value?: string;
}

export class DelegationProof {
  issuerURA: string;
  subjectURA: string;
  callerURA: string;
  audience: string;
  scopes: string[];
  issuedAtMS: number;
  expiresAtMS: number;
  signatureBase64: string;
  signature: Uint8Array;
  metadataValue: string;
  constructor(fields: DelegationProofFields);
  static fromMetadata(value: string): DelegationProof;
  metadata(): AuthorityMetadata;
  toJSON(): Omit<DelegationProofFields, "metadata_value">;
}

export interface SessionAuthorityFields {
  issuer_ura: string;
  session_id: string;
  session_owner_user_id: string;
  session_owner_ura?: string;
  creator_principal_id: string;
  creator_principal_ura?: string;
  callee_ura: string;
  subject_ura: string;
  audience: string;
  scopes: string[];
  allowed_actions: string[];
  allowed_followup_abilities: string[];
  issued_at_ms: number;
  expires_at_ms: number;
  signature_base64: string;
  metadata_value?: string;
}

export class SessionAuthority {
  issuerURA: string;
  sessionID: string;
  sessionOwnerUserID: string;
  sessionOwnerURA: string;
  creatorPrincipalID: string;
  creatorPrincipalURA: string;
  calleeURA: string;
  subjectURA: string;
  audience: string;
  scopes: string[];
  allowedActions: string[];
  allowedFollowupAbilities: string[];
  issuedAtMS: number;
  expiresAtMS: number;
  signatureBase64: string;
  signature: Uint8Array;
  metadataValue: string;
  constructor(fields: SessionAuthorityFields);
  static fromMetadata(value: string): SessionAuthority;
  metadata(): AuthorityMetadata;
  toJSON(): Omit<SessionAuthorityFields, "metadata_value">;
}

export interface DelegationRequestFields {
  issuer_ura: string;
  subject_ura: string;
  caller_ura: string;
  audience: string;
  scopes: string[];
  issued_at_ms: number;
  expires_at_ms: number;
  metadata?: Record<string, unknown>;
}

export class DelegationRequest {
  issuerURA: string;
  subjectURA: string;
  callerURA: string;
  audience: string;
  scopes: string[];
  issuedAtMS: number;
  expiresAtMS: number;
  metadata: Record<string, unknown>;
  constructor(fields: DelegationRequestFields);
  toJSON(): Required<DelegationRequestFields>;
}

export interface SessionAuthorityRequestFields {
  issuer_ura: string;
  session_id: string;
  session_owner_user_id: string;
  session_owner_ura?: string;
  creator_principal_id: string;
  creator_principal_ura?: string;
  callee_ura: string;
  subject_ura: string;
  audience: string;
  scopes: string[];
  allowed_actions: string[];
  allowed_followup_abilities: string[];
  issued_at_ms: number;
  expires_at_ms: number;
  metadata?: Record<string, unknown>;
}

export class SessionAuthorityRequest {
  issuerURA: string;
  sessionID: string;
  sessionOwnerUserID: string;
  sessionOwnerURA: string;
  creatorPrincipalID: string;
  creatorPrincipalURA: string;
  calleeURA: string;
  subjectURA: string;
  audience: string;
  scopes: string[];
  allowedActions: string[];
  allowedFollowupAbilities: string[];
  issuedAtMS: number;
  expiresAtMS: number;
  metadata: Record<string, unknown>;
  constructor(fields: SessionAuthorityRequestFields);
  toJSON(): Omit<
    Required<SessionAuthorityRequestFields>,
    "session_owner_ura" | "creator_principal_ura"
  >;
}

export interface AuthorityTransport {
  mintDelegationProof(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  mintSessionAuthority?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  close?(): Promise<void> | void;
}

export class AuthorityClient {
  constructor(transport: AuthorityTransport);
  mintDelegationProof(request: DelegationRequest | DelegationRequestFields): Promise<DelegationProof>;
  mintSessionAuthority(request: SessionAuthorityRequest | SessionAuthorityRequestFields): Promise<SessionAuthority>;
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

export class RuntimeReceipt {
  raw: Record<string, unknown>;
  invocationId: string;
  receiptType: string;
  state: string;
  constructor(raw: Record<string, unknown>);
  static fromObject(raw: Record<string, unknown>): RuntimeReceipt;
  lifecycleState(): string;
  rawProjection(): Record<string, unknown>;
  validateSummary(): void;
}

export interface RuntimeCallContextFields {
  caller_ura: string;
  callee_ura: string;
  subject_ura: string;
  nonce_base64?: string | null;
  causal_context?: Record<string, unknown> | null;
  metadata?: Record<string, unknown>;
  authority?: DelegationProof | SessionAuthority | AuthorityMetadata | DelegationProofFields | SessionAuthorityFields | null;
}

export class RuntimeCallContext {
  callerURA: string;
  calleeURA: string;
  subjectURA: string;
  nonceBase64: string;
  causalContext: Record<string, unknown> | null;
  metadata: Record<string, unknown>;
  authority: DelegationProof | SessionAuthority | null;
  constructor(fields: RuntimeCallContextFields);
  toJSON(): Record<string, unknown>;
}

export interface ReceiptReadCallContextFields {
  caller_ura: string;
  callee_ura: string;
  nonce_base64?: string | null;
  causal_context?: Record<string, unknown> | null;
  metadata?: Record<string, unknown>;
  authority: DelegationProof | SessionAuthority | AuthorityMetadata | DelegationProofFields | SessionAuthorityFields;
}

export declare function receiptReadCallContext(fields: ReceiptReadCallContextFields): RuntimeCallContext;

export declare function runtimeStateReadSubjectURA(realm: string, userID: string): string;

export interface ReceiptFilterFields {
  caller_ura?: string | null;
  callee_ura?: string | null;
  subject_ura?: string | null;
  ability_ura?: string | null;
  state?: string | null;
}

export class ReceiptFilter {
  callerURA: string;
  calleeURA: string;
  subjectURA: string;
  abilityURA: string;
  state: string;
  constructor(fields?: ReceiptFilterFields);
  toJSON(): Record<string, unknown>;
}

export interface ReceiptListRequestFields {
  call: RuntimeCallContext | RuntimeCallContextFields;
  filter?: ReceiptFilter | ReceiptFilterFields | null;
  limit?: number | null;
  cursor?: string | null;
}

export class ReceiptListRequest {
  call: RuntimeCallContext;
  filter: ReceiptFilter | null;
  limit: number;
  cursor: string;
  constructor(fields: ReceiptListRequestFields);
  toJSON(): Record<string, unknown>;
}

export interface ReceiptHistoryPageFields {
  records: Array<Record<string, unknown>>;
  next_cursor?: string | null;
  limit?: number | null;
  source: string;
}

export class ReceiptHistoryPage {
  records: Array<Record<string, unknown>>;
  nextCursor: string;
  limit: number;
  source: string;
  constructor(fields: ReceiptHistoryPageFields);
  static fromJSON(raw: Uint8Array | string): ReceiptHistoryPage;
  toJSON(): ReceiptHistoryPageFields;
}

export interface ReceiptProvider {
  list(request: ReceiptListRequest): Promise<ReceiptHistoryPage | ReceiptHistoryPageFields | Uint8Array | string> | ReceiptHistoryPage | ReceiptHistoryPageFields | Uint8Array | string;
}

export class SessionHistoryOperations {
  constructor(receipts: ReceiptProvider);
  list(request: ReceiptListRequest | ReceiptListRequestFields): Promise<ReceiptHistoryPage>;
}

export class RuntimeReceiptProvider implements ReceiptProvider {
  constructor(ability: RuntimeAbilityClient);
  receiptHistoryListAuthorityScope(): string;
  list(request: ReceiptListRequest | ReceiptListRequestFields): Promise<ReceiptHistoryPage>;
}

export interface AbilityDescriptorProjectionFields {
  ability_ura: string;
  descriptor_ref: string;
  name: string;
  owner_ura: string;
  version: string;
  schema_hash?: string | null;
  descriptor_hash?: string | null;
  call_mode?: string | null;
  class?: string | null;
  receipt_semantics?: Record<string, unknown> | null;
  visibility?: string | null;
  source?: string | null;
  description?: string | null;
  hints?: Record<string, unknown> | null;
  schema_summary?: Record<string, unknown> | null;
  input_schema?: Record<string, unknown> | null;
  metadata?: Record<string, unknown> | null;
}

export class AbilityDescriptorProjection {
  abilityURA: string;
  descriptorRef: string;
  name: string;
  ownerURA: string;
  version: string;
  schemaHash: string;
  descriptorHash: string;
  callMode: string;
  className: string;
  receiptSemantics: Record<string, unknown>;
  visibility: string;
  source: string;
  description: string;
  hints: Record<string, unknown>;
  schemaSummary: Record<string, unknown>;
  inputSchema: Record<string, unknown>;
  metadata: Record<string, unknown>;
  constructor(fields: AbilityDescriptorProjectionFields);
  toJSON(): AbilityDescriptorProjectionFields;
}

export interface AbilityDescriptorListRequestFields {
  call: RuntimeCallContext | RuntimeCallContextFields;
  scope?: string | null;
  owner_ura?: string | null;
  ability_ura?: string | null;
}

export class AbilityDescriptorListRequest {
  call: RuntimeCallContext;
  scope: string;
  ownerURA: string;
  abilityURA: string;
  constructor(fields: AbilityDescriptorListRequestFields);
  toJSON(): Record<string, unknown>;
}

export interface AbilityDescriptorGetRequestFields {
  call: RuntimeCallContext | RuntimeCallContextFields;
  ability_ura: string;
  descriptor_version?: string | null;
  call_mode?: string | null;
  scope?: string | null;
}

export class AbilityDescriptorGetRequest {
  call: RuntimeCallContext;
  abilityURA: string;
  descriptorVersion: string;
  callMode: string;
  scope: string;
  constructor(fields: AbilityDescriptorGetRequestFields);
}

export class AbilityDescriptorPage {
  descriptors: AbilityDescriptorProjection[];
  constructor(fields: { descriptors: Array<AbilityDescriptorProjection | AbilityDescriptorProjectionFields> });
}

export interface AbilityDescriptorProvider {
  list(request: AbilityDescriptorListRequest): Promise<AbilityDescriptorPage | { descriptors: Array<AbilityDescriptorProjection | AbilityDescriptorProjectionFields> }> | AbilityDescriptorPage | { descriptors: Array<AbilityDescriptorProjection | AbilityDescriptorProjectionFields> };
  get(request: AbilityDescriptorGetRequest): Promise<AbilityDescriptorProjection | AbilityDescriptorProjectionFields> | AbilityDescriptorProjection | AbilityDescriptorProjectionFields;
}

export class AbilityDescriptorClient {
  constructor(provider: AbilityDescriptorProvider);
  list(request: AbilityDescriptorListRequest | AbilityDescriptorListRequestFields): Promise<AbilityDescriptorPage>;
  get(request: AbilityDescriptorGetRequest | AbilityDescriptorGetRequestFields): Promise<AbilityDescriptorProjection>;
}

export class RuntimeAbilityDescriptorProvider implements AbilityDescriptorProvider {
  constructor(ability: RuntimeAbilityClient);
  list(request: AbilityDescriptorListRequest | AbilityDescriptorListRequestFields): Promise<AbilityDescriptorPage>;
  get(request: AbilityDescriptorGetRequest | AbilityDescriptorGetRequestFields): Promise<AbilityDescriptorProjection>;
}

export class RuntimeAbilityClient {
  constructor(runtime: RuntimeClient);
  build(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown,
    options?: Record<string, unknown>
  ): Promise<InvocationDraft>;
  invoke(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown
  ): Promise<Record<string, unknown>>;
  buildGovernanceRead(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown,
    provider?: string
  ): Promise<InvocationDraft>;
  invokeGovernanceRead(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown,
    provider?: string
  ): Promise<Record<string, unknown>>;
  buildCatalogueRead(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown
  ): Promise<InvocationDraft>;
  invokeCatalogueRead(
    call: RuntimeCallContext | RuntimeCallContextFields,
    abilityName: string,
    argumentsValue: unknown
  ): Promise<Record<string, unknown>>;
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
  withAuthorityMetadata(value: AuthorityMetadata | AuthorityMetadataFields): this;
  inspect(): InvocationDraft;
  build(): InvocationDraft;
}

export interface RuntimeTransport {
  invoke(draftJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  resolveDescriptorRef?(requestJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  prepare?(draftJSON: Uint8Array, optionsJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  submitSigned?(signedJSON: Uint8Array): Promise<Uint8Array | string> | Uint8Array | string;
  awaitHandle?(control: InvocationControlCapability): Promise<Uint8Array | string> | Uint8Array | string;
  cancelHandle?(control: InvocationControlCapability, reason: string): Promise<Uint8Array | string> | Uint8Array | string;
  handleEvents?(control: InvocationControlCapability): Promise<Uint8Array | string> | Uint8Array | string;
  freeHandle?(control: InvocationControlCapability): Promise<void> | void;
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
  resolveDescriptorRef(request: Record<string, unknown>): Promise<string>;
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

export interface InvocationControlCapability {}

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
  controlCapability: InvocationControlCapability;
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
  request_accepted: boolean;
  deduplicated: boolean;
  cancelled: boolean;
  state: string;
  terminal: boolean;
}

export class InvocationCancel {
  controlCapability: InvocationControlCapability;
  requestAccepted: boolean;
  deduplicated: boolean;
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
