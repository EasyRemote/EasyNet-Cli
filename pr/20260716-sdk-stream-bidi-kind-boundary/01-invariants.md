# Invariants

- Stream and bidi frame domain decoders require `kind`.
- Rust C ABI callback projection emits canonical `kind` and never emits the
  retired `event` alias.
- Stream callback projection emits `payload_content_type` for payload MIME
  facts and never emits the retired stream callback `content_type` alias.
- Bidi binary callback projection emits `payload_base64` and never emits the
  retired `data_base64` alias.
- C ABI callback projection may normalize ordering, state, and error objects,
  but it does not synthesize `kind` from legacy `event`.
- Existing canonical `kind` behavior remains unchanged for stream and bidi
  sessions.
- Bidi cancel remains a non-terminal `CancelRequested` state. A later local
  `close` may release callback/session resources, but it must not claim runtime
  terminality without a terminal frame or receipt.
- No public SDK capability is reclassified in this slice.
- No alternate address terminology is introduced.
