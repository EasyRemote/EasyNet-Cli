# Decisions Log

## 2026-07-17

- Treat the checked-in current worktree as the implementation baseline. Do not
  revert pre-existing edits.
- Follow SPEC delivery order: RF-5/RF-3 before daemon tuple and route work,
  then lifecycle, receipt, product/Mission extraction, and URA/schema gates.
- Use URA terminology even where older local skill text still says URI.
- Compatibility code is acceptable only as edge adaptation that constructs a
  complete descriptor-bound request. It is not allowed inside the canonical
  runtime domain.
- Keep the public matrix wire values (`unsupported`, `seam`,
  `provider-backed`, `cutover-ready`) for compatibility, but bind them to the
  SPEC names (`Unsupported`, `Seam`, `ProviderBacked`, `CutoverReady`) through
  `status_canonical_names`.
- Replace resolved `InvocationTarget` optional subject/causal fields with
  explicit policy values. Public callers may provide explicit tuple fields;
  daemon system calls must name their descriptor/root derivation policy before
  LocalRuntime dispatch.
- Treat plain `canonical_invocation_bytes`, `verify_signature`, and
  `run_admission` as quarantined public-surface defects in this repository's
  conformance evidence. The canonical runtime entry point is descriptor-bound
  proof.
- Treat omitted receipt authority/proof facts as a construction error in every
  Axon SDK language. Explicit zero-valued proof facts remain valid seam-level
  facts only when the caller supplies the proof-fact object directly.
- Keep receipt byte layout unchanged. RF-6 changes construction/parsing policy:
  no SDK encoder or JSON parser may synthesize self-authority or empty proof
  facts from omission.
- Treat `core/proto/axon/v1` in EasyNet-Axon as the only canonical proto source.
  EasyNet-Cli's V2 convergence gate must call Axon's
  `scripts/proto/sync_axon_v1.sh --check` instead of duplicating byte-for-byte
  mirror logic locally. This preserves a single schema ownership point while
  making RF-9 observable from the cross-repo conformance runner.
- Classify `Uri`/`.uri()` as a transport-library term only for HTTP/gRPC
  request routing and connector setup. A variable, type, function, test name,
  schema field, or error message that represents caller, callee, agent,
  ability, subject, receipt, resource, principal, or invocation identity must
  use URA naming.
- Treat RF-1 enforcement as layered. EasyNet-Cli's runtime SDK neutrality gate
  is now part of the canonical V2 runner. Axon proto/Rust product extraction is
  also checked from the V2 runner, while non-Rust Axon package extraction
  remains a separate migration because live Python, Go, Node, Java, and Swift
  product packages still require caller movement and deletion.
- Treat `tools/scripts/check-daemon-invocation-migration.sh` as the local
  RF-7/RF-8 daemon route evidence until the remaining direct response synthesis
  deletion is complete. It pins JSON control demotion, complete
  `DaemonInvocation` builder usage, and daemon-local runtime-record adapter
  boundaries.
- Treat Mission/EAL boundary enforcement in EasyNet-Cli as daemon-owned
  execution policy evidence, not as Axon protocol evidence. The canonical V2
  runner now requires hard mission-context failure, manifest-only ability
  publication, and explicit orchestration service ownership; RF-2 remains open
  until Axon Mission schema/runtime state is migrated out and gated.
- Treat key custody as a cross-boundary property, not only an SDK source-scan.
  The V2 runner now requires daemon key-service custody and product repository
  custody checks so backend/EasyRemote code cannot regain private key material,
  raw process spawning, or daemon vault/passphrase ownership.
- Treat Go SDK `audio`, `mcp`, and `tool_adapter` as product-owned RF-1
  surfaces. They are removed from Axon's canonical Go SDK rather than wrapped,
  and the V2 runner now rejects tracked Go product package files alongside the
  existing proto/Rust product checks.
- Treat Python SDK `audio`, `mcp`, `tool_adapter`, and `presets/*` packages as
  product-owned RF-1 surfaces. They are removed from Axon's canonical Python
  SDK, with generic ability package descriptor construction retained inside
  `ability_lifecycle.py`; the V2 runner now rejects tracked Python product
  package files.
- Treat Node SDK `audio`, `mcp`, `tool_adapter`, and `presets/*` packages as
  product-owned RF-1 surfaces. They are removed from Axon's canonical Node SDK
  rather than re-exported through aliases, with generic provider package
  descriptor construction and neutral `ToolSpec` retained inside
  `ability_lifecycle.ts`; the V2 runner now rejects tracked Node product
  package files.
- Treat Java SDK `Audio`, `Voice*`, `mcp/*`, `AbilityToolAdapter`,
  MCP list-dir request models, `presets/remote_control/*`, and
  `cases/ability_dispatch/*` as product-owned RF-1 surfaces. They are removed
  from Axon's canonical Java SDK, with generic provider package descriptor
  construction retained inside `AbilityLifecycle`; the V2 runner now rejects
  tracked Java product package files.
- Treat Swift SDK `Audio`, `StdioMcpServer`, and `ToolAdapter` as
  product-owned RF-1 surfaces. They are removed from Axon's canonical Swift
  SDK, with generic provider package descriptor construction and neutral
  `ToolSpec` retained inside `AbilityLifecycle.swift`; the V2 runner now
  rejects tracked Swift product package files.
- Treat `AgentSkillLayout` as the current source of truth for managed skill
  directory selection. The skill-list boundary gate must validate that layout
  selector directly rather than referring back to the retired `AgentType`
  selector name.
- Treat committed adapter reports as coverage manifests only. SDK parity live
  validation consumes generated `language.json` result files from
  `check-sdk-conformance-reports.sh`; missing live results must fail closed.
- Treat Go invocation result fixtures that still emit `receipt` as RF-6
  regressions. Normal unary result fixtures must use `terminal_receipt`;
  legacy `receipt` remains only in explicit rejection tests.
