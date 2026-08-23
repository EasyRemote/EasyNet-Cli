# Architecture

```text
host runner
  -> AppKit selected + unrelated observer fixtures
  -> resource.refresh_remote_targets
  -> grant consent + create interactive session
  -> one WebRTC peer
       -> H.264 receive/decode
       -> easynet.remote_desktop.input.v1
            -> pointer + key frames with current epochs
  -> daemon target-local guard + CGEvent post
  -> session events: INPUT_FRAME_APPLIED
  -> AppKit observer JSONL: mouse/key callbacks
  -> evidence join + strict existing verifier
  -> end session + fixture cleanup
```

The receiver owns WebRTC peer behavior. The fixture owns independent host
observation. The shell runner owns orchestration and evidence assembly. Runtime
Core and the Remote Desktop plugin remain the only owners of admission,
authority, lifecycle, target validation, and OS injection.
