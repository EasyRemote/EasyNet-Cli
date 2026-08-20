# DescriptorRef Host Binding Seam Plan

## Objective

Converge Host Binding descriptor references onto the same Directory + Identity
DescriptorRef seam required by `docs/spec/daemon-sdk-requirements-v1.md`.
Local Host Binding transports may own frame codecs and rolling output hashes,
but they must not own DescriptorRef grammar or accept descriptor references with
local string checks.

## Invariants

- The SPEC remains unchanged.
- DescriptorRef canonicalization belongs to Axon/daemon helpers projected
  through Directory + Identity.
- Host Binding may build bindings only from a canonicalized DescriptorRef.
- Local frame/hash codec use remains available without daemon transport.
- There is no fallback to language-local `@` parsing for DescriptorRef validity.
- Go and Python converge on the same seam shape and error behavior.

## Implementation Steps

1. Audit Go and Python Host Binding local transports for DescriptorRef grammar
   ownership.
2. Replace local DescriptorRef fallback validation with a required canonicalizer
   for binding construction.
3. Add Identity-to-HostBinding canonicalizer adapters so product hosts can wire
   the correct profile boundary without duplicating closure logic.
4. Update Go/Python unit and shared conformance tests to prove missing
   canonicalizer fails closed and injected canonicalizers are used.
5. Run Go, Python, conformance runner, scaffold, adapter reports, SPEC diff,
   and forbidden terminology scans before committing.

## Boundary Proof

Host Binding is the owner of host-stream request/frame/terminal/hash semantics.
Directory + Identity is the owner of URA and DescriptorRef projections. A host
binding request crosses both profiles: Host Binding validates endpoint,
lifecycle, frame schema, and hash semantics; Directory + Identity supplies the
canonical DescriptorRef. Requiring a canonicalizer at the binding-construction
edge preserves both ownership boundaries without introducing a direct Axon
dependency into language facades.

## Verification Plan

- `go test ./...` in `sdk/go`
- `uv run python -m unittest discover tests` in `sdk/python`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo run --bin sdk-conformance-runner -- --language {rust,c_abi,go,python} --adapter-report ... --format jsonl`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`
- forbidden addressing/product-drift terminology scan on touched files
