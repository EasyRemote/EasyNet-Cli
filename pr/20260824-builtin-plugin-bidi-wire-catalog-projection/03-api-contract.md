# API Contract

For a builtin plugin ability with `call_mode = bidi` and a declared wire kind,
`meta.list_abilities` and the EasyNet ability catalog route expose:

```json
{
  "call_mode": "bidi",
  "bidi_wire_kind": "metadata_json_plus_binary"
}
```

Remote Desktop uses `metadata_json_plus_binary`; Browser CDP uses
`json_frames`. Missing or unsupported values remain non-executable at product
surfaces.
