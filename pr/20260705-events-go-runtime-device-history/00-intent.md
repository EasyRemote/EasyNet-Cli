# Intent

Implement Go Events runtime device-history execution as a daemon-owned
Runtime Core adapter.

This closes the gap where `EventClient.ListDeviceEvents` worked for memory/C
ABI transports but `NewRuntimeEventClient` reported `NOT_IMPLEMENTED`, even
though the daemon SDK already owns the `events.device.history` carrier and
device-event page projection contract.
