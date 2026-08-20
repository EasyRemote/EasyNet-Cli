# Companion SDK Convergence Verification

## Commands

```sh
swift test --filter RuntimeCoreSeamTests/testCompanionProfileProjectsStateMachineAndLifecycleActions
classes=/tmp/easynet-java-sdk-classes && rm -rf "$classes" && mkdir -p "$classes" && javac -d "$classes" $(find sdk/java/src/main/java sdk/java/src/test/java -name '*.java') && java -cp "$classes" run.easynet.daemon.RuntimeCoreSeamTest
go test -tags easynet_cabi . -run 'TestCABICompanion|TestCABIRuntimeTransportDrivesStreamAndBidiCallbacks'
git diff --check
rg -n "U[R]I|u[r]i" sdk/swift/Sources/EasyNetDaemonSDK/Companion.swift sdk/java/src/main/java/run/easynet/daemon/Companion*.java sdk/java/src/main/java/run/easynet/daemon/DesktopCompanion*.java sdk/go/cabi_runtime.go sdk/go/cabi_runtime_test.go
```

## Results

- Swift companion seam test passed.
- Java compile plus runtime-core seam suite passed.
- Go C ABI companion and stream/bidi tests passed.
- Whitespace check passed.
- URA terminology audit on touched SDK files returned no matches.
