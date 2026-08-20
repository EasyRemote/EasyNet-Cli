# Intent

Converge Go and Python direct runtime transports on one capability boundary.

Direct gRPC owns unary, stream, and bidi dispatch only. The configured runtime
handle transport owns prepare, signed submission, and invocation-handle
lifecycle operations. Go must not locally manufacture prepared invocations or
terminal handle snapshots when that owner is absent.

Removal condition: no Go direct runtime source contains a synthetic direct
handle store or direct prepare/submit fallback.
