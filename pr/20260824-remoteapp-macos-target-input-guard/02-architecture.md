# Architecture

```text
WebRTC input frame
  -> session transport/replay gate
  -> committed geometry revision + focus epoch gate
  -> macOS host target snapshot
       -> exact owner/window/application identity
       -> visible selected display/window set
       -> topmost focused target
       -> exact committed geometry/window-set
  -> Accessibility permission gate
  -> CGEvent post
  -> applied/rejected session event
```

`target_observer` remains the owner of host target truth and exposes a focused
validation operation over the same snapshot model used by lifecycle tracking.
`input` owns data-channel policy and OS injection. The frontend continues to
consume daemon-projected readiness and supplies epochs; it does not decide
whether the target is safe at execution time.
