# Daemon SDK Requirements v1

Status: active design spec.

Audience: EasyNet-Cli, EasyNet backend, EasyRemote, and language binding
maintainers.

Purpose: define the public SDK surface owned by EasyNet-Cli for controlling and
calling `easynet-daemon`. This is a Daemon SDK, not a command-line SDK. The
`easynet` executable is one consumer of this SDK; it is not the SDK boundary.

## 1. Decision

EasyNet product clients MUST depend on EasyNet-Cli SDK surfaces, not directly on
EasyNet-Axon SDKs or raw `axon.v1` generated protocol types.

The target dependency graph is:

```text
EasyNet backend / EasyRemote / GUI / host app / easynet CLI
  -> EasyNet-Cli SDK
  -> local easynet-daemon
  -> Axon SDK, proto, LocalRuntime, admission, receipts
```

The forbidden product graph is:

```text
EasyNet backend / EasyRemote
  -> EasyNet-Axon SDK or generated axon.v1 proto
  -> raw axon-runtime
```

This does not mean the SDK hides the words Invocation, URA, AbilityDescriptor,
or Receipt. Those are public EasyNet/Axon semantic terms. It means consumers do
not import Axon packages, do not construct Axon proto messages, and do not start
or dial raw `axon-runtime` for product paths.

## 2. Ownership

| Layer | Owns | Must not own |
|---|---|---|
| Axon | Invocation wire shape, canonical bytes, URA profile, admission, receipt verification, stream/bidi protocol invariants | EasyNet daemon lifecycle, plugins, keyring forwarding, EAL/Mission product policy, backend DB/product UX |
| EasyNet-Cli daemon | Device/Hub process lifecycle, local sockets, identity, keyring, plugin/MCP/EAL/Mission execution, local and remote dispatch, session state, daemon system abilities | Browser auth, product database, OAuth/JWT, frontend DTOs |
| EasyNet-Cli SDK | Language facade over daemon lifecycle and daemon Invocation transport | New protocol semantics, raw shell command transport, backend product state |
| EasyNet backend | Browser/product HTTP API, user/device dashboards, DB projections, Join Token UX | Ability execution, daemon session routing, Axon protocol rules |
| EasyRemote | Python developer facade for turning functions/classes/pipelines into abilities | Low-level daemon transport, canonical signing, receipt verification, daemon process ABI |

## 3. Consumers

The SDK MUST support these first-class consumers:

1. EasyNet backend, primarily through a Go SDK.
2. EasyRemote, primarily through a Python SDK or Python binding over
   `libeasynet_cli`.
3. The `easynet` command-line executable.
4. Desktop or GUI products that need daemon lifecycle and invocation.
5. Third-party host apps that expose local resources as EasyNet abilities.

The EasyNet backend MUST NOT depend on EasyRemote. EasyRemote is a product
facade above the daemon SDK, not a runtime substrate for the backend.

## 4. Public SDK Families

The SDK family SHOULD have these surfaces:

| Surface | Role | Stability target |
|---|---|---|
| Rust crate API, `easynet_cli::daemon` | Implementation-adjacent native API and source of truth for daemon lifecycle/client shapes | Semver crate API |
| C ABI, `libeasynet_cli` | Stable language-binding ABI | ABI versioned, header-checked |
| Go SDK, `easynet.run/cli/sdk/go/...` | Primary EasyNet backend dependency | Semver module API |
| Python SDK, package name TBD | Primary EasyRemote low-level dependency | Semver package API |
| Future bindings | Node, Swift, Java, etc. | Generated or hand-written over the same schema |

All surfaces MUST share the same logical data model and JSON schemas for
Invocation, authority metadata, daemon status, receipt summaries, stream events,
and typed errors.

## 5. Non-goals

The Daemon SDK MUST NOT expose:

1. `StartAxonRuntime`, `start_runtime`, or any raw `axon-runtime` lifecycle.
2. Raw `pb.InvokeRequest`, `pb.InvokeResponse`, `pb.InvokeBidiUp`, or other
   generated `axon.v1` types at product-facing boundaries.
3. Direct imports of `easynet.run/axon/*` in product SDK APIs.
4. `InvokeAbility(name, args)` or any public ability+args-only call surface.
5. A shell transport such as `RunCLICommand("easynet ...")`.
6. Product invocation over `control.sock` JSON frames.
7. Backend auth/JWT/OAuth/database helpers.
8. Frontend DTOs or HTTP route helpers.
9. One method per system ability as the only calling model.

Convenience helpers MAY exist, but every helper MUST eventually produce an
inspectable complete Invocation before dispatch.

## 6. Core Invariants

### 6.1 Complete Invocation

Every dispatch API MUST preserve the complete Invocation tuple:

```text
caller, callee, ability, subject, nonce, causal_context, args -> receipt
```

SDK builders MAY fill defaults such as a fresh nonce or null causal context, but
the filled values MUST be visible before dispatch. Hidden `subject`, hidden
`nonce`, or silently empty causal context at a public boundary is a correctness
bug.

### 6.2 Identity split

The SDK MUST keep these separate:

| Term | Meaning |
|---|---|
| URA | Routable logical identity/address for agents, devices, abilities, resources, sessions, receipts |
| Endpoint | Transport locator such as UDS path or TCP endpoint |
| Process | Local OS process, e.g. `easynet-daemon` |
| Plugin/AbilityImpl | Executable implementation that can satisfy one descriptor version |

`callee` is the logical agent/device that advertises an AbilityDescriptor. It is
not the UDS path, TCP endpoint, daemon process, node id string, or plugin
instance.

### 6.3 Transport split

`control.sock` is only for boot/status/discovery. Product calls MUST use the
daemon Invocation endpoint (`daemon.sock` locally, TCP+TLS for remote
device/Hub traffic). SDKs MAY use `control.json` to discover the current
Invocation endpoint, but MUST NOT dispatch product ability calls over control
frames.

### 6.4 Axon dependency containment

SDK implementation crates/packages MAY depend on Axon SDKs and generated
protocol types internally. Public product APIs MUST wrap those in EasyNet-Cli
owned DTOs and typed errors.

## 7. Package Layout

The Go SDK SHOULD use this package family:

```text
easynet.run/cli/sdk/go/daemon      // lifecycle, discovery, health
easynet.run/cli/sdk/go/runtime     // daemon Invocation client
easynet.run/cli/sdk/go/invocation  // complete Invocation DTO and signing material
easynet.run/cli/sdk/go/identity    // local identity, signer, pubkey helpers
easynet.run/cli/sdk/go/directory   // devices, agents, abilities read model
easynet.run/cli/sdk/go/session     // stream/bidi/session abstractions
easynet.run/cli/sdk/go/files       // fs.transfer convenience wrapper
easynet.run/cli/sdk/go/terminal    // terminal and remote desktop wrappers
easynet.run/cli/sdk/go/plugin      // local plugin/AbilityImpl lifecycle
easynet.run/cli/sdk/go/mission     // EAL/Mission facade
```

The Python SDK SHOULD mirror the same conceptual modules. EasyRemote SHOULD
depend on the Python SDK and keep only function/class/pipeline developer
experience in `easyremote`.

## 8. Daemon Lifecycle API

Lifecycle APIs manage or attach to a local `easynet-daemon` process. They do not
perform ability dispatch.

Required Go shape:

```go
type Mode string

const (
    ModeDevice Mode = "device"
    ModeHub    Mode = "hub"
    ModeBoth   Mode = "both"
)

type StartConfig struct {
    Mode        Mode
    Realm       string
    NodeID      string
    HomeDir     string
    DaemonBin   string
    LogPath     string
    Detached    bool
    Env         map[string]string

    UDSPath     string
    ListenTCP   string
    TLSCertPath string
    TLSKeyPath  string
    HubEndpoint string
    TrustPath   string
}

type Endpoints struct {
    ControlEndpoint    string
    InvocationEndpoint string
    PublicEndpoint     string
}

type Handle interface {
    Status(ctx context.Context) (DaemonStatus, error)
    Endpoints() Endpoints
    OpenClient(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
    Stop(ctx context.Context) error
}

func Start(ctx context.Context, cfg StartConfig) (Handle, error)
func Attach(ctx context.Context, opts AttachOptions) (Handle, error)
func Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error)
func ConnectLocal(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
```

Requirements:

1. `Start` MUST return only after both control and Invocation endpoints are
   accepting connections.
2. `Attach` MUST fail closed when a control endpoint is alive but the Invocation
   endpoint is down.
3. `Discover` MUST prefer the daemon-advertised `invocation_endpoint` over
   hard-coded `~/.easynet/daemon.sock`.
4. `ModeDevice` MUST NOT accept a public TCP listener.
5. Hub or both mode with `ListenTCP` MUST require TLS material or an explicit
   daemon-side provisioning path.

## 9. Runtime Client API

Runtime APIs submit complete Invocations to the daemon.

Required Go shape:

```go
type Client struct{}

type ConnectOptions struct {
    Endpoint        string
    ControlPath     string
    DialTimeout     time.Duration
    InvokeTimeout   time.Duration
    MaxMessageBytes int
    Signer          identity.Signer
    Reconnect       bool
}

func Connect(ctx context.Context, opts ConnectOptions) (*Client, error)
func (c *Client) Health(ctx context.Context) (RuntimeHealth, error)
func (c *Client) Invoke(ctx context.Context, inv invocation.Invocation) (invocation.Result, error)
func (c *Client) InvokeStream(ctx context.Context, inv invocation.Invocation) (session.Stream, error)
func (c *Client) OpenBidi(ctx context.Context, inv invocation.Invocation, streams []session.StreamDescriptor) (session.Bidi, error)
func (c *Client) Close() error
```

Requirements:

1. `Client` MUST hide gRPC, UDS, generated protocol types, reconnect state, and
   frame-zero construction.
2. `Invoke` MUST return an SDK `invocation.Result`, not raw proto.
3. `InvokeStream` MUST expose SDK `session.Stream` events and terminal state,
   not raw `InvokeStreamChunk`.
4. `OpenBidi` MUST send a correct frame 0 before returning the session.
5. Reconnecting clients MUST invalidate only transport-broken connections, not
   application-level invocation failures.

## 10. Invocation API

The Invocation package is the SDK's load-bearing contract.

Required Go shape:

```go
type URA string
type DescriptorRef string // ability_ura@descriptor_version

type Invocation struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Nonce         [16]byte
    Causal        CausalContext
    Args          Payload
    ContentType   string

    Metadata      map[string]string
    Authority     AuthorityProof
    CallerSig     *CallerSignature
    Timeout       time.Duration
}

type Payload struct {
    JSON any
    Raw  []byte
    ContentType string
}

type Builder struct{}

func New(caller, callee URA, descriptor DescriptorRef, subject URA) *Builder
func (b *Builder) JSON(v any) *Builder
func (b *Builder) Bytes(contentType string, raw []byte) *Builder
func (b *Builder) Nonce(n [16]byte) *Builder
func (b *Builder) FreshNonce() *Builder
func (b *Builder) CausalNone(reason string) *Builder
func (b *Builder) CausalScalar(ref ReceiptRef) *Builder
func (b *Builder) CausalVector(refs []ReceiptRef) *Builder
func (b *Builder) CausalMerkle(root [32]byte, proofURA URA) *Builder
func (b *Builder) Authority(a AuthorityProof) *Builder
func (b *Builder) CallerSignature(sig CallerSignature) *Builder
func (b *Builder) Timeout(d time.Duration) *Builder
func (b *Builder) Metadata(k, v string) *Builder
func (b *Builder) Build() (Invocation, error)
```

Requirements:

1. `Build` MUST reject empty caller, callee, descriptor, subject, args content
   type, invalid nonce length, and ambiguous payload forms.
2. `DescriptorRef` MUST bind ability identity and descriptor version.
3. `Payload` MUST support JSON and raw bytes without re-encoding raw payloads.
4. SDKs MUST NOT publish a public `InvokeAbility(name, args)` API that bypasses
   caller/callee/subject/nonce/causal visibility.

## 11. Prepare, Sign, Submit

This API supports browser/user signing flows and backend-mediated signed submit.

Required Go shape:

```go
type Draft struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Args          Payload
    Causal        CausalContext
    Metadata      map[string]string
    Timeout       time.Duration
}

type PrepareOptions struct {
    ResolveDescriptor bool
    FillNonce         bool
    RequireUserSig    bool
    ExpiresIn         time.Duration
}

type PreparedInvocation struct {
    Invocation    Invocation
    RequestID     string
    DescriptorRef DescriptorRef
    ExpiresAt     time.Time
}

type SigningMaterial struct {
    CanonicalBytes []byte
    ArgsDigestHex  string
    NonceBase64    string
    SignedFields   []string
}

func (c *Client) Prepare(ctx context.Context, draft invocation.Draft, opts invocation.PrepareOptions) (invocation.PreparedInvocation, invocation.SigningMaterial, error)
func (c *Client) SubmitSigned(ctx context.Context, prepared invocation.PreparedInvocation, sig invocation.CallerSignature) (invocation.Result, error)
```

Requirements:

1. Canonical bytes MUST be computed inside the CLI SDK by delegating to Axon
   internals, never by product code.
2. `SigningMaterial` MUST be stable across Go, Python, Rust, and C ABI bindings
   for the same Invocation.
3. `SubmitSigned` MUST preserve the caller's signature and public key material;
   it MUST NOT replace a user signature with the backend's hub key.
4. Prepared invocations MUST expire or carry enough metadata for callers to
   reject stale browser signatures.

## 12. Authority API

The SDK MUST expose authority metadata as typed values.

Required Go shape:

```go
type Signer interface {
    PublicKey() []byte
    Sign(ctx context.Context, msg []byte) ([]byte, error)
}

type SessionAuthority struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

type DelegationProof struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

func NewSessionAuthority(ctx context.Context, signer Signer, req SessionAuthorityRequest) (SessionAuthority, error)
func NewDelegationProof(ctx context.Context, signer Signer, req DelegationRequest) (DelegationProof, error)
```

Requirements:

1. `SessionAuthority` and `DelegationProof` MUST be mutually exclusive on one
   Invocation.
2. Authority metadata MUST bind issuer, subject, audience, scopes, expiry, and
   signature.
3. Builder APIs MUST reject ambiguous authority.

## 13. Directory and Catalog API

Directory APIs project daemon read models. They may call daemon abilities such
as `namespace.resolve`, `namespace.proxy_resolve`, or
`federation.list_user_devices` internally, but product consumers do not need to
know those carrier names.

Required Go shape:

```go
type Client struct { Runtime *runtime.Client }

func (d *Client) Resolve(ctx context.Context, q ResolveQuery) (ResolveAnswer, error)
func (d *Client) ListDevices(ctx context.Context, q DeviceQuery) ([]Device, error)
func (d *Client) ListAgents(ctx context.Context, q AgentQuery) ([]Agent, error)
func (d *Client) ListAbilities(ctx context.Context, q AbilityQuery) ([]AbilityDescriptor, error)
func (d *Client) Subscribe(ctx context.Context, f DirectoryFilter) (DirectoryStream, error)
```

Requirements:

1. Directory results MUST expose URAs for owners, devices, agents, and
   abilities.
2. Directory errors MUST use typed SDK errors, not human string parsing.
3. Event subscriptions MUST expose SDK event DTOs, not raw federation proto.

## 14. Identity API

The SDK MUST expose identity helpers needed by daemon clients and product
backends while keeping keyring policy daemon-owned.

Required capabilities:

1. Load or initialize local Hub identity for a realm.
2. Load device identity from daemon/keyring context.
3. Construct URAs through SDK-owned builders.
4. Register, list, and revoke user signing keys through daemon identity
   abilities.
5. Provide `Signer` implementations for local Ed25519 identities.

The SDK MUST NOT expose backend JWT/OAuth/session concepts.

## 15. Stream, Bidi, and Session API

SDK sessions hide raw daemon frame details.

Required Go shape:

```go
type Stream interface {
    Recv(ctx context.Context) (StreamEvent, error)
    Close() error
}

type Bidi interface {
    Send(ctx context.Context, frame UpFrame) error
    Recv(ctx context.Context) (DownFrame, error)
    CloseSend() error
    Cancel(ctx context.Context) error
}
```

Requirements:

1. Streams MUST make terminal state explicit.
2. Bidi sessions MUST hide frame-zero construction and sequence numbering.
3. EOF, terminal receipt, cleanup receipt, and transport cancellation MUST be
   distinguishable.
4. Long-lived streams MUST support caller cancellation through context and an
   explicit close/cancel method.

## 16. Files, Terminal, and Remote Desktop Wrappers

These wrappers are convenience APIs over complete Invocation and bidi sessions.
They MUST NOT be the only way to reach those abilities.

Required capabilities:

```go
func OpenFileTransfer(ctx context.Context, c *runtime.Client, req FileTransferRequest) (FileTransfer, error)
func OpenTerminal(ctx context.Context, c *runtime.Client, req TerminalRequest) (TerminalSession, error)
func OpenRemoteDesktop(ctx context.Context, c *runtime.Client, req RemoteDesktopRequest) (RemoteDesktopSession, error)
```

Requirements:

1. Wrappers MUST still produce a complete Invocation internally.
2. Wrappers MUST surface admission failure, routing failure, terminal failure,
   and client cancellation as typed errors.
3. Backend HTTP/WS bridges MAY consume these wrappers, but wrappers MUST NOT
   import backend packages.

## 17. Device, Plugin, AbilityImpl, and Mission APIs

These APIs primarily serve CLI, desktop apps, local host apps, and EasyRemote.
Backend usage is allowed but not required.

Required capabilities:

```go
func JoinHub(ctx context.Context, req JoinRequest) (JoinResult, error)
func LeaveHub(ctx context.Context, req LeaveRequest) error

func InstallPlugin(ctx context.Context, source string, opts InstallOptions) (PluginInstallResult, error)
func EnableAbilityImpl(ctx context.Context, id AbilityImplID) error
func DisableAbilityImpl(ctx context.Context, id AbilityImplID) error

func RunMission(ctx context.Context, req MissionRequest) (MissionResult, error)
func TrackMission(ctx context.Context, id MissionID) (MissionStatus, error)
func CancelMission(ctx context.Context, id MissionID) error
```

Requirements:

1. Plugin, skill, and host-process management belongs to implementation-resource
   management, not protocol ability identity.
2. Mission/EAL helpers MUST create child Invocations for ability calls rather
   than redefining Invocation semantics.

## 18. Health and Diagnostics API

SDK health MUST separate product process liveness from runtime readiness.

Required shape:

```go
type RuntimeHealth struct {
    DaemonVersion       string
    SDKVersion          string
    Mode                daemon.Mode
    Realm               string
    PID                 int
    ControlEndpoint     string
    InvocationEndpoint  string
    PublicEndpoint      string
    UDSReady            bool
    PublicListenerReady bool
    TrustReady          bool
    DirectoryReady      bool
    LastError           string
}
```

Requirements:

1. Local health MUST check the Invocation endpoint, not only control discovery.
2. Hub health SHOULD check public TCP+TLS listener readiness when configured.
3. Health MUST expose typed readiness flags so product backends can distinguish
   "API alive but runtime unavailable" from successful runtime readiness.

## 19. Error Model

All bindings MUST map daemon, transport, admission, timeout, cancellation, and
protocol failures into typed errors.

Required Go shape:

```go
type ErrorCode string

const (
    ErrDaemonOffline     ErrorCode = "DAEMON_OFFLINE"
    ErrPermissionDenied  ErrorCode = "PERMISSION_DENIED"
    ErrAdmissionDenied   ErrorCode = "ADMISSION_DENIED"
    ErrAbilityNotFound   ErrorCode = "ABILITY_NOT_FOUND"
    ErrRouteUnavailable  ErrorCode = "ROUTE_UNAVAILABLE"
    ErrTimeout           ErrorCode = "TIMEOUT"
    ErrCancelled         ErrorCode = "CANCELLED"
    ErrInvalidInvocation ErrorCode = "INVALID_INVOCATION"
    ErrProtocolMismatch  ErrorCode = "PROTOCOL_MISMATCH"
)

type RuntimeError struct {
    Code       ErrorCode
    Stage      string
    Message    string
    Retryable  bool
    ReceiptURA string
    Cause      error
}
```

Requirements:

1. Product consumers MUST NOT parse human-readable daemon error strings.
2. Errors SHOULD preserve receipt URA or invocation id when the daemon provides
   one.
3. Retriability MUST be explicit; do not infer it from string fragments.

## 20. Wire JSON Schemas

The C ABI, Python SDK, and any binding that does not use generated Go/Rust
types MUST accept the same Invocation JSON shape:

```json
{
  "caller_ura": "easynet:///r/example/agent/alice",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/device/dev-a/ability/observe.health@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {},
  "caller_signature": {
    "algorithm": "ed25519",
    "signature_base64": "...",
    "signer_public_key_base64": "..."
  }
}
```

For non-JSON payloads, callers pass `arguments_base64` and `content_type`
instead of `args`.

The SDK MUST reject:

1. Both `args` and `arguments_base64`.
2. Neither `args` nor `arguments_base64`.
3. Empty tuple fields.
4. Missing descriptor version in descriptor-bound calls.
5. Non-16-byte nonces.
6. Ambiguous authority metadata.

## 21. EasyRemote Relationship

EasyRemote SHOULD become a high-level consumer of the Python Daemon SDK.

The following EasyRemote responsibilities SHOULD move to the SDK:

1. `libeasynet_cli` loading and ABI version checks.
2. Daemon lifecycle/status wrappers.
3. Complete Invocation JSON codec.
4. URA builders/parsers used for daemon calls.
5. Canonical signing material and caller signature transport.
6. Receipt and typed error projection.
7. Ability/agent/mission daemon control surfaces that are not Python-specific.

The following SHOULD remain in EasyRemote:

1. `ComputeNode`.
2. `@node.register`.
3. `@remote`.
4. Python schema extraction from functions/classes.
5. Warm Python host process integration.
6. `Pipeline` Python DSL.
7. Gallery, examples, and product positioning.

EasyRemote MUST NOT be a dependency of EasyNet backend.

## 22. Backend Migration Requirements

EasyNet backend migration is complete only when:

1. Backend `go.mod` no longer requires `easynet.run/axon/sdk/go`.
2. Backend does not import `easynet.run/axon/*`.
3. Backend does not expose or depend on generated `axon.v1` proto types.
4. Backend runtime injection is named `Runtime` or `DaemonRuntime`, not `Axon`.
5. Backend ability invoke, directory, events, identity, file, terminal, remote
   desktop, and OpenAI compatibility paths all use the CLI Go SDK.
6. Backend tests use `fakeRuntimeClient`, not `fakeAxonClient`.
7. Backend health distinguishes API liveness from daemon runtime readiness.

## 23. Conformance and CI Gates

The SDK repo MUST provide these gates before backend cutover:

1. Golden conformance cases for Invocation JSON -> canonical signing material.
2. Rust, C ABI, Go, and Python parity tests for the same Invocation fixture.
3. ABI v3 header/export/version checks.
4. Go SDK import ban: public Go SDK packages may import Axon internally only in
   explicitly whitelisted adapter packages; consumers must not see Axon types.
5. EasyNet backend import ban: no `easynet.run/axon/*` after cutover.
6. `control.sock` product-call ban: no `Invoke`, `Subscribe`, or `OpenBidi`
   product dispatch over JSON control frames.
7. Live daemon smoke covering unary, stream, bidi, file transfer, and typed
   terminal failure.
8. Health smoke covering daemon down, UDS permission denied, public listener
   down, and trust not ready.

## 24. MVP Scope

MVP order:

1. Go SDK core: `ConnectLocal`, `Invocation`, `Prepare`, `SubmitSigned`,
   `Invoke`.
2. Reconnecting client, health, and typed errors.
3. Directory helpers: devices, agents, abilities, directory events.
4. Stream and bidi abstractions.
5. File, terminal, and remote desktop wrappers.
6. Python SDK extraction for EasyRemote.
7. Device join, plugin, and mission lifecycle helpers.
8. Backend cutover and import-ban enforcement.

The minimum stable core is complete Invocation plus signing material plus daemon
client. Daemon lifecycle alone is not sufficient.

## 25. Open Questions

1. Exact public module path for the Go SDK (`easynet.run/cli/sdk/go` vs
   `easynet.run/easynet-cli/sdk/go`).
2. Exact Python package name (`easynet-runtime`, `easynet-cli-sdk`, or another
   name).
3. Whether Go SDK should use pure Go gRPC over UDS or call `libeasynet_cli`
   through cgo. The default target is pure Go for backend deploy simplicity.
4. Whether ACME/TLS provisioning belongs in SDK lifecycle or remains an
   operator/Gateway convenience.
5. Full receipt fetch and receipt-chain verification API shape once receipt URA
   builders and fetch paths are finalized.
