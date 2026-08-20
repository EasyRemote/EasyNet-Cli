# SDK Parity Four-State Documentation Alignment

## Objective

Align `sdk/SDK_PARITY.md` with the canonical four-state capability model required by `docs/spec/daemon-sdk-requirements-v1.md` and enforced by `sdk/conformance/sdk-parity-matrix.json`.

## Boundary Proof

- Ownership: `sdk/conformance/sdk-parity-matrix.json` is the machine-checked Go/Python capability state model.
- Taxonomy: capability states are exactly `unsupported`, `seam`, `provider-backed`, and `cutover-ready`.
- Documentation boundary: `SDK_PARITY.md` may summarize matrix status and evidence, but must not introduce parallel status words such as `partial` or `gap` in capability matrix cells.
- Product boundary: product cutover readiness remains external evidence and does not become an SDK capability row.

## Implementation

- Update `SDK_PARITY.md` language status wording to use four-state vocabulary.
- Replace the hand-written capability matrix cells with explicit four-state labels plus evidence notes.
- Replace legacy stability-level prose with capability-state definitions matching the JSON matrix.
- Keep detailed known-gap paragraphs as evidence/remain-work notes, not alternate state taxonomy.

## Verification

- Run `tools/scripts/check-sdk-parity-matrix.sh`.
- Run `tools/scripts/check-sdk-parity-matrix.sh --self-test`.
- Run Python conformance tests touching the parity matrix gate.
- Run Go conformance tests touching the parity matrix gate.
- Scan `SDK_PARITY.md` for obsolete capability-state cells using `partial` or `gap`.
