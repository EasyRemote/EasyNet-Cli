# Intent

Keep device session liveness aligned with the admitted reverse-bidi contract.

The pre-admission watchdog bounds silent `session.open` attempts. After the Hub sends `SessionEstablished`, ordinary down-stream silence is valid stream membership and must not be interpreted as device offline.

