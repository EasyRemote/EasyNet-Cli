# Design Slice

- The selected candidate pair remains the authoritative transport row.
- Candidate lookup accepts the exact pair reference and the `rtc` library's
  deterministic local/remote report-ID projection.
- Local references can resolve only local-candidate rows; remote references can
  resolve only remote-candidate rows.
- Route classification still derives exclusively from typed candidate stats:
  host/host is direct, srflx or prflx is STUN/reflexive, and relay is relay.
- Native `direct` classification requires both candidate rows. An unresolved
  mDNS-derived remote stats row leaves the native route unknown instead of
  guessing from the local host candidate; the browser-selected pair remains a
  separately labelled client observation.
- Addresses, TURN URLs, usernames and credentials remain absent from the
  product view.
- Browser lifecycle evidence is accepted only when the selected pair is
  nominated and succeeded, both candidate types and protocol are present, and
  the reported route class agrees with those candidate types.

## Evidence requirement

Unit tests pin the ID projection and route taxonomy. Product completion still
requires live direct, STUN, TURN and EasyNet relay artifacts with a selected,
nominated, succeeded candidate pair and rendered media bound to that pair.
