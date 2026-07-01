# Session Frame Carrier Baseline — T0.4 (to-be-fix.spec §A2 gate)

Date: 2026-06-12 · Host: Apple Silicon macOS (darwin 25.3.0), `cargo bench
--bench session_frame_carrier`, criterion 0.5, dev machine — treat ratios
as the signal, absolute numbers as machine-relative.

## What is measured

Today's session business frames (`runtime.invoke_remote` dispatch over the
device session channel) travel as **serde-JSON `SessionDispatch` inside a
proto `BinaryChunk` inside `InvokeBidiDown`** — every frame pays a JSON
encode + proto encode on send and a proto decode + JSON decode on receive,
and `args: Vec<u8>` serializes as a JSON number array. `carrier_roundtrip`
is that full path; `canonical_proto_roundtrip` is the same payload through
a plain `InvokeRequest` proto roundtrip — the T2.1 target carrier.

## Re-baseline (2026-06-13, T2.1 shipped — the after side is REAL)

With both hemispheres landed, `carrier_v1_roundtrip` measures the
actual step-2d/step-3 wire frame (DispatchCall carrying the complete
InvokeRequest inside InvokeBidiDown):

| Metric | 1 KB args | 64 KB args |
|---|---|---|
| carrier_roundtrip (JSON, retiring) | 21.0 µs | 1.073 ms |
| **carrier_v1_roundtrip (shipped)** | **0.87 µs** | **5.97 µs** |
| **measured speedup** | **24×** | **180×** |
| canonical_proto (bare, reference) | 0.61 µs | 5.94 µs |

The v1 frame runs within ~5% of the bare InvokeRequest at 64 KB — the
InvokeBidiDown envelope costs essentially nothing. Wire sizes
unchanged from the baseline table below (the JSON column retires with
step 5).

## Results

| Metric | 1 KB args | 64 KB args |
|---|---|---|
| json_encode | 3.47 µs | 150 µs |
| json_decode | 14.5 µs | 731 µs |
| **carrier_roundtrip (today)** | **27.5 µs** | **~1.03 ms** |
| **canonical_proto_roundtrip (T2.1 target)** | **0.61 µs** | **6.6 µs** |
| ratio (today / target) | **~45×** | **~156×** |
| wire bytes (today) | 4,412 B | 262,460 B |
| wire bytes (target) | 1,271 B | 65,784 B |
| **wire inflation** | **3.47×** | **3.99×** |

## Reading

1. The double-parse cost is dominated by the JSON layer (decode alone is
   half the roundtrip); the proto layer is effectively free.
2. `args: Vec<u8>` as a JSON number array is the wire-inflation driver
   (~4× at both sizes — converging on the ~4 B/byte cost of decimal
   number encoding, worse than the +33% base64 figure the debt entry
   estimated).
3. T2.1 (carrier unification: dispatch frames carry the canonical
   Invocation proto shape) removes both costs at once. At 64 KB frames
   the per-frame CPU saving is ~1 ms — at file-transfer or media frame
   rates this is the difference between a saturated core and idle.

## Reproduce

```
cargo bench --bench session_frame_carrier
```

Re-run on the same machine twice; criterion run-to-run drift observed
< 10% (T0.4 acceptance). Re-baseline after T2.1 lands and cite both
numbers in its commit message.
