import Foundation
import XCTest
@testable import EasyNetDaemonSDK

final class RuntimeCoreSeamTests: XCTestCase {
    func testFeatureDiscoveryAndTypedErrors() async throws {
        let transport = MemoryDiscoveryTransport()
        let client = Client(transport: transport)
        let features = try await client.requireABI(4)
        XCTAssertEqual(features.profiles["runtime_core"], "seam")
        await expectSDKError(.versionIncompatible) {
            _ = try await client.requireABI(5)
        }
        try await client.close()
        await expectSDKError(.invalidHandle) {
            _ = try await client.featureDiscovery()
        }
        XCTAssertEqual(SDKError.validation("x", "bad").errorClass, .validation)
    }

    func testCompleteInvocationDraftAndRuntimeDispatch() async throws {
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport())
        let draft = try runtime.newInvocation()
            .withCallerURA("easynet:///r/example/agent/alice")
            .withCalleeURA("easynet:///r/example/agent/bob")
            .withDescriptorRef("easynet:///r/example/ability/bob.echo@1.0.0")
            .withSubjectURA("easynet:///r/example/resource/message")
            .withNonce("n-1")
            .withCausalContext("root")
            .withArgsJSON("{\"text\":\"hi\"}")
            .build()
        XCTAssertTrue(draft.inspectTuple().descriptorRef.hasSuffix("@1.0.0"))
        let result = try await runtime.invoke(draft)
        XCTAssertEqual(result.terminalState, .completed)
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationBuilder().withCallerURA("x").build()
        }
        try await runtime.close()
        expectSyncSDKError(.invalidHandle) {
            _ = try runtime.newInvocation()
        }
    }

    func testPreparedInvocationSeparatesCanonicalMaterialFromSignedSubmit() async throws {
        final class SigningTransport: MemoryRuntimeTransport, @unchecked Sendable {
            var seenDraft: [String: Any] = [:]
            var seenOptions: [String: Any] = [:]
            var seenSigned: [String: Any] = [:]
            var submitTouched = false

            override func prepare(_ draftJSON: Data, optionsJSON: Data) async throws -> Data {
                seenDraft = try decodedObject(draftJSON)
                seenOptions = try decodedObject(optionsJSON)
                return fixture("prepared.signing-material.v4.json")
            }

            override func submitSigned(_ signedJSON: Data) async throws -> Data {
                submitTouched = true
                seenSigned = try decodedObject(signedJSON)
                return Data("{\"handle_id\":7,\"state\":\"Submitted\",\"terminal\":false}".utf8)
            }
        }

        let transport = SigningTransport()
        let runtime = RuntimeClient(transport: transport)
        let draft = try runtime.newInvocation()
            .withCallerURA("easynet:///r/example/agent/alice.sdk")
            .withCalleeURA("easynet:///r/example/device/dev-a")
            .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
            .withSubjectURA("easynet:///r/example/device/dev-a")
            .withNonce("AQIDBAUGBwgJCgsMDQ4PEA==")
            .withCausalContext("{\"form\":\"none\"}")
            .withArgsJSON("{}")
            .build()

        let prepared = try await runtime.prepare(draft, options: ["deadline_unix_ms": 1_783_000_000_000])
        XCTAssertFalse(prepared.submitReady())
        XCTAssertEqual(prepared.preparedId, "prepared-example-1")
        XCTAssertEqual(prepared.signingMaterial.canonicalBytesBase64, "ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM=")
        XCTAssertEqual(transport.seenDraft["descriptor_ref"] as? String, prepared.signingMaterial.descriptorRef)
        XCTAssertEqual((transport.seenOptions["deadline_unix_ms"] as? NSNumber)?.int64Value, 1_783_000_000_000)

        await expectSDKError(.invalidArgument) {
            _ = try PreparedInvocation.fromJSON(
                replacingFixture("prepared.signing-material.v4.json", "\"submit_ready\": false", "\"submit_ready\": true")
            )
        }
        await expectSDKError(.invalidArgument) {
            _ = try PreparedInvocation.fromJSON(preparedFixtureWithMismatchedSigningDescriptor())
        }

        await expectSDKError(.invalidArgument) {
            _ = try await runtime.submitSigned(prepared)
        }
        XCTAssertFalse(transport.submitTouched)

        let signed = try prepared.signWithCallerSignature(
            InvocationSignature(algorithm: "ed25519", signatureBase64: "c2lnbmF0dXJl", keyIdHint: "signer-alice-key-1")
        )
        XCTAssertTrue(signed.submitReady())
        let handle = try await runtime.submitSigned(signed)
        XCTAssertEqual(handle.handleId, 7)
        XCTAssertTrue(transport.submitTouched)
        let signedPrepared = try XCTUnwrap(transport.seenSigned["prepared"] as? [String: Any])
        let signature = try XCTUnwrap(transport.seenSigned["signature"] as? [String: Any])
        XCTAssertEqual(transport.seenSigned["signer_id"] as? String, "signer-alice-key-1")
        XCTAssertEqual(signature["signature_base64"] as? String, "c2lnbmF0dXJl")
        XCTAssertEqual(
            signedPrepared["canonical_bytes_base64"] as? String,
            prepared.signingMaterial.canonicalBytesBase64
        )
    }

    func testStreamHistoryIsBounded() async throws {
        let handle = StreamHandle(source: QueueStreamSource(count: StreamHandle.maxRetainedEvents + 2))
        for _ in 0..<(StreamHandle.maxRetainedEvents + 2) {
            _ = try await handle.next()
        }
        XCTAssertEqual(handle.terminalEvent()?.error?.code, .transport)
        XCTAssertEqual(handle.retainedEvents().count, StreamHandle.maxRetainedEvents + 1)
        try await handle.close()
    }

    func testStreamHandleIsAsyncSequence() async throws {
        let handle = StreamHandle(source: QueueStreamSource(count: 2))
        var sequences: [Int] = []
        var terminalCount = 0
        for try await event in handle {
            sequences.append(event.sequence)
            if event.terminal {
                terminalCount += 1
            }
        }
        XCTAssertEqual(sequences, [0, 1, 9999])
        XCTAssertEqual(terminalCount, 1)
        XCTAssertEqual(handle.terminalEvent()?.state, "Completed")
        try await handle.close()
    }

    func testBidiHistoryIsBounded() async throws {
        let session = BidiSession(source: QueueBidiSource(count: BidiSession.maxRetainedFrames + 2))
        try await session.send(.data(0, payloadJSON: "{\"hello\":true}"))
        for _ in 0..<(BidiSession.maxRetainedFrames + 2) {
            _ = try await session.next()
        }
        XCTAssertEqual(session.terminalFrame()?.kind, "backpressure_terminated")
        XCTAssertEqual(session.retainedFrames().count, BidiSession.maxRetainedFrames + 1)
        try await session.close()
    }

    func testBidiSessionIsAsyncSequence() async throws {
        let session = BidiSession(source: QueueBidiSource(count: 2))
        var sequences: [Int] = []
        var terminalCount = 0
        for try await frame in session {
            sequences.append(frame.sequence)
            if frame.terminal {
                terminalCount += 1
            }
        }
        XCTAssertEqual(sequences, [0, 1, 9999])
        XCTAssertEqual(terminalCount, 1)
        XCTAssertEqual(session.terminalFrame()?.kind, "completed")
        try await session.close()
    }

    func testBidiCloseSendKeepsReceiveOpenAndRejectsFurtherSend() async throws {
        let session = BidiSession(source: QueueBidiSource(count: 1))
        try await session.send(.data(0, payloadJSON: "{\"hello\":true}"))
        let closeSend = try await session.closeSend()
        XCTAssertEqual(closeSend.kind, "send_closed")
        await expectSDKError(.cancelled) {
            try await session.send(.data(1, payloadJSON: "{\"after\":true}"))
        }
        let received = try await session.next()
        XCTAssertTrue(received.payloadJSON.contains("\"n\":0"))
        try await session.close()
        try await session.close()
        await expectSDKError(.invalidHandle) {
            _ = try await session.next()
        }
    }

    func testRuntimeHealthDistinguishesLivenessFromReadiness() async throws {
        let client = HealthClient(transport: MemoryHealthTransport(
            healthJSON: """
            {
              "api_ready": true,
              "daemon_ready": true,
              "invocation_ready": false,
              "directory_ready": true,
              "trust_ready": true,
              "runtime_ready": false,
              "version": "0.0.0-seam",
              "abi_version": 4,
              "mismatch": null,
              "diagnostics": ["invocation endpoint unavailable"]
            }
            """,
            diagnosticsJSON: """
            {
              "profile": "health",
              "kind": "diagnostics_report",
              "state": "Running",
              "ready": true,
              "version": "0.0.0-seam",
              "abi_version": 4,
              "control_endpoint": "/tmp/easynet/control.json",
              "invocation_endpoint": "/tmp/easynet/daemon.sock",
              "checks": [{"name": "runtime", "ready": true, "message": null}],
              "diagnostics": []
            }
            """
        ))

        let health = try await client.runtimeHealth()
        XCTAssertTrue(health.apiAlive)
        XCTAssertFalse(health.ready)
        XCTAssertFalse(health.invocationReady)
        XCTAssertEqual(health.abiVersion, 4)

        let diagnostics = try await client.diagnostics()
        XCTAssertEqual(diagnostics.kind, "diagnostics_report")
        XCTAssertEqual(diagnostics.checks.count, 1)
    }

    func testRuntimeDiagnosticsRequireTransportCapability() async throws {
        let client = HealthClient(transport: HealthOnlyTransport(healthJSON: """
        {
          "api_ready": true,
          "daemon_ready": true,
          "invocation_ready": true,
          "directory_ready": true,
          "trust_ready": true,
          "runtime_ready": true,
          "diagnostics": []
        }
        """))
        await expectSDKError(.notImplemented) {
            _ = try await client.diagnostics()
        }
    }

    func testRuntimeHealthWrapsTransportFailures() async throws {
        let client = HealthClient(transport: FailingHealthTransport())
        await expectSDKError(.transport) {
            _ = try await client.runtimeHealth()
        }
    }

    func testRuntimeHealthRejectsMalformedPayload() async throws {
        let client = HealthClient(transport: HealthOnlyTransport(
            healthJSON: "{\"api_ready\": true, \"runtime_ready\": false}"
        ))
        await expectSDKError(.invalidArgument) {
            _ = try await client.runtimeHealth()
        }
    }

    func testRuntimeHealthRejectsClosedClient() async throws {
        let transport = HealthOnlyTransport(healthJSON: """
        {
          "api_ready": true,
          "daemon_ready": true,
          "invocation_ready": true,
          "directory_ready": true,
          "trust_ready": true,
          "runtime_ready": true,
          "diagnostics": []
        }
        """)
        let client = HealthClient(transport: transport)
        try await client.close()
        XCTAssertTrue(transport.closed)
        await expectSDKError(.invalidHandle) {
            _ = try await client.runtimeHealth()
        }
    }

    func testDirectoryIdentityBuildsCarriersAndProjectsReadModels() async throws {
        let directory = DirectoryClient(transport: FixtureDirectoryTransport())
        let base = try DirectoryQueryBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            limit: 2,
            cursor: "0",
            metadata: ["request_id": .string("directory-list-devices-1")]
        )

        let deviceCarrier = try await directory.buildListDevicesInvocation(base)
        XCTAssertEqual(
            try optionalDirectoryJSONString(deviceCarrier["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.node.list@1.0.0"
        )
        let devicePage = try await directory.listDevices(base)
        XCTAssertEqual(devicePage.kind, "device_page")

        let agentBase = try DirectoryQueryBase(
            callerURA: base.callerURA,
            calleeURA: base.calleeURA,
            subjectURA: base.subjectURA,
            descriptorVersion: base.descriptorVersion,
            nonceBase64: base.nonceBase64,
            causalContext: base.causalContext,
            limit: base.limit,
            cursor: base.cursor,
            metadata: ["request_id": .string("directory-list-agents-1")]
        )
        let agentPage = try await directory.listAgents(agentBase)
        let agentCarrier = try await directory.buildListAgentsInvocation(agentBase)
        XCTAssertEqual(agentPage.itemKind, "agent")
        XCTAssertTrue(
            try optionalDirectoryJSONString(agentCarrier["descriptor_ref"], "descriptor_ref")?
                .contains("agent.list") == true
        )

        let abilityBase = try DirectoryQueryBase(
            callerURA: base.callerURA,
            calleeURA: base.calleeURA,
            subjectURA: base.subjectURA,
            descriptorVersion: base.descriptorVersion,
            nonceBase64: base.nonceBase64,
            causalContext: base.causalContext,
            limit: base.limit,
            cursor: base.cursor,
            metadata: ["request_id": .string("directory-list-abilities-1")]
        )
        let abilityQuery = try AbilityQuery(
            base: abilityBase,
            scope: "local",
            ownerURA: "easynet:///r/example/device/dev-a",
            abilityURA: "easynet:///r/example/ability/device.dev-a.fs.read"
        )
        let abilityPage = try await directory.listAbilities(abilityQuery)
        let abilityCarrier = try await directory.buildListAbilitiesInvocation(abilityQuery)
        XCTAssertEqual(abilityPage.kind, "ability_page")
        XCTAssertTrue(
            try optionalDirectoryJSONString(abilityCarrier["descriptor_ref"], "descriptor_ref")?
                .contains("meta.list_abilities") == true
        )

        let resolveQuery = try ResolveQuery(
            base: base,
            queryName: "easynet:///r/example/device/dev-a",
            abilityName: "agent.list",
            queryType: "route"
        )
        let resolveCarrier = try await directory.buildResolveInvocation(resolveQuery)
        let resolved = try await directory.resolve(resolveQuery)
        XCTAssertTrue(
            try optionalDirectoryJSONString(resolveCarrier["descriptor_ref"], "descriptor_ref")?
                .contains("namespace.resolve") == true
        )
        XCTAssertEqual(resolved.abilityURA, "easynet:///r/example/ability/device.dev-a.agent.list")
    }

    func testDirectorySubscriptionUsesStreamLifecycle() async throws {
        let directory = DirectoryClient(transport: FixtureDirectoryTransport())
        let base = try DirectoryQueryBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            metadata: ["request_id": .string("directory-subscribe")]
        )
        let request = try DirectorySubscriptionRequest(base: base, itemKind: "ability")

        let carrier = try await directory.buildDirectorySubscriptionInvocation(request)
        XCTAssertTrue(
            try optionalDirectoryJSONString(carrier["descriptor_ref"], "descriptor_ref")?
                .contains("directory.subscribe") == true
        )

        let subscription = try await directory.projectSubscription(fixture("directory-subscription.v4.json"))
        XCTAssertEqual(subscription.state, "Live")
        XCTAssertEqual(subscription.resumeToken, "directory:3")
        XCTAssertEqual(subscription.events.count, 3)
        XCTAssertEqual(subscription.events.last?.phase, "live")

        let stream = try await directory.subscribeDirectory(request)
        let first = try await stream.next()
        let terminal = try await stream.next()
        XCTAssertTrue(first.payloadJSON.contains("\"phase\":\"live\""))
        XCTAssertTrue(terminal.terminal)

        expectSyncSDKError(.invalidArgument) {
            _ = try DirectorySubscriptionRequest(base: base, stream: "device")
        }
    }

    func testDirectoryIdentityRejectsInvalidState() async throws {
        expectSyncSDKError(.invalidArgument) {
            _ = try DirectoryQueryBase(
                callerURA: "easynet:///r/example/agent/alice.sdk",
                calleeURA: "easynet:///r/example/device/dev-a",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")],
                limit: maxDirectoryPageSize + 1
            )
        }

        let directory = DirectoryClient(transport: EmptyDirectoryTransport())
        let base = try DirectoryQueryBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            limit: 2
        )
        await expectSDKError(.notImplemented) {
            _ = try await directory.listDevices(base)
        }
        try await directory.close()
        await expectSDKError(.invalidHandle) {
            _ = try await directory.listAgents(base)
        }
    }

    func testIdentityDescriptorHelpersDelegateToTransport() async throws {
        let identity = IdentityClient(transport: FixtureIdentityTransport())
        let projection = try await identity.projectDescriptorRef(
            try DescriptorRefRequest(
                descriptorRef: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
            )
        )
        XCTAssertTrue(projection.valid)
        let abilityURA = try await identity.abilityURAFromDescriptorRef(
            "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
        )
        let descriptorRef = try await identity.ownerAbilityDescriptorRef(
            ownerURA: "easynet:///r/example/device/dev-a",
            abilityName: "observe.health",
            descriptorVersion: "1.0.0"
        )
        XCTAssertEqual(abilityURA, "easynet:///r/example/ability/device.dev-a.observe.health")
        XCTAssertEqual(descriptorRef, "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
        await expectSDKError(.invalidArgument) {
            _ = try await identity.canonicalAbilityDescriptorRef("")
        }
        await expectSDKError(.invalidArgument) {
            _ = try await identity.projectDescriptorRef(try DescriptorRefRequest(descriptorRef: "not-a-descriptor"))
        }
        try await identity.close()
        await expectSDKError(.invalidHandle) {
            _ = try await identity.projectDescriptorRef(
                try DescriptorRefRequest(
                    descriptorRef: "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"
                )
            )
        }
    }

    func testReceiptBuildsFetchCarrierAndProjectsSummary() async throws {
        let receipt = ReceiptClient(transport: FixtureReceiptTransport())
        let carrier = try receipt.buildFetchInvocation(receiptFetchRequest())
        let expected = try decodeObject(fixture("receipt-fetch-invocation.v4.json"), label: "receipt fetch invocation")
        XCTAssertEqual(carrier, expected)

        let fetched = try await receipt.fetch(receiptFetchRequest())
        XCTAssertEqual(fetched.state, "completed")
        XCTAssertFalse(fetched.verified)

        let projected = try receipt.project(fixture("receipt.summary.v4.json"))
        XCTAssertEqual(projected.invocationID, "inv-example-1")
        let verification = try receipt.verifySummary(projected)
        XCTAssertFalse(verification.verified)
    }

    func testReceiptRejectsInvalidSelectorAndSummaryVerification() async throws {
        expectSyncSDKError(.invalidArgument) {
            _ = try ReceiptFetchRequest(
                callerURA: "easynet:///r/example/agent/alice.sdk",
                calleeURA: "easynet:///r/example/device/dev-a",
                descriptorRef: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")]
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try ReceiptFetchRequest(
                callerURA: "easynet:///r/example/agent/alice.sdk",
                calleeURA: "easynet:///r/example/device/dev-a",
                descriptorRef: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")],
                invocationURA: "easynet:///r/example/invocation/inv-example-1",
                requestID: "inv-example-1"
            )
        }

        let receipt = ReceiptClient(transport: FixtureReceiptTransport())
        let summary = try receipt.project(fixture("receipt.summary.v4.json"))
        let verification = try receipt.verifySummary(summary)
        expectSyncSDKError(.invalidArgument) {
            _ = try verification.requireCryptographic()
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try ReceiptRef.fromSummary(summary)
        }
    }

    func testReceiptOpaqueRefRequiresExplicitAnchorFacts() async throws {
        let ref = try ReceiptRef.fromJSON(fixture("receipt-ref.v4.json"))
        XCTAssertEqual(ref.receiptURA, "easynet:///r/example/receipt/receipt-1")
        XCTAssertEqual(ref.receiptHashHex.count, 64)
        expectSyncSDKError(.invalidArgument) {
            _ = try ReceiptRef(
                receiptURA: "easynet:///r/example/receipt/receipt-1",
                receiptHashHex: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                invocationID: "inv-example-1",
                index: 0
            )
        }
        let receipt = ReceiptClient(transport: EmptyReceiptTransport())
        await expectSDKError(.notImplemented) {
            _ = try await receipt.causalRef(ref)
        }
    }

    func testEventsProfileDelegatesCarriersProjectionsHistoryAndStreams() async throws {
        let events = EventClient(transport: FixtureEventTransport())
        let base = try eventsCarrierBase(metadata: ["request_id": .string("events-directory-subscribe-1")])
        let directoryRequest = try EventsSubscriptionRequest(
            base: base,
            realm: "example",
            agentURA: "easynet:///r/example/agent/alice.main",
            resumeCursor: EventCursor(stream: "directory", sequence: 7),
            heartbeatIntervalMS: 30000
        )
        let deviceRequest = try EventsSubscriptionRequest(
            base: eventsCarrierBase(metadata: ["request_id": .string("events-device-subscribe-1")]),
            stream: "device",
            filter: EventFilter(deviceURA: "easynet:///r/example/device/dev-a"),
            deviceURA: "easynet:///r/example/device/dev-a",
            resumeCursor: EventCursor(stream: "device", sequence: 2),
            heartbeatIntervalMS: 30000
        )
        let sessionRequest = try EventsSubscriptionRequest(
            base: eventsCarrierBase(metadata: ["request_id": .string("events-session-subscribe-1")]),
            stream: "session",
            sessionID: "run-1",
            resumeCursor: EventCursor(stream: "session", sequence: 4)
        )
        let invocationRequest = try EventsSubscriptionRequest(
            base: eventsCarrierBase(metadata: ["request_id": .string("events-invocation-subscribe-1")]),
            stream: "invocation",
            filter: EventFilter(invocationID: "inv-1"),
            invocationID: "inv-1",
            resumeCursor: EventCursor(stream: "invocation", sequence: 9)
        )

        let directoryCarrier = try await events.buildDirectorySubscriptionInvocation(directoryRequest)
        let deviceCarrier = try await events.buildDeviceSubscriptionInvocation(deviceRequest)
        let sessionCarrier = try await events.buildSessionSubscriptionInvocation(sessionRequest)
        let invocationCarrier = try await events.buildInvocationSubscriptionInvocation(invocationRequest)
        XCTAssertTrue(try optionalDirectoryJSONString(directoryCarrier["descriptor_ref"], "descriptor_ref")?
            .contains("federation.subscribe_directory_v2") == true)
        XCTAssertTrue(try optionalDirectoryJSONString(deviceCarrier["descriptor_ref"], "descriptor_ref")?
            .contains("events.device.subscribe") == true)
        XCTAssertTrue(try optionalDirectoryJSONString(sessionCarrier["descriptor_ref"], "descriptor_ref")?
            .contains("session.attach") == true)
        XCTAssertTrue(try optionalDirectoryJSONString(invocationCarrier["descriptor_ref"], "descriptor_ref")?
            .contains("events.invocation.subscribe") == true)

        let page = try await events.listDeviceEvents(EventsDeviceEventListRequest(
            base: eventsCarrierBase(metadata: ["request_id": .string("events-device-history-1")]),
            filter: EventFilter(deviceURA: "easynet:///r/example/device/dev-a"),
            deviceURA: "easynet:///r/example/device/dev-a"
        ))
        XCTAssertEqual(page.limit, 50)
        XCTAssertEqual(page.items.first?.stream, "device")

        let directoryFrame = try await events.projectDirectoryEvent(EventProjectionInput(
            cursor: EventCursor(stream: "directory", sequence: 8),
            event: ["type": .string("agent_advertised")]
        ))
        XCTAssertEqual(directoryFrame.cursor.resumeToken(), "directory:8")
        let live = try await events.projectLiveEvent(EventProjectionInput(
            cursor: EventCursor(stream: "device", sequence: 8),
            event: ["state": .string("online")]
        ))
        XCTAssertEqual(live.stream, "device")
        let drop = try await events.projectDropReport(EventDropReportInput(
            cursor: EventCursor(stream: "directory", sequence: 10),
            occurredUnixMS: 1_783_100_000_123,
            droppedCount: 4,
            reconnectAfterMS: 1000,
            reason: "consumer_lagged"
        ))
        XCTAssertEqual(drop.droppedCount, 4)
        let terminal = try await events.projectTerminal(EventTerminalInput(
            cursor: EventCursor(stream: "directory", sequence: 11),
            occurredUnixMS: 1_783_100_000_123,
            reason: "client_closed"
        ))
        XCTAssertTrue(terminal.terminal)

        let stream = try await events.subscribeDirectory(directoryRequest)
        let firstFrame = try await stream.receive()
        let terminalFrame = try await stream.receive()
        XCTAssertEqual(firstFrame.kind, "directory.agent_advertised")
        XCTAssertTrue(terminalFrame.terminal)
        XCTAssertEqual(stream.state, "Terminal")

        await expectSDKError(.invalidArgument) {
            _ = try await events.buildSessionSubscriptionInvocation(try EventsSubscriptionRequest(
                base: base,
                stream: "session",
                sessionURA: "easynet:///r/example/resource/session.run-1"
            ))
        }
    }

    private func expectSyncSDKError(_ code: SDKErrorCode, _ action: () throws -> Void) {
        do {
            try action()
        } catch let error as SDKError {
            XCTAssertEqual(error.code, code)
            return
        } catch {
            XCTFail("expected SDKError, got \(error)")
            return
        }
        XCTFail("expected SDKError \(code)")
    }

    private func expectSDKError(_ code: SDKErrorCode, _ action: () async throws -> Void) async {
        do {
            try await action()
        } catch let error as SDKError {
            XCTAssertEqual(error.code, code)
            return
        } catch {
            XCTFail("expected SDKError, got \(error)")
            return
        }
        XCTFail("expected SDKError \(code)")
    }

    private func receiptFetchRequest() throws -> ReceiptFetchRequest {
        try ReceiptFetchRequest(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            descriptorRef: "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            requestID: "inv-example-1",
            metadata: ["request_id": .string("receipt-fetch-1")]
        )
    }

    private func eventsCarrierBase(metadata: [String: JSONValue]) throws -> EventsCarrierBase {
        try EventsCarrierBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            metadata: metadata
        )
    }

}

final class MemoryDiscoveryTransport: DiscoveryTransport, @unchecked Sendable {
    var closed = false

    func featureDiscovery() async throws -> FeatureSet {
        if closed {
            throw SDKError.closed("discovery")
        }
        return try FeatureSet(abiVersion: 4, sdkVersion: "0.0.0-seam", profiles: ["runtime_core": "seam"])
    }

    func close() async throws {
        closed = true
    }
}

class MemoryRuntimeTransport: RuntimeTransport, @unchecked Sendable {
    func invoke(_ draft: InvocationDraft) async throws -> InvocationResult {
        try InvocationResult(ok: true, terminalState: .completed, outputJSON: "{\"ok\":true}")
    }

    func prepare(_ draftJSON: Data, optionsJSON: Data) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime prepare transport is not implemented"
        )
    }

    func submitSigned(_ signedJSON: Data) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime submit-signed transport is not implemented"
        )
    }

    func openStream(_ draft: InvocationDraft) async throws -> StreamSource {
        QueueStreamSource(count: 1)
    }

    func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) async throws -> BidiSource {
        QueueBidiSource(count: 1)
    }
}

final class QueueStreamSource: StreamSource, @unchecked Sendable {
    private var events: [StreamEvent] = []

    init(count: Int) {
        for index in 0..<count {
            events.append(try! .data(index, payloadJSON: "{\"n\":\(index)}"))
        }
    }

    func next() async throws -> StreamEvent {
        if events.isEmpty {
            return try .terminal(9999, state: "Completed")
        }
        return events.removeFirst()
    }
}

final class DirectoryStreamSource: StreamSource, @unchecked Sendable {
    private var events: [StreamEvent] = [
        try! .data(
            3,
            payloadJSON: """
            {"profile":"directory_identity","stream":"directory","kind":"upsert","event_id":"evt-3","phase":"live","cursor":{"stream":"directory","sequence":3,"token":"directory:3"},"resume_token":"directory:3","terminal":false,"metadata":{"source":"directory.subscribe"}}
            """
        ),
        try! .terminal(4, state: "Closed"),
    ]

    func next() async throws -> StreamEvent {
        events.removeFirst()
    }
}

final class EventsDirectoryStreamSource: StreamSource, @unchecked Sendable {
    private var events: [StreamEvent] = [
        try! .data(8, payloadJSON: String(decoding: fixture("event.directory.v4.json"), as: UTF8.self)),
        try! .data(11, payloadJSON: String(decoding: fixture("event.directory-terminal.v4.json"), as: UTF8.self)),
    ]

    func next() async throws -> StreamEvent {
        events.removeFirst()
    }
}

final class QueueBidiSource: BidiSource, @unchecked Sendable {
    private var frames: [BidiFrame] = []

    init(count: Int) {
        for index in 0..<count {
            frames.append(try! .data(index, payloadJSON: "{\"n\":\(index)}"))
        }
    }

    func send(_ frame: BidiFrame) async throws {}

    func next() async throws -> BidiFrame {
        if frames.isEmpty {
            return try .terminal(9999, kind: "completed")
        }
        return frames.removeFirst()
    }
}

final class MemoryHealthTransport: HealthTransport, DiagnosticsTransport, @unchecked Sendable {
    private let healthJSON: String
    private let diagnosticsJSON: String
    var closed = false

    init(healthJSON: String, diagnosticsJSON: String) {
        self.healthJSON = healthJSON
        self.diagnosticsJSON = diagnosticsJSON
    }

    func runtimeHealth() async throws -> Data {
        if closed {
            throw SDKError.closed("health_transport")
        }
        return Data(healthJSON.utf8)
    }

    func runtimeDiagnostics() async throws -> Data {
        Data(diagnosticsJSON.utf8)
    }

    func close() async throws {
        closed = true
    }
}

final class HealthOnlyTransport: HealthTransport, @unchecked Sendable {
    private let healthJSON: String
    var closed = false

    init(healthJSON: String) {
        self.healthJSON = healthJSON
    }

    func runtimeHealth() async throws -> Data {
        Data(healthJSON.utf8)
    }

    func close() async throws {
        closed = true
    }
}

final class FailingHealthTransport: HealthTransport, @unchecked Sendable {
    func runtimeHealth() async throws -> Data {
        throw FixtureFailure.down
    }
}

enum FixtureFailure: Error {
    case down
}

final class FixtureDirectoryTransport: DirectoryTransport, @unchecked Sendable {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data {
        let request = String(decoding: requestJSON, as: UTF8.self)
        XCTAssertTrue(request.contains("\"stream\":\"directory\""))
        XCTAssertFalse(request.contains("\"limit\""))
        return fixture("directory-subscription-invocation.v4.json")
    }

    func buildListDevicesInvocation(_ requestJSON: Data) async throws -> Data {
        XCTAssertTrue(String(decoding: requestJSON, as: UTF8.self).contains("\"limit\":2"))
        return fixture("directory-list-devices-invocation.v4.json")
    }

    func buildListAgentsInvocation(_ requestJSON: Data) async throws -> Data {
        fixture("directory-list-agents-invocation.v4.json")
    }

    func buildListAbilitiesInvocation(_ requestJSON: Data) async throws -> Data {
        XCTAssertTrue(String(decoding: requestJSON, as: UTF8.self).contains("\"scope\":\"local\""))
        return fixture("directory-list-abilities-invocation.v4.json")
    }

    func buildResolveInvocation(_ requestJSON: Data) async throws -> Data {
        fixture("directory-resolve-invocation.v4.json")
    }

    func listDevices(_ requestJSON: Data) async throws -> Data {
        fixture("directory-device-page.v4.json")
    }

    func listAgents(_ requestJSON: Data) async throws -> Data {
        fixture("directory-agent-page.v4.json")
    }

    func listAbilities(_ requestJSON: Data) async throws -> Data {
        fixture("directory-ability-page.v4.json")
    }

    func resolve(_ requestJSON: Data) async throws -> Data {
        fixture("directory-resolved-ref.v4.json")
    }

    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource {
        XCTAssertTrue(String(decoding: requestJSON, as: UTF8.self).contains("\"item_kind\":\"ability\""))
        return DirectoryStreamSource()
    }

    func projectSubscription(_ subscriptionJSON: Data) async throws -> Data {
        subscriptionJSON
    }
}

final class EmptyDirectoryTransport: DirectoryTransport, @unchecked Sendable {}

final class FixtureIdentityTransport: IdentityTransport, @unchecked Sendable {
    func projectDescriptorRef(_ requestJSON: Data) async throws -> Data {
        if String(decoding: requestJSON, as: UTF8.self).contains("not-a-descriptor") {
            throw invalidDirectory("descriptor_ref is invalid")
        }
        return fixture("identity.descriptor-ref.v4.json")
    }

    func buildDescriptorRef(_ requestJSON: Data) async throws -> Data {
        fixture("identity.descriptor-ref.v4.json")
    }

    func ownerAbilityURA(_ requestJSON: Data) async throws -> Data {
        Data("""
        {
          "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health"
        }
        """.utf8)
    }
}

final class FixtureReceiptTransport: ReceiptTransport, @unchecked Sendable {
    func fetch(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "receipt fetch request")
        let expected = try decodeObject(fixture("receipt-fetch-request.v4.json"), label: "receipt fetch fixture")
        XCTAssertEqual(request, expected)
        return fixture("receipt.summary.v4.json")
    }
}

final class EmptyReceiptTransport: ReceiptTransport, @unchecked Sendable {}

final class FixtureEventTransport: EventTransport, @unchecked Sendable {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "events directory request")
        XCTAssertEqual(try optionalDirectoryJSONString(request["stream"], "stream"), "directory")
        return fixture("events-directory-subscription-invocation.v4.json")
    }

    func buildDeviceSubscriptionInvocation(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "events device request")
        XCTAssertEqual(try optionalDirectoryJSONString(request["stream"], "stream"), "device")
        return fixture("events-device-subscription-invocation.v4.json")
    }

    func buildSessionSubscriptionInvocation(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "events session request")
        XCTAssertEqual(try optionalDirectoryJSONString(request["session_id"], "session_id"), "run-1")
        return fixture("events-session-subscription-invocation.v4.json")
    }

    func buildInvocationSubscriptionInvocation(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "events invocation request")
        XCTAssertEqual(try optionalDirectoryJSONString(request["invocation_id"], "invocation_id"), "inv-1")
        return fixture("events-invocation-subscription-invocation.v4.json")
    }

    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource {
        EventsDirectoryStreamSource()
    }

    func listDeviceEvents(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "events device history request")
        if case let .number(limit) = request["limit"] {
            XCTAssertEqual(limit, 50)
        } else {
            XCTFail("missing event history limit")
        }
        return fixture("event.device-page.v4.json")
    }

    func projectDirectoryEvent(_ eventJSON: Data) async throws -> Data {
        fixture("event.directory.v4.json")
    }

    func projectLiveEvent(_ eventJSON: Data) async throws -> Data {
        let request = try decodeObject(eventJSON, label: "events live projection request")
        let cursor = try requiredDirectoryObject(request, "cursor")
        return try optionalDirectoryJSONString(cursor["stream"], "stream") == "invocation"
            ? fixture("event.invocation-live.v4.json")
            : fixture("event.device-live.v4.json")
    }

    func projectDropReport(_ dropJSON: Data) async throws -> Data {
        fixture("event.directory-drop-report.v4.json")
    }

    func projectTerminal(_ terminalJSON: Data) async throws -> Data {
        fixture("event.directory-terminal.v4.json")
    }
}

func fixture(_ name: String) -> Data {
    let cwd = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
    let candidates = [
        cwd.appendingPathComponent("sdk/conformance/fixtures").appendingPathComponent(name),
        cwd.appendingPathComponent("../..").appendingPathComponent("sdk/conformance/fixtures").appendingPathComponent(name),
    ]
    for url in candidates where FileManager.default.fileExists(atPath: url.path) {
        return try! Data(contentsOf: url)
    }
    fatalError("fixture not found: \(name)")
}

func decodedObject(_ data: Data) throws -> [String: Any] {
    guard let object = try JSONSerialization.jsonObject(with: data, options: []) as? [String: Any] else {
        throw SDKError.validation("test", "fixture must be an object")
    }
    return object
}

func replacingFixture(_ name: String, _ old: String, _ new: String) -> Data {
    let text = String(decoding: fixture(name), as: UTF8.self).replacingOccurrences(of: old, with: new)
    return Data(text.utf8)
}

func preparedFixtureWithMismatchedSigningDescriptor() throws -> Data {
    var object = try decodedObject(fixture("prepared.signing-material.v4.json"))
    var material = object["signing_material"] as? [String: Any] ?? [:]
    material["descriptor_ref"] = "easynet:///r/example/ability/other@1.0.0"
    object["signing_material"] = material
    return try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
}
