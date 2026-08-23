# Invariants

1. The runner uses public EasyNet CLI abilities and the same selected Resource,
   consent receipt, session binding, and WebRTC transport as the product.
2. Interactive authority is requested explicitly with `--input-control`; no
   diagnostic or direct in-process injection path substitutes for the data
   channel.
3. Input uses the canonical `easynet.remote_desktop.input.v1` channel created
   before the offer, and preserves monotonic client sequence and timestamps.
4. Pointer frames carry the current committed target geometry revision;
   pointer and key frames carry the current committed target focus epoch.
5. The AppKit fixture is only an observer. It never injects or fabricates input
   and records actual target-local AppKit mouse/key callbacks.
6. The selected target must be focused and the unrelated target must remain
   free of the unique test events.
7. A pass requires matching submitted frames, daemon `INPUT_FRAME_APPLIED`
   events with fresh target-guard proof, and independently observed OS effects.
8. Timeouts, missing permissions, focus drift, stale epochs, target leakage, or
   absent terminal cleanup fail closed with retained artifacts.
