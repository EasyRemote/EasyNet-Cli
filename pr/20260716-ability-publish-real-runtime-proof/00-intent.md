# Intent

`ability.publish` is a DeviceLifecycle local RPC used by the curator and daemon
control paths to persist an Agent ability manifest. Existing real-invoke
coverage only proved that the route was present by checking the missing-argument
error was not a dispatcher miss.

This slice strengthens the proof: a realistic LocalRuntime dispatch writes the
manifest and returns the public publish envelope.
