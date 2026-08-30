# Invariants

1. Browser ICE server config is projected only as RemoteApp transport metadata.
2. Public route evidence still redacts credentials.
3. Authorized session views may carry the RTC ICE server fields required by the
   browser (`urls`, `username`, `credential`) because the browser must present
   relay credentials to use TURN.
4. If route environment config is invalid, the view reports a typed
   configuration error instead of silently pretending relay is unavailable.
5. Product readiness remains incomplete until real direct, STUN, TURN, and
   EasyNet relay deployment reports exist.
