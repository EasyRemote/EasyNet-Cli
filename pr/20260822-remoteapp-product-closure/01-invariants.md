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
