# EasyNet Runtime and SDK Agent Guide

This file is the operating manual for agents working in EasyNet-Cli. The
repository owns the EasyNet product Runtime, its operator CLI, and the public
`easynet-sdk` facades. It consumes Axon protocol truth; it does not redefine it.

## Agent bootstrap

1. Read this file and `AGENTS.md` completely.
2. Run `git status --short` here and in sibling EasyNet-Axon/EasyRemote
   repositories. Preserve every unrelated change and stage explicit paths only.
3. Read the nearest module, contract tests, and current specification before
   editing. Historical `pr/` notes are evidence, not current truth.
4. Create or update `pr/<date>-<task>/` with intent, invariants, architecture,
   execution checklist, verification, and decisions before implementation.
5. Implement in ownership order: Axon protocol -> Runtime -> SDK -> downstream
   facade/Gallery. Do not repair a lower-layer contract in a higher layer.

## What this repository owns

| Area | Canonical location | Responsibility |
|---|---|---|
| Operator CLI | `src/cli/` | noun-first commands, diagnostics, lifecycle control |
| Product daemon | `src/daemon/` | boot, admission integration, routing, execution, persistence |
| Runtime support | `src/support/`, `src/runtime/` | platform and product runtime services |
| Python SDK | `sdk/python/` | public Runtime/Invocation/addressing/transport facades |
| Other SDK facades | `sdk/` | language projections over the same product-neutral contract |
| Ability descriptors | `ability-descriptors/` | installed system ability contracts |
| Product plugins | `plugins/` | separately bounded executable integrations |
| Release/conformance tooling | `tools/scripts/`, `.github/workflows/` | deterministic gates and publication |

Axon owns URA grammar, Invocation and Receipt semantics, canonical signing and
verification, admission state machines, stream/bidi terminal semantics, and
protocol error taxonomy. EasyRemote owns Python decorators, callable schema
derivation, and resident host adaptation. Product-specific logic must not leak
into Axon SDKs; protocol logic must not be duplicated in EasyRemote.

## Runtime state and security boundaries

- `easynet-daemon` is the single product Runtime process. Product callers must
  attach to it rather than start a separate Axon reference runtime.
- Credentials are persisted pairing state, not a public identity request.
  Public SDK projections must receive only accepted identity fields and must
  never receive secrets or unknown credential keys.
- User identity is the immutable paired `user_id`, not a display username.
- Runtime authority is supplied by SDK-owned authority providers. Callers and
  facades do not mint `x-runtime-delegation` or
  `x-runtime-session-authority` themselves.
- Invocation signing uses the active key-service managed signing key and its
  exact policy binding. Do not restore the retired legacy runtime signer.
- A lifecycle change must have explicit states, bounded concurrency, recovery
  after crash/restart, replay-safe outcomes, and one terminal closure.

## Build and operator startup SOP

Development build:

```bash
cargo check --locked -p easynet
cargo test --features axon-pb --no-run
cargo test --features axon-pb --lib
uv sync --project sdk/python --extra dev
uv run --project sdk/python pytest -q sdk/python/tests
```

Install and start the operator Runtime:

```bash
cargo install --path .
easynet login
easynet device join <pairing-token>   # autostarts unless --boot no
easynet runtime start                 # safe explicit start/attach
easynet status
easynet runtime logs --tail 100
```

Use `easynet runtime stop` for an orderly shutdown. There is no required
`easynet dev init` command. `easynet device join` owns pairing and credential
persistence; `runtime start` owns daemon lifecycle; downstream `node.serve()`
owns only its provider host.

For a foreground/container process use `easynet runtime connect`. Do not use it
as a background-daemon substitute in examples.

## Version coordinates

This repository has two independently scoped publishable version lines:

| Distribution | Source of truth | Update/check command | Release tag |
|---|---|---|---|
| EasyNet Runtime | root `VERSION` plus Runtime manifests | `./tools/scripts/bump-version.sh [VERSION]` | `runtime-v<VERSION>` |
| Python SDK (`easynet-sdk`) | `sdk/python/pyproject.toml` plus `sdk/python/uv.lock` | `./tools/scripts/update-python-sdk-version.sh [VERSION]`; add `--check` for read-only verification | `sdk-python-v<VERSION>` |

Never run the Runtime-wide updater to change only `easynet-sdk`: it owns root
`VERSION`, Cargo manifests, and product package manifests. Never edit a lockfile
as the source of truth.

Tide resolves a coordinate for the current functional HEAD. Release preparation
must therefore be serialized:

1. Freeze functional changes and ensure the intended branch is checked out.
2. Capture once: `RELEASE_VERSION="$(tide mark --local-only)"`.
3. Run the correct component updater with that explicit value.
4. Run the same updater in explicit `--check` mode where supported.
5. Commit only version/lock/release metadata.
6. Tag that metadata commit with the captured value. Do not recompute Tide after
   the metadata commit or after unrelated commits land.

The Python SDK flow is:

```bash
SDK_VERSION="$(tide mark --local-only)"
./tools/scripts/update-python-sdk-version.sh "$SDK_VERSION"
./tools/scripts/update-python-sdk-version.sh --check "$SDK_VERSION"
uv lock --project sdk/python --check
```

`.github/workflows/release-runtime.yml` and
`.github/workflows/publish-python-sdk.yml` validate exact tag/metadata equality.
Manual dispatch validates and builds but does not publish. A tag push publishes;
never create or push one without explicit authorization.

## Release dependency order

1. Publish the exact compatible Axon release first.
2. Prove `easynet-sdk` resolves Axon using registry-only inputs, then publish the
   SDK tag.
3. Release Runtime artifacts with the compatible Axon pin.
4. Update and publish EasyRemote only after its SDK range resolves from the
   registry.

Local path sources are allowed for development and locks, but a publish job must
use `--no-sources`/registry-only resolution so unpublished siblings cannot be
mistaken for releasable dependencies.

## Change SOP

- Diagnose ownership before coding. Identity, authority, signer, admission,
  routing, descriptor publication, receipt, and stream changes require focused
  tests in their owning layer.
- Keep public interfaces compatible while replacing obsolete internal
  architecture. Remove migrated branches; do not add fallback paths solely to
  preserve legacy behavior.
- Use focused domain objects and state machines. Avoid procedural helper piles
  and duplicated policy across unary, stream, bidi, local, and remote paths.
- For core Rust or major Runtime modules, maintain the repository contract
  header: file/description, protocol responsibility, implementation approach,
  usage contract, and architectural position.
- Run `cargo fmt --check` only after confirming unrelated dirty Rust files; do
  not format the whole tree if it would rewrite another task's work.
- Record exact passing and failing commands in the plan pack. A pre-existing
  failure must be reproduced and attributed; it must never be described as a
  pass or silently fixed outside scope.

## Script development standard

All installation, migration, version, release, and conformance scripts must:

- start with strict error handling (`set -euo pipefail` where compatible);
- resolve the repository root from `BASH_SOURCE`, never assume `$PWD`;
- validate arguments and complete target inventories before any write;
- offer a read-only `--check` or `--dry-run` path for mutating release logic;
- update coupled manifests and locks transactionally, with rollback after a
  failed generator;
- use portable baseline tools in release mutation paths; do not require `rg`
  when `find`/`awk`/`sed` can express the operation;
- use arrays and quoted paths, reject ambiguous targets, and avoid unbounded
  globs or environment-variable deletion targets;
- propagate failures instead of warning-and-continuing on required lock/build
  steps;
- emit concise old/new/result output without credentials, private keys, tokens,
  authority headers, or full credential files;
- include a focused self-test that proves success, no-write check mode,
  malformed input rejection, isolation from unrelated manifests, and rollback.

Search with `rg` is preferred for developer diagnostics and conformance checks;
the portability restriction applies to bootstrap and release mutation scripts
that must work on clean user/CI hosts.

## Verification matrix

Choose the smallest sufficient set, then expand for release work:

```bash
# Rust formatting/build/unit contract
cargo fmt --check
cargo check --locked -p easynet
cargo test --features axon-pb --lib

# Python SDK
uv lock --project sdk/python --check
uv run --project sdk/python ruff check sdk/python
uv run --project sdk/python pytest -q sdk/python/tests
bash tools/scripts/check-sdk-package-metadata.sh

# Release artifacts
uv build --project sdk/python --out-dir dist/easynet-sdk
uvx --from twine twine check dist/easynet-sdk/*
```

Run the focused `tools/scripts/check-<boundary>.sh` and its mutation/self-test
for every architecture boundary changed. Live, Docker, browser, or cross-device
claims require the corresponding real E2E; unit tests alone do not certify them.

## Commit discipline

Split commits by semantic capability and repository. Stage explicit files in a
dirty tree. Commit only as `Silan.Hu <silan.hu@u.nus.edu>`, without co-author
trailers. Do not tag, push, publish, or dispatch external workflows unless the
user explicitly requests it.
