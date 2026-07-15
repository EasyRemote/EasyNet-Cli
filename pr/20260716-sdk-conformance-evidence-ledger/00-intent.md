## Intent

Close the SDK conformance evidence-ledger fork exposed by
`check-sdk-conformance-reports.sh`: adapter report records referenced stale
source hashes while the current Go, Python, Java, and Swift evidence files are
the active proof artifacts.

This slice updates only conformance proof metadata. Runtime behavior, public
SDK interfaces, and language facade code remain unchanged.
