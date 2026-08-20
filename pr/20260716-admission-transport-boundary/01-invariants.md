# Invariants

1. Admission local-self bypass is governed by an explicit transport boundary.
2. Local-only IPC admits daemon self callers without public signatures.
3. Off-box TCP/TLS never admits daemon-URA spoofing as local self admission.
4. Quota exemption uses the same local-self admission predicate as admission.
5. No `loopback_trusted` boolean compatibility API remains.
