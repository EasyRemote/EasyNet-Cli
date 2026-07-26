# Execution Checklist

- [x] Confirm realtime activation plan references are construction/rendering only.
- [x] Remove `Deserialize` derives from plugin realtime/surface output read model types.
- [x] Add SPEC v2 gate to keep realtime/surface reports output-only.
- [x] Add self-test fixture proving a deserialize-capable read model fails the gate.
- [x] Run targeted tests, fmt, and gates.
- [x] Commit with required author if stable.
