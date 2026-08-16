# Invariants

1. `DirectWebRtcRouteCandidateProvider` remains the transport route abstraction. The endpoint consumes a provider; it does not assemble ad-hoc UDP addresses or parse environment variables directly.
2. Host bind endpoints and ICE server routes are distinct concepts. Only host candidates can become local UDP bind addresses.
3. STUN routes are server-reflexive route evidence and WebRTC ICE server config; they are not local bind addresses.
4. TURN routes are relay route evidence and WebRTC ICE server config. Credentials are used only for RTC configuration and must be redacted from public evidence.
5. EasyNet relay routes remain distinct from generic TURN. A WebRTC-compatible EasyNet relay may be represented as an ICE server, but public evidence must preserve `easynet_relay`.
6. Empty route configuration keeps the explicit `host_local_only` state.
7. Invalid configured route input fails closed at endpoint creation rather than silently producing a misleading route state.
8. All routable model text remains URA-only.
