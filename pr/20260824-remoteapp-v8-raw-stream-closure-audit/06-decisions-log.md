# Decisions log

## 2026-08-24

- Audit the already-landed v8 work rather than reimplementing it from the
  earlier design proposal.
- Preserve `runtime_abi_version() == 7` if feature discovery provides the
  additive v8 capability and existing bindings intentionally accept the v7
  base; a major-version integer bump is not required merely to expose one
  additive optional symbol.
- Require SDK selection to match all three feature facts: extension enabled,
  exact advertised symbol name, and symbol capability bit. Symbol presence
  alone or a lone boolean is not a valid negotiation.
- Keep RemoteApp media on WebRTC/binary InvokeBidi. ABI v8 is the raw
  server-stream representation used by EasyRemote and other Runtime stream
  consumers, not a replacement RemoteApp media transport.
- Add a standalone v8 contract spec and ship it with release packages instead
  of hiding the extension contract only inside the v7 base document.
