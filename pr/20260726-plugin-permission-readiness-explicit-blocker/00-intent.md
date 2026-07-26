# Intent

Remove the plugin realtime permission readiness compatibility state that reports a declared-but-unhandled permission requirement as `unknown`.

When a plugin declares permissions and neither a status nor request ability is available, the runtime has a deterministic answer: activation is blocked by a missing policy action path.
