# Verification

Passed focused registration tests for Remote Desktop and Browser, plus
`git diff --check`.

Live same-source Hub/Device verification returned an invokable
`remote_desktop.attach` route with:

```text
call_mode=bidi
bidi_wire_kind=metadata_json_plus_binary
available_nodes=1
resolve_unavailable=null
```

The real browser lifecycle subsequently reached WebRTC connected and presented
decoded window frames, proving the frontend consumed this route contract.
