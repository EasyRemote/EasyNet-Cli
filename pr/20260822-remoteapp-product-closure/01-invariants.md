# Invariants — RemoteApp Product Closure

1. User account is a Principal, not an Agent.
2. Device is an execution substrate and key custodian, not the public callee.
3. Device-native remote desktop abilities are SystemAgent-owned descriptors.
4. RemoteDesktopPlugin is an AbilityImpl, never a Principal or Agent.
5. Display/window/application Resource URA is the Invocation subject.
6. Session ids and tokens are args/access facts, not Invocation subjects.
7. WebRTC/native transport cannot bypass target binding, authority, receipt,
   sequence, lifecycle, or audit semantics.
8. App/window target loss must never fall back to display capture.
9. Input is disabled unless focus, coordinate mapping, permission, and target
   epoch are proven on the execution path.
10. Product-complete requires current authoritative evidence for each supported
    OS/network/frontend path; source-contract gates alone are insufficient.
11. Audio RTP backpressure never blocks the media control loop. Pending encoded
    audio is hard-bounded, stale packets are dropped before fresh packets, and
    the session owns cancellation of the only audio track writer.
12. `retry_session` preserves the public session id, daemon-local session token,
    consent grant, selected Resource URA, and event history. It may replace only
    the local/remote transport generation with a strictly newer transport epoch.
    Only an explicit `new_session_required` outcome may end the old session and
    mint a new session or consent grant. Every asynchronous retry continuation is
    fenced by a monotonic client generation so offline, end, reset, or a newer
    retry cannot be overwritten by stale session lookup or WebRTC negotiation.
13. Recovery persistence shares the session aggregate's terminal-row retention
    decision. A row removed from memory is deleted durably, snapshot reads and
    writes are capped before decode/allocation growth, and commit/load/delete
    serialize through one store lock. Startup derives its row ceiling from the
    same active/terminal retention formula and also caps total bytes and every
    directory entry before JSON decode.
14. Browser input frames and the plugin parser share one closed field set.
    Frontend-only pointer metadata must never cross the data channel, because
    the host parser rejects unknown fields before admission and OS injection.
    Every successfully applied key/button press is session-transport state:
    its matching release is allowed to reduce input state even after target
    focus changes, and channel termination attempts a bounded release of every
    still-pressed key/button before the input plane closes.
