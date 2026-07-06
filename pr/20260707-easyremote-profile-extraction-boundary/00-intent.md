# EasyRemote Profile Extraction Boundary

## Goal

Make the EasyRemote cutover boundary explicit at the profile level. The SDK
already exposes Python facade objects for Runtime Core, Directory + Identity,
Receipt, Publication, Host Binding, Mission, and Admin + Gateway. This slice
turns the remaining EasyRemote extraction contract into a shared conformance
case and executable boundary gate so EasyRemote cannot regress to raw daemon
system ability carriers, raw URA parsing, or host-stream wire semantics.

## Non-goals

- Do not add new Axon protocol semantics to the Python SDK.
- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not migrate EasyRemote product code in this repository slice.
- Do not introduce compatibility fallbacks for old EasyRemote `_transport`.

## Acceptance Criteria

- A shared `python/easyremote_profile_extraction` case names the absorbed SDK
  profiles, required SDK facade types, and forbidden EasyRemote semantics.
- Python conformance tests prove the case is present and tied to the existing
  consumer boundary auditor.
- The EasyRemote boundary self-test fails on raw publication, admin/gateway,
  mission, host-stream, addressing, and descriptor-ref semantics.
- Scaffold and action-adapter reports include the new case.
