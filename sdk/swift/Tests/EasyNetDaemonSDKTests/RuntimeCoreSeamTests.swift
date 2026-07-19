import Foundation
import XCTest
@testable import EasyNetDaemonSDK

final class RuntimeCoreSeamTests: XCTestCase {
    private let caller = "easynet:///r/example/agent/alice.sdk"
    private let callee = "easynet:///r/example/device/dev-a"
    private let descriptor = "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
    private let nonce = "AQIDBAUGBwgJCgsMDQ4PEA=="

    func testProductNeutralModuleExportsOnlyGenericRuntimeConcepts() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let sourceDirectory = packageRoot.appendingPathComponent("Sources/EasyNetDaemonSDK")
        let sourceNames = Set(
            try FileManager.default.contentsOfDirectory(atPath: sourceDirectory.path)
                .filter { $0.hasSuffix(".swift") }
        )
        XCTAssertEqual(
            sourceNames,
            Set([
                "Authority.swift",
                "Bidi.swift",
                "Client.swift",
                "Health.swift",
                "Invocation.swift",
                "JSONValue.swift",
                "Runtime.swift",
                "SDKError.swift",
                "Stream.swift",
            ])
        )

        let forbiddenExports = [
            "AdminClient",
            "CompanionClient",
            "CompatibilityClient",
            "DirectoryClient",
            "IdentityClient",
            "EventClient",
            "HostBindingClient",
            "MissionClient",
            "PublicationClient",
            "ReceiptClient",
            "SurfaceClient",
            "WrapperClient",
        ]
        for sourceName in sourceNames {
            let source = try String(
                contentsOf: sourceDirectory.appendingPathComponent(sourceName),
                encoding: .utf8
            )
            for forbidden in forbiddenExports {
                XCTAssertFalse(source.contains(forbidden), "\(sourceName) exports \(forbidden)")
            }
        }
    }

    func testDiscoveryAndLifecycleAreExplicit() async throws {
        let transport = MemoryDiscoveryTransport()
        let client = Client(transport: transport)
        let features = try await client.requireABI(5)
        XCTAssertEqual(features.profiles["runtime_core"], "seam")
        XCTAssertEqual(features.symbols["runtime_prepare"], true)
        await expectSDKError(.versionIncompatible) {
            _ = try await client.requireABI(4)
        }

        try await client.close()
        try await client.close()
        let transportClosed = await transport.isClosed()
        XCTAssertTrue(transportClosed)
        await expectSDKError(.invalidHandle) {
            _ = try await client.featureDiscovery()
        }
    }

    func testHealthSeparatesLivenessFromReadiness() async throws {
        let health = HealthClient(transport: MemoryHealthTransport())
        let state = try await health.runtimeHealth()
        XCTAssertTrue(state.apiAlive)
        XCTAssertFalse(state.ready)
        XCTAssertEqual(state.abiVersion, 5)
        XCTAssertEqual(state.diagnostics, ["runtime warming"])

        let diagnostics = try await health.diagnostics()
        XCTAssertEqual(diagnostics.profile, "health")
        XCTAssertEqual(diagnostics.checks.count, 1)
        XCTAssertFalse(diagnostics.ready)

        try await health.close()
        await expectSDKError(.invalidHandle) {
            _ = try await health.runtimeHealth()
        }
    }

    func testInvocationPrepareSignSubmitPreservesCompleteTuple() async throws {
        let transport = MemoryRuntimeTransport(descriptor: descriptor)
        let runtime = RuntimeClient(transport: transport)
        let draft = try completeDraft(runtime)

        let result = try await runtime.invoke(draft)
        XCTAssertTrue(result.ok)
        XCTAssertEqual(result.terminalReceipt["receipt_ref"], "opaque-receipt-ref")

        let prepared = try await runtime.prepare(draft, options: ["deadline_ms": 1000])
        XCTAssertFalse(prepared.submitReady())
        XCTAssertEqual(prepared.tuple().caller, caller)
        XCTAssertEqual(prepared.tuple().descriptorRef, descriptor)

        let signature = try InvocationSignature(
            algorithm: "ed25519",
            signatureBase64: "c2lnbmF0dXJl",
            keyIdHint: "caller-key-1"
        )
        let signed = try prepared.signWithCallerSignature(signature)
        XCTAssertTrue(signed.submitReady())
        let handle = try await signed.submit()
        XCTAssertFalse(handle.terminal)
        let awaited = try await runtime.awaitResult(handle)
        XCTAssertTrue(awaited.ok)
        let cancelled = try await runtime.cancel(handle, reason: "done")
        XCTAssertTrue(cancelled.requestAccepted)
        XCTAssertTrue(cancelled.terminal)
        let events = try await runtime.events(handle)
        XCTAssertTrue(events.terminal)
        try await runtime.closeHandle(handle)
        let submittedSigner = await transport.submittedSigner()
        XCTAssertEqual(submittedSigner, "caller-key-1")
        let forged = try InvocationHandle.fromJSON(
            Data(#"{"handle_id":7,"state":"Running","terminal":false}"#.utf8)
        )
        await expectSDKError(.invalidArgument) {
            _ = try await runtime.awaitResult(forged)
        }
        await transport.setEventHandleId(8)
        await expectSDKError(.invalidArgument) {
            _ = try await runtime.events(handle)
        }

        await expectSDKError(.invalidArgument) {
            _ = try await runtime.submitSigned(prepared)
        }
        try await runtime.close()
        expectSyncSDKError(.invalidHandle) {
            _ = try runtime.newInvocation()
        }
    }

    func testInvocationResultUsesTerminalReceipt() throws {
        let canonical = try InvocationResult.fromJSON(
            Data(
                #"{"ok":true,"terminal_state":"Completed","terminal_receipt":{"receipt_ref":"canonical-terminal"}}"#
                    .utf8
            )
        )
        XCTAssertEqual(canonical.terminalReceipt["receipt_ref"], "canonical-terminal")

        let legacyOnly = try InvocationResult.fromJSON(
            Data(
                #"{"ok":true,"terminal_state":"Completed","receipt":{"receipt_ref":"legacy-only"}}"#
                    .utf8
            )
        )
        XCTAssertTrue(legacyOnly.terminalReceipt.isEmpty)
    }

    func testAuthorityMetadataIsTypedAndMutuallyExclusive() async throws {
        let delegationValue = try delegationMetadataValue()
        let sessionValue = try sessionMetadataValue()
        let authority = AuthorityClient(
            transport: MemoryAuthorityTransport(
                delegationValue: delegationValue,
                sessionValue: sessionValue
            )
        )

        let delegation = try await authority.mintDelegationProof(
            DelegationRequest(
                issuerURA: "easynet:///r/example/user/alice",
                subjectURA: "easynet:///r/example/user/alice",
                callerURA: caller,
                audience: callee,
                scopes: ["invoke"],
                issuedAtMS: 10,
                expiresAtMS: 20,
                metadata: ["trace": .string("delegation")]
            )
        )
        XCTAssertEqual(delegation.metadataValue, delegationValue)

        let session = try await authority.mintSessionAuthority(
            SessionAuthorityRequest(
                issuerURA: caller,
                sessionID: "session-1",
                sessionOwnerUserID: "alice",
                creatorPrincipalID: caller,
                calleeURA: callee,
                subjectURA: "easynet:///r/example/user/alice",
                audience: callee,
                scopes: ["invoke"],
                allowedActions: ["invoke"],
                allowedFollowupAbilities: ["observe.health"],
                issuedAtMS: 10,
                expiresAtMS: 20,
                metadata: ["trace": .string("session")]
            )
        )
        XCTAssertEqual(session.metadataValue, sessionValue)

        let authorized = try completeBuilder()
            .withAuthorityMetadata(delegation.metadata())
            .inspect()
        XCTAssertEqual(
            authorized.inspectTuple().metadata[delegationMetadataKey],
            .string(delegationValue)
        )

        expectSyncSDKError(.invalidArgument) {
            _ = try completeBuilder()
                .withMetadata([
                    delegationMetadataKey: .string(delegationValue),
                    sessionAuthorityMetadataKey: .string(sessionValue),
                ])
                .inspect()
        }

        try await authority.close()
        await expectSDKError(.invalidHandle) {
            _ = try await authority.mintDelegationProof(
                DelegationRequest(
                    issuerURA: "easynet:///r/example/user/alice",
                    subjectURA: "easynet:///r/example/user/alice",
                    callerURA: self.caller,
                    audience: self.callee,
                    scopes: ["invoke"],
                    issuedAtMS: 10,
                    expiresAtMS: 20
                )
            )
        }
    }

    func testAuthorityMetadataRejectsAllZeroSessionOwners() throws {
        let value = try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "00000000-0000-0000-0000-000000000000",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/user/alice",
            "audience": callee,
            "scopes": ["invoke"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])

        expectSyncSDKError(.invalidArgument) {
            _ = try SessionAuthority.fromMetadata(value)
        }
    }

    func testStreamAndBidiStateMachinesAreBounded() async throws {
        let stream = StreamHandle(source: CountingStreamSource())
        for _ in 0...StreamHandle.maxRetainedEvents {
            _ = try await stream.next()
        }
        XCTAssertNil(stream.terminalEvent())
        XCTAssertEqual(stream.transportTerminalEvent()?.state, "Failed")
        XCTAssertEqual(stream.transportTerminalEvent()?.transportTerminal, true)
        XCTAssertEqual(stream.retainedEvents().count, StreamHandle.maxRetainedEvents + 1)
        try await stream.close()
        await expectSDKError(.invalidHandle) {
            _ = try await stream.next()
        }

        let bidi = BidiSession(source: CountingBidiSource())
        try await bidi.send(.data(0, payloadJSON: "{\"hello\":true}"))
        let sendClosed = try await bidi.closeSend()
        XCTAssertFalse(sendClosed.terminal)
        XCTAssertTrue(sendClosed.transportTerminal)
        await expectSDKError(.cancelled) {
            try await bidi.send(.data(1, payloadJSON: "{}"))
        }
        let received = try await bidi.next()
        XCTAssertFalse(received.terminal)
        let cancelled = try await bidi.cancel(reason: "done")
        XCTAssertFalse(cancelled.terminal)
        XCTAssertTrue(cancelled.transportTerminal)
        try await bidi.close()
    }

    func testBidiFrame0IsRequiredBeforeRuntimeSessionEntry() async throws {
        let transport = MemoryRuntimeTransport(descriptor: descriptor)
        let runtime = RuntimeClient(transport: transport)
        await expectSDKError(.invalidArgument) {
            _ = try await runtime.openBidi(try self.completeDraft(runtime), frame0: nil)
        }
        let openedBidi = await transport.openedBidiCount()
        XCTAssertEqual(openedBidi, 0)
    }

    func testTypedErrorsPreserveStableCategories() {
        XCTAssertEqual(SDKError.validation("test", "bad").errorClass, .validation)
        XCTAssertEqual(
            SDKError(code: .authorityDenied, stage: "admission", message: "denied").errorClass,
            .admission
        )
        XCTAssertEqual(
            SDKError(code: .routeUnavailable, stage: "routing", message: "missing").errorClass,
            .availability
        )
    }

    func testABICompatibleAcceptsExactVersion() async throws {
        let client = Client(transport: MemoryDiscoveryTransport())
        let features = try await client.requireABI(5)
        XCTAssertEqual(features.abiVersion, 5)
    }

    func testABIIncompatibleRejectsMismatch() async throws {
        let client = Client(transport: MemoryDiscoveryTransport())
        await expectSDKError(.versionIncompatible) { _ = try await client.requireABI(4) }
    }

    func testRetryHintsPreserveRetryability() {
        let safe = SDKError(code: .timeout, stage: "execution", retryHint: .safe, retryable: true, message: "timeout")
        let never = SDKError.validation("input", "bad")
        XCTAssertEqual(safe.retryHint, .safe)
        XCTAssertTrue(safe.retryable)
        XCTAssertEqual(never.retryHint, .never)
        XCTAssertFalse(never.retryable)
    }

    func testCanonicalSigningMaterialComesFromPrepare() async throws {
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport(descriptor: descriptor))
        let prepared = try await runtime.prepare(completeDraft(runtime), options: ["deadline_ms": 1000])
        XCTAssertEqual(prepared.signingMaterial.descriptorRef, descriptor)
        XCTAssertEqual(Data(base64Encoded: prepared.signingMaterial.canonicalBytesBase64), Data("canonical".utf8))
    }

    func testCompleteTupleRejectsMissingCaller() {
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationBuilder()
                .withCalleeURA(callee)
                .withDescriptorRef(descriptor)
                .withSubjectURA(callee)
                .withNonce(nonce)
                .withCausalContext("{\"form\":\"none\"}")
                .withArgsJSON("{}")
                .inspect()
        }
    }

    func testPreparedInvocationCannotBeSubmitted() async throws {
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport(descriptor: descriptor))
        let prepared = try await runtime.prepare(completeDraft(runtime), options: ["deadline_ms": 1000])
        await expectSDKError(.invalidArgument) { _ = try await runtime.submitSigned(prepared) }
    }

    func testStreamAndBidiBackpressureAreBounded() async throws {
        let stream = StreamHandle(source: CountingStreamSource())
        for _ in 0...StreamHandle.maxRetainedEvents { _ = try await stream.next() }
        XCTAssertNil(stream.terminalEvent())
        XCTAssertEqual(stream.transportTerminalEvent()?.transportTerminal, true)
        let bidi = BidiSession(source: CountingBidiSource())
        for _ in 0...BidiSession.maxRetainedFrames { _ = try await bidi.next() }
        XCTAssertNil(bidi.terminalFrame())
        XCTAssertEqual(bidi.transportTerminalFrame()?.kind, "backpressure_terminated")
        XCTAssertEqual(bidi.transportTerminalFrame()?.transportTerminal, true)
    }

    func testStreamOrderAndTerminalArePreserved() async throws {
        let stream = StreamHandle(source: OrderedTerminalStreamSource())
        let first = try await stream.next()
        XCTAssertEqual(first.sequence, 0)
        let terminal = try await stream.next()
        XCTAssertEqual(terminal.sequence, 1)
        XCTAssertTrue(terminal.terminal)
    }

    private func completeBuilder() -> InvocationBuilder {
        InvocationBuilder()
            .withCallerURA(caller)
            .withCalleeURA(callee)
            .withDescriptorRef(descriptor)
            .withSubjectURA(callee)
            .withNonce(nonce)
            .withCausalContext("{\"form\":\"none\"}")
            .withArgsJSON("{\"probe\":true}")
            .withMetadata(["trace_id": .string("trace-1")])
    }

    private func completeDraft(_ runtime: RuntimeClient) throws -> InvocationDraft {
        try runtime.newInvocation()
            .withCallerURA(caller)
            .withCalleeURA(callee)
            .withDescriptorRef(descriptor)
            .withSubjectURA(callee)
            .withNonce(nonce)
            .withCausalContext("{\"form\":\"none\"}")
            .withArgsJSON("{\"probe\":true}")
            .withMetadata(["trace_id": .string("trace-1")])
            .build()
    }

    private func delegationMetadataValue() throws -> String {
        try authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": "easynet:///r/example/user/alice",
            "caller_ura": caller,
            "audience": callee,
            "scopes": ["invoke"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])
    }

    private func sessionMetadataValue() throws -> String {
        try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "alice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/user/alice",
            "audience": callee,
            "scopes": ["invoke"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])
    }

    private func authorityMetadataValue(_ payload: [String: Any]) throws -> String {
        let signature = Data("signature".utf8).base64EncodedString()
        let data = try JSONSerialization.data(
            withJSONObject: ["payload": payload, "signature": signature],
            options: [.sortedKeys]
        )
        return data.base64EncodedString()
    }
}

actor MemoryDiscoveryTransport: DiscoveryTransport {
    private var closed = false

    func featureDiscovery() throws -> FeatureSet {
        try FeatureSet(
            abiVersion: 5,
            sdkVersion: "0.0.0-seam",
            profiles: ["runtime_core": "seam", "health": "seam", "authority": "seam"],
            symbols: ["runtime_prepare": true, "runtime_submit_signed": true]
        )
    }

    func close() {
        closed = true
    }

    func isClosed() -> Bool {
        closed
    }
}

actor MemoryHealthTransport: HealthTransport, DiagnosticsTransport {
    func runtimeHealth() -> Data {
        Data(
            """
            {
              "api_ready": true,
              "invocation_ready": false,
              "directory_ready": false,
              "trust_ready": true,
              "runtime_ready": false,
              "version": "0.0.0-seam",
              "abi_version": 5,
              "mismatch": null,
              "diagnostics": ["runtime warming"]
            }
            """.utf8
        )
    }

    func runtimeDiagnostics() -> Data {
        Data(
            """
            {
              "profile": "health",
              "kind": "diagnostics_report",
              "state": "Running",
              "ready": false,
              "version": "0.0.0-seam",
              "abi_version": 5,
              "control_endpoint": "/tmp/easynet-control.sock",
              "invocation_endpoint": "/tmp/easynet-daemon.sock",
              "checks": [{"name":"runtime","ready":false,"message":"warming"}],
              "diagnostics": ["runtime warming"]
            }
            """.utf8
        )
    }

    func close() {}
}

actor MemoryRuntimeTransport: RuntimeTransport {
    private let descriptor: String
    private var signer = ""
    private var eventHandleId: Int64 = 7
    private var openedBidi = 0

    init(descriptor: String) {
        self.descriptor = descriptor
    }

    func invoke(_ draft: InvocationDraft) throws -> InvocationResult {
        try InvocationResult(
            ok: true,
            terminalState: .completed,
            outputJSON: "{\"ok\":true}",
            terminalReceipt: ["receipt_ref": "opaque-receipt-ref", "receipt_hash": "opaque-hash"]
        )
    }

    func prepare(_ draftJSON: Data, optionsJSON: Data) throws -> Data {
        let tuple = try object(draftJSON)
        let options = try object(optionsJSON)
        XCTAssertEqual((options["deadline_ms"] as? NSNumber)?.intValue, 1000)
        return try JSONSerialization.data(
            withJSONObject: [
                "prepared_id": "prepared-1",
                "request_id": "request-1",
                "tuple": tuple,
                "signing_material": [
                    "algorithm": "ed25519",
                    "canonical_bytes_base64": "Y2Fub25pY2Fs",
                    "args_digest_hex": String(repeating: "a", count: 64),
                    "descriptor_ref": descriptor,
                    "expires_at_unix_ms": 4_102_444_800_000,
                ],
                "descriptor_ref": descriptor,
                "descriptor_hash_hex": "",
                "schema_hash_hex": "",
                "canonical_hash_hex": "",
                "expires_at_unix_ms": 4_102_444_800_000,
                "submit_ready": false,
            ],
            options: [.sortedKeys]
        )
    }

    func submitSigned(_ signedJSON: Data) throws -> Data {
        let signed = try object(signedJSON)
        signer = signed["signer_id"] as? String ?? ""
        return try JSONSerialization.data(
            withJSONObject: ["handle_id": 7, "state": "Running", "terminal": false],
            options: [.sortedKeys]
        )
    }

    func awaitHandle(_ control: InvocationControlCapability) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [
                "ok": true,
                "terminal_state": "Completed",
                "output_json": ["done": true],
                "terminal_receipt": ["receipt_ref": "opaque-receipt-ref"],
            ],
            options: [.sortedKeys]
        )
    }

    func cancelHandle(_ control: InvocationControlCapability, reason: String) throws -> Data {
        let handleId = try control.adapterHandleId()
        return try JSONSerialization.data(
            withJSONObject: [
                "handle_id": handleId,
                "request_accepted": true,
                "deduplicated": false,
                "cancelled": true,
                "state": "Cancelled",
                "terminal": true,
            ],
            options: [.sortedKeys]
        )
    }

    func handleEvents(_ control: InvocationControlCapability) throws -> Data {
        let handleId = eventHandleId
        _ = try control.adapterHandleId()
        return try JSONSerialization.data(
            withJSONObject: [
                "handle_id": handleId,
                "state": "Completed",
                "terminal": true,
            ],
            options: [.sortedKeys]
        )
    }

    func freeHandle(_ control: InvocationControlCapability) throws {}

    func openStream(_ draft: InvocationDraft) -> StreamSource {
        CountingStreamSource()
    }

    func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) -> BidiSource {
        openedBidi += 1
        return CountingBidiSource(initial: frame0)
    }

    func close() {}

    func submittedSigner() -> String {
        signer
    }

    func setEventHandleId(_ handleId: Int64) {
        eventHandleId = handleId
    }

    func openedBidiCount() -> Int {
        openedBidi
    }
}

actor MemoryAuthorityTransport: AuthorityTransport {
    private let delegationValue: String
    private let sessionValue: String

    init(delegationValue: String, sessionValue: String) {
        self.delegationValue = delegationValue
        self.sessionValue = sessionValue
    }

    func mintDelegationProof(_ requestJSON: Data) throws -> Data {
        XCTAssertFalse(requestJSON.isEmpty)
        return try JSONSerialization.data(
            withJSONObject: ["metadata_value": delegationValue],
            options: [.sortedKeys]
        )
    }

    func mintSessionAuthority(_ requestJSON: Data) throws -> Data {
        XCTAssertFalse(requestJSON.isEmpty)
        return try JSONSerialization.data(
            withJSONObject: ["metadata": [sessionAuthorityMetadataKey: sessionValue]],
            options: [.sortedKeys]
        )
    }

    func close() {}
}

actor CountingStreamSource: StreamSource {
    private var sequence = 0

    func next() throws -> StreamEvent {
        defer { sequence += 1 }
        return try .data(sequence, payloadJSON: "{\"sequence\":\(sequence)}")
    }

    func cancel(reason: String) throws -> StreamEvent {
        try .transportTerminal(sequence, kind: "cancel_requested", state: "CancelRequested")
    }

    func close() {}
}

actor OrderedTerminalStreamSource: StreamSource {
    private var sequence = 0

    func next() throws -> StreamEvent {
        defer { sequence += 1 }
        return sequence == 0 ? try .data(0, payloadJSON: "{}") : try .terminal(1, state: "Completed")
    }
}

actor CountingBidiSource: BidiSource {
    private var frames: [BidiFrame]
    private var sequence = 1

    init(initial: BidiFrame? = nil) {
        frames = initial.map { [$0] } ?? []
    }

    func send(_ frame: BidiFrame) {
        frames.append(frame)
    }

    func next() throws -> BidiFrame {
        if !frames.isEmpty {
            return frames.removeFirst()
        }
        defer { sequence += 1 }
        return try .data(sequence, payloadJSON: "{}")
    }

    func closeSend() throws -> BidiFrame {
        try .transportTerminal(sequence, kind: "send_closed")
    }

    func cancel(reason: String) throws -> BidiFrame {
        try .transportTerminal(sequence, kind: "cancel_requested")
    }

    func close() {}
}

private func object(_ data: Data) throws -> [String: Any] {
    guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw SDKError.validation("test", "JSON object required")
    }
    return object
}

private func expectSyncSDKError(
    _ code: SDKErrorCode,
    operation: () throws -> Void
) {
    do {
        try operation()
        XCTFail("expected SDKError \(code.rawValue)")
    } catch let error as SDKError {
        XCTAssertEqual(error.code, code)
    } catch {
        XCTFail("expected SDKError, got \(error)")
    }
}

private func expectSDKError(
    _ code: SDKErrorCode,
    operation: () async throws -> Void
) async {
    do {
        try await operation()
        XCTFail("expected SDKError \(code.rawValue)")
    } catch let error as SDKError {
        XCTAssertEqual(error.code, code)
    } catch {
        XCTFail("expected SDKError, got \(error)")
    }
}
