# Boundary Proof

## Ownership

The Rust/C ABI projection owns converting daemon/Axon prepared material into the
SDK-facing prepared JSON shape. Language facades own strict DTO decoding. Once
the ABI projection emits `tuple` as an `InvocationDraft`, Go and Python should
not carry fallback normalizers for daemon materialization fields.

## Invariants

1. Public `InvocationDraft` decoding remains strict.
2. `PreparedInvocation` tuple decoding uses the same strict draft decoder.
3. Daemon/Axon materialization fields remain in `signing_material` or the
   prepared envelope, not inside `tuple`.
4. DescriptorRef equality between tuple and signing material remains enforced.
5. No URI terminology or legacy input alias is introduced.

## Rejected Designs

- Keeping language-side stripping as a compatibility fallback: rejected because
  the SPEC requires latest-only surfaces and duplicate permissive paths hide ABI
  projection regressions.
- Making tuple unknown-field rejection conditional by language: rejected because
  Go and Python must project one shared runtime model.
