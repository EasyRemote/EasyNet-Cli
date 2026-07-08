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

    func testAuthorityMetadataProjectsAndRejectsAmbiguousDrafts() async throws {
        let fixture = try decodedObject(fixture("authority-metadata.v4.json"))
        let delegationValue = try XCTUnwrap(fixture["delegation_metadata_value"] as? String)
        let sessionValue = try XCTUnwrap(fixture["session_authority_metadata_value"] as? String)

        let delegation = try DelegationProof.fromMetadata(delegationValue)
        let session = try SessionAuthority.fromMetadata(sessionValue)
        XCTAssertEqual(delegation.issuerURA, "easynet:///r/example/user/alice")
        XCTAssertEqual(delegation.signatureBase64, "ZGVsZWdhdGlvbi1zaWduYXR1cmU=")
        XCTAssertEqual(session.sessionID, "session-1")
        XCTAssertEqual(session.sessionOwnerUserID, "alice")
        XCTAssertEqual(session.creatorPrincipalID, "easynet:///r/example/agent/backend")
        XCTAssertEqual(session.calleeURA, "easynet:///r/example/device/dev-a")
        XCTAssertEqual(session.audience, "easynet:///r/example/device/dev-a")

        let builder = InvocationBuilder()
            .withCallerURA("easynet:///r/example/agent/backend")
            .withCalleeURA("easynet:///r/example/device/dev-a")
            .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
            .withSubjectURA("easynet:///r/example/user/alice")
            .withNonce("AQIDBAUGBwgJCgsMDQ4PEA==")
            .withCausalContext("{\"form\":\"none\"}")
            .withArgsJSON("{}")
            .withMetadata(["trace": .string("authority-shared")])
        try builder.withAuthorityMetadata(delegation.metadata())
        let draft = try builder.build()
        XCTAssertEqual(draft.inspectTuple().metadata["trace"], .string("authority-shared"))
        XCTAssertEqual(draft.inspectTuple().metadata[delegationMetadataKey], .string(delegationValue))

        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationBuilder()
                .withCallerURA("easynet:///r/example/agent/backend")
                .withCalleeURA("easynet:///r/example/device/dev-a")
                .withDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
                .withSubjectURA("easynet:///r/example/user/alice")
                .withNonce("AQIDBAUGBwgJCgsMDQ4PEA==")
                .withCausalContext("{\"form\":\"none\"}")
                .withArgsJSON("{}")
                .withMetadata([
                    delegationMetadataKey: .string(delegationValue),
                    sessionAuthorityMetadataKey: .string(sessionValue),
                ])
                .build()
        }

        let authority = AuthorityClient(transport: FixtureAuthorityTransport(delegationValue: delegationValue, sessionValue: sessionValue))
        let mintedDelegation = try await authority.mintDelegationProof(try DelegationRequest(
            issuerURA: delegation.issuerURA,
            subjectURA: delegation.subjectURA,
            callerURA: delegation.callerURA,
            audience: delegation.audience,
            scopes: delegation.scopes,
            issuedAtMS: delegation.issuedAtMS,
            expiresAtMS: delegation.expiresAtMS,
            metadata: ["trace": .string("delegation")]
        ))
        let mintedSession = try await authority.mintSessionAuthority(try SessionAuthorityRequest(
            issuerURA: session.issuerURA,
            sessionID: session.sessionID,
            sessionOwnerUserID: session.sessionOwnerUserID,
            creatorPrincipalID: session.creatorPrincipalID,
            calleeURA: session.calleeURA,
            subjectURA: session.subjectURA,
            audience: session.audience,
            scopes: session.scopes,
            allowedActions: session.allowedActions,
            allowedFollowupAbilities: session.allowedFollowupAbilities,
            issuedAtMS: session.issuedAtMS,
            expiresAtMS: session.expiresAtMS,
            metadata: ["trace": .string("session")]
        ))
        XCTAssertEqual(mintedDelegation.metadataValue, delegationValue)
        XCTAssertEqual(mintedSession.metadataValue, sessionValue)

        expectSyncSDKError(.invalidArgument) {
            _ = try DelegationRequest(
                issuerURA: delegation.issuerURA,
                subjectURA: delegation.subjectURA,
                callerURA: delegation.callerURA,
                audience: delegation.audience,
                scopes: [],
                issuedAtMS: delegation.issuedAtMS,
                expiresAtMS: delegation.expiresAtMS
            )
        }
        try await authority.close()
        try await authority.close()
        await expectSDKError(.invalidHandle) {
            _ = try await authority.mintSessionAuthority(try SessionAuthorityRequest(
                issuerURA: session.issuerURA,
                sessionID: session.sessionID,
                sessionOwnerUserID: session.sessionOwnerUserID,
                creatorPrincipalID: session.creatorPrincipalID,
                calleeURA: session.calleeURA,
                subjectURA: session.subjectURA,
                audience: session.audience,
                scopes: session.scopes,
                allowedActions: session.allowedActions,
                allowedFollowupAbilities: session.allowedFollowupAbilities,
                issuedAtMS: session.issuedAtMS,
                expiresAtMS: session.expiresAtMS
            ))
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
        let descriptorBoundSubject = try await identity.descriptorBoundResourceSubjectURA(
            ownerURA: "easynet:///r/example/user/alice",
            path: "invoke/meta.list_resources"
        )
        XCTAssertEqual(
            descriptorBoundSubject,
            "easynet:///r/example/resource/user.alice/invoke/meta.list_resources"
        )
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

        let projected = try await receipt.project(fixture("receipt.summary.v4.json"))
        XCTAssertEqual(projected.invocationID, "inv-example-1")
        let providerVerification = try await receipt.verify(fixture("receipt-ref.v4.json"))
        XCTAssertTrue(providerVerification.verified)
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
        let summary = try await receipt.project(fixture("receipt.summary.v4.json"))
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
        XCTAssertEqual(ref.receiptURA, "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt")
        XCTAssertEqual(ref.receiptHashHex.count, 64)
        expectSyncSDKError(.invalidArgument) {
            _ = try ReceiptRef(
                receiptURA: "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
                receiptHashHex: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                invocationID: "inv-example-1",
                index: 0
            )
        }
        let receipt = ReceiptClient(transport: EmptyReceiptTransport())
        await expectSDKError(.notImplemented) {
            _ = try await receipt.causalRef(ref)
        }
        let fixtureReceipt = ReceiptClient(transport: FixtureReceiptTransport())
        let chain = try ReceiptChain(receipts: [ref])
        let verification = try await fixtureReceipt.verifyChain(chain)
        XCTAssertTrue(verification.verified)
        XCTAssertEqual(verification.rootReceiptURA, "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt")
    }

    func testPublicationProfileDelegatesResourceValidationAndCarriers() async throws {
        let publication = PublicationClient(transport: FixturePublicationTransport())
        let resource = try await publication.buildLocalResourceRef(
            LocalResourceRefRequest(path: "/tmp/easynet-weather-package", capability: "read")
        )
        XCTAssertEqual(resource.namespace, "fs")
        XCTAssertEqual(resource.capability, "read")

        let validation = try await publication.validatePackage(
            "",
            options: ValidatePackageOptions(
                manifest: try AbilityPackageManifest(
                    name: "weather",
                    namespace: "er",
                    description: "Weather stream",
                    inputSchema: ["type": .string("object"), "properties": .object([:])],
                    exec: [
                        "kind": .string("host_stream"),
                        "host_socket": .string("/tmp/easynet-weather.sock"),
                        "function": .string("weather.stream"),
                    ]
                )
            )
        )
        XCTAssertTrue(validation.valid)
        XCTAssertEqual(validation.manifest.wireKey, "er.weather")
        XCTAssertEqual(
            try optionalDirectoryJSONString(validation.metadata["frame_contract_owner"], "frame_contract_owner"),
            "daemon_sdk"
        )

        let deploy = try await publication.buildDeployInvocation(try publicationDeployRequest(resource))
        XCTAssertEqual(
            try optionalDirectoryJSONString(deploy["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(try requiredDirectoryObject(deploy, "metadata")["system_ability"], "system_ability"),
            "ability.deploy"
        )

        let unpublish = try await publication.buildUnpublishInvocation(UnpublishAbilityRequest(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            abilityURA: "easynet:///r/example/ability/device.dev-a.er.weather"
        ))
        XCTAssertEqual(
            try optionalDirectoryJSONString(unpublish["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0"
        )

        expectSyncSDKError(.invalidArgument) {
            _ = try LocalResourceRefRequest(path: "tmp/easynet-weather-package", capability: "read")
        }
        await expectSDKError(.invalidArgument) {
            _ = try await publication.buildDeployInvocation(AbilityDeployRequest(
                callerURA: "easynet:///r/example/agent/alice.sdk",
                calleeURA: "easynet:///r/example/device/dev-a",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")],
                resourceRef: ResourceRef(
                    resourceURA: "easynet:///r/example/resource/device.dev-a/system/tmp/easynet-weather-package",
                    ownerURA: "easynet:///r/example/device/dev-a",
                    namespace: "system",
                    displayPath: "tmp/easynet-weather-package",
                    capability: "read",
                    expiresUnixMS: 4_102_444_800_000,
                    revision: "fs-local-mapping-v1"
                ),
                nodeID: "local",
                metadata: ["request_id": .string("publication-deploy-1")]
            ))
        }
        await expectSDKError(.invalidArgument) {
            _ = try await publication.buildDeployInvocation(AbilityDeployRequest(
                callerURA: "",
                calleeURA: "easynet:///r/example/device/dev-a",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")],
                resourceRef: resource,
                nodeID: "local",
                metadata: ["request_id": .string("publication-deploy-1")]
            ))
        }
        try await publication.close()
        await expectSDKError(.invalidHandle) {
            _ = try await publication.buildLocalResourceRef(
                LocalResourceRefRequest(path: "/tmp/easynet-weather-package", capability: "read")
            )
        }
    }

    func testMissionProfileDelegatesCarriersStatusAndStreams() async throws {
        let mission = MissionClient(transport: FixtureMissionTransport())

        let run = try await mission.buildRunEALInvocation(missionRunRequest())
        XCTAssertEqual(
            try optionalDirectoryJSONString(run["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0"
        )
        let runFile = try await mission.buildRunFileInvocation(missionRunFileRequest())
        XCTAssertEqual(
            try optionalDirectoryJSONString(runFile["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.mission.run@1.0.0"
        )
        let track = try await mission.buildTrackInvocation(missionTrackRequest())
        XCTAssertEqual(
            try optionalDirectoryJSONString(track["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.mission.track@1.0.0"
        )
        let cancel = try await mission.buildCancelInvocation(missionCancelRequest())
        XCTAssertEqual(
            try optionalDirectoryJSONString(cancel["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0"
        )
        let eventsCarrier = try await mission.buildEventsInvocation(missionEventsRequest())
        XCTAssertEqual(
            try optionalDirectoryJSONString(eventsCarrier["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0"
        )

        let status = try await mission.track(missionTrackRequest())
        XCTAssertTrue(status.terminal)
        XCTAssertEqual(status.state, "partial")
        XCTAssertEqual(status.parentReceiptURA, "easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt")
        XCTAssertEqual(status.childInvocations.count, 1)
        XCTAssertEqual(status.childReceipts.count, 1)
        XCTAssertEqual(status.outputRefs.count, 4)

        let page = try await mission.events(missionEventsRequest())
        XCTAssertEqual(page.events.count, 2)
        XCTAssertEqual(page.nextCursorSequence, 7)
        XCTAssertTrue(page.events[1].terminal)

        let stream = try await mission.openEventStream(missionEventsRequest())
        let streamed = try await stream.receive()
        XCTAssertEqual(streamed.eventType, "progress")
        XCTAssertEqual(streamed.sequence, 7)
        try await stream.cancel(reason: "done")
        try await stream.close()

        expectSyncSDKError(.invalidArgument) {
            _ = try MissionTrackRequest(base: missionCarrier(), missionID: "../weather")
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try MissionStatus.fromJSON(
                replacingFixture("mission-status.v4.json", "\"request_id\": \"req-1\"", "\"request_id\": null")
            )
        }

        try await mission.close()
        await expectSDKError(.invalidHandle) {
            _ = try await mission.track(missionTrackRequest())
        }
    }

    func testAdminGatewayProfileDelegatesCarriersAndStatus() async throws {
        let admin = AdminClient(transport: FixtureAdminTransport())

        let list = try await admin.buildAgentListInvocation(adminAgentListRequest())
        XCTAssertEqual(try optionalDirectoryJSONString(list["descriptor_ref"], "descriptor_ref"), "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0")
        let start = try await admin.buildAgentStartInvocation(adminAgentStartRequest())
        XCTAssertEqual(try optionalDirectoryJSONString(start["descriptor_ref"], "descriptor_ref"), "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0")
        let stop = try await admin.buildAgentStopInvocation(adminAgentStopRequest())
        XCTAssertEqual(try optionalDirectoryJSONString(stop["descriptor_ref"], "descriptor_ref"), "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0")
        let refresh = try await admin.buildAgentRefreshInvocation(adminAgentRefreshRequest())
        XCTAssertEqual(try optionalDirectoryJSONString(refresh["descriptor_ref"], "descriptor_ref"), "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0")
        let sessionsCarrier = try await admin.buildSessionListInvocation(adminSessionListRequest())
        XCTAssertEqual(try optionalDirectoryJSONString(sessionsCarrier["descriptor_ref"], "descriptor_ref"), "easynet:///r/example/ability/device.dev-a.session.list@1.0.0")

        let gateway = try await admin.gatewayStatus()
        XCTAssertTrue(gateway.ready)
        XCTAssertTrue(gateway.processLive)
        XCTAssertFalse(gateway.publicListenerReady)
        XCTAssertEqual(gateway.listeners.count, 2)

        let agents = try await admin.listAgents(adminAgentListRequest())
        XCTAssertEqual(agents.items.count, 1)
        let lifecycle = try await admin.agentStart(adminAgentStartRequest())
        XCTAssertEqual(lifecycle.operation, "agent.start")
        XCTAssertEqual(lifecycle.agentURA, "easynet:///r/example/agent/alice.codex")

        let preflight = try await admin.pairingPreflight(pairingPreflightRequest())
        XCTAssertTrue(preflight.pairingRequired)
        XCTAssertFalse(preflight.trustReady)
        let token = try await admin.createPairing(createPairingRequest())
        XCTAssertEqual(token.tokenID, "pair-token-1")
        XCTAssertEqual(token.scopes, ["invoke", "events"])
        let credential = try await admin.validatePairing(validatePairingRequest())
        XCTAssertEqual(credential.credentialID, "cred-dev-a")
        let session = try await admin.createDeviceSession(createDeviceSessionRequest())
        XCTAssertEqual(session.sessionID, "dev-session-1")
        let sessions = try await admin.listDeviceSessions(adminSessionListRequest())
        XCTAssertEqual(sessions.items.count, 1)
        let deleted = try await admin.deleteDeviceSession(deleteDeviceSessionRequest())
        XCTAssertEqual(deleted.operation, "session.delete")
        XCTAssertEqual(deleted.ack, true)

        expectSyncSDKError(.invalidArgument) {
            _ = try AdminAgentStartRequest(
                base: adminCarrier(requestID: "admin-agent-start-1"),
                name: "device.system",
                agentType: "codex",
                model: "gpt-5",
                label: "primary"
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try AdminCarrierBase(
                callerURA: "",
                calleeURA: "easynet:///r/example/device/dev-a",
                subjectURA: "easynet:///r/example/device/dev-a",
                descriptorVersion: "1.0.0",
                nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
                causalContext: ["form": .string("none")]
            )
        }

        try await admin.close()
        await expectSDKError(.invalidHandle) {
            _ = try await admin.listAgents(adminAgentListRequest())
        }
    }

    func testHostBindingProfileDelegatesCodecHashAndLifecycle() async throws {
        let lifecycleProvider = FixtureHostLifecycleProvider()
        let hostBinding = HostBindingClient(
            transport: FixtureHostBindingTransport(),
            lifecycleProvider: lifecycleProvider
        )
        let binding = try await hostBinding.buildHostStreamBinding(HostStreamBindingRequest(
            bindingID: "binding-weather-1",
            descriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
            endpoint: "/tmp/easynet-weather.sock",
            frameSchema: hostStreamFrameSchema,
            cleanup: ["mode": .string("unlink_socket")],
            timeoutMS: 30000,
            readiness: ["state": .string("declared"), "checked": .bool(false), "endpoint_ready": .null],
            metadata: ["owner": .string("easyremote")]
        ))
        XCTAssertEqual(
            try optionalDirectoryJSONString(binding.lifecycle["frame_contract_owner"], "frame_contract_owner"),
            "daemon_sdk"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(binding.metadata["hash_algorithm"], "hash_algorithm"),
            hostStreamHashAlgorithm
        )

        let request = try await hostBinding.decodeRequest(HostStreamEnvelope(
            function: "weather.stream",
            args: .object(["city": .string("Singapore")]),
            callID: "call-weather-1",
            caller: "easynet:///r/example/user/alice"
        ))
        XCTAssertEqual(request.function, "weather.stream")
        XCTAssertEqual(try optionalDirectoryJSONString(request.metadata["wire"], "wire"), "host_stream_request_v1")

        let item = try await hostBinding.encodeItem(seq: 0, value: .object(["token": .string("hello")]))
        XCTAssertEqual(item.frameType, "item")
        XCTAssertEqual(item.seq, 0)
        let error = try await hostBinding.encodeError(SDKError.validation("host", "bad input"))
        XCTAssertEqual(error.frameType, "error")
        XCTAssertEqual(try optionalDirectoryJSONString(error.error["code"], "code"), "INVALID_ARGUMENT")
        let folded = try await hostBinding.foldOutputHash(
            state: HostStreamHashState.initial(),
            seq: 0,
            value: .object(["token": .string("hello")])
        )
        XCTAssertEqual(folded.outputHash, "sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15")
        XCTAssertEqual(
            folded,
            try hostBinding.foldOutputHashLocal(
                state: HostStreamHashState.initial(),
                seq: 0,
                value: .object(["token": .string("hello")])
            )
        )
        let terminal = try await hostBinding.encodeTerminal(
            HostStreamTerminalSummary.fromJSON(fixture("host-stream-terminal.v4.json"))
        )
        XCTAssertEqual(terminal.frameType, "terminal")
        XCTAssertEqual(terminal.outputHash, folded.outputHash)

        expectSyncSDKError(.invalidArgument) {
            _ = try HostStreamBindingRequest(
                bindingID: "binding-weather-1",
                descriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint: "tmp/easynet-weather.sock",
                frameSchema: hostStreamFrameSchema
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try HostStreamBindingRequest(
                bindingID: "binding-weather-1",
                descriptorRef: "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                endpoint: "/tmp/easynet-weather.sock",
                frameSchema: "drift.schema.json"
            )
        }
        await expectSDKError(.invalidArgument) {
            _ = try await hostBinding.foldOutputHash(
                state: HostStreamHashState.fromJSON(fixture("host-stream-hash-state.v4.json")),
                seq: 2,
                value: .object(["token": .string("skip")])
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try HostStreamHashState.fromJSON(fixture("host-stream-hash-state-corrupted-zero.v4.json"))
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try HostStreamHashState.fromJSON(fixture("host-stream-hash-state-corrupted-gap.v4.json"))
        }

        let lifecycle = try hostBinding.openLifecycle(binding)
        let readiness = try await lifecycle.checkReadiness()
        let cleanup = try await lifecycle.cleanup()
        let cleanupAgain = try await lifecycle.cleanup()
        XCTAssertEqual(readiness.state, "ready")
        XCTAssertEqual(readiness.endpointReady, true)
        XCTAssertEqual(cleanup.mode, "unlink_socket")
        XCTAssertEqual(cleanup.metadata["cleaned"], .bool(true))
        XCTAssertEqual(cleanupAgain.metadata["cleaned"], .bool(true))
        XCTAssertEqual(lifecycleProvider.cleanupCalls, 1)
        try await lifecycle.close()
        XCTAssertEqual(lifecycle.state, .closed)

        try await hostBinding.close()
        await expectSDKError(.invalidHandle) {
            _ = try await hostBinding.encodeItem(seq: 0, value: .object(["token": .string("hello")]))
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

    func testSurfaceProfileDelegatesCarriersAndProjections() async throws {
        let surface = SurfaceClient(transport: FixtureSurfaceTransport())
        let list = try SurfaceListPagesRequest(
            base: surfaceCarrierBase(metadata: ["request_id": .string("surface-list-1")]),
            limit: 50
        )
        let create = try SurfaceCreatePageRequest(
            base: surfaceCarrierBase(metadata: ["request_id": .string("surface-create-1")]),
            projectID: "docs",
            folder: "/tmp/easynet-pages-docs",
            visibility: "public"
        )
        let delete = try SurfaceDeletePageRequest(
            base: surfaceCarrierBase(metadata: ["request_id": .string("surface-delete-1")]),
            projectID: "docs"
        )
        let manifest = try SurfaceManifestRequest(
            base: surfaceCarrierBase(metadata: ["request_id": .string("surface-manifest-1")]),
            projectID: "docs"
        )
        let health = try SurfaceHealthRequest(
            base: surfaceCarrierBase(metadata: ["request_id": .string("surface-health-1")]),
            surfaceRef: "easynet:///r/example/resource/alice.docs"
        )

        let listInvocation = try await surface.buildListPagesInvocation(list)
        let createInvocation = try await surface.buildCreatePageInvocation(create)
        let deleteInvocation = try await surface.buildDeletePageInvocation(delete)
        let manifestInvocation = try await surface.buildManifestInvocation(manifest)
        let healthInvocation = try await surface.buildHealthInvocation(health)
        XCTAssertEqual(
            try optionalDirectoryJSONString(listInvocation["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/alice.pages.project_list@1.0.0"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(createInvocation["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/alice.pages.pages.publish@1.0.0"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(deleteInvocation["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(manifestInvocation["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/alice.pages.pages.get@1.0.0"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(healthInvocation["descriptor_ref"], "descriptor_ref"),
            "easynet:///r/example/ability/alice.pages.pages.health@1.0.0"
        )

        let pagePage = try await surface.listPages(list)
        XCTAssertEqual(pagePage.limit, 50)
        XCTAssertEqual(pagePage.items.count, 1)
        XCTAssertEqual(try surface.projectPagePage(fixture("surface-page-page.v4.json")).source, "pages_read_model")
        let record = try await surface.createPage(create)
        XCTAssertEqual(record.pageID, "docs")
        let mutation = try await surface.deletePage(delete)
        let projectedManifest = try await surface.surfaceManifest(manifest)
        let publicRef = try await surface.publicPageRef(record)
        let projectedHealth = try await surface.surfaceHealth(health)
        let status = try await surface.surfaceStatus(health)
        XCTAssertTrue(mutation.removed)
        XCTAssertEqual(projectedManifest.entrypoint["kind"], .string("public_page_ref"))
        XCTAssertEqual(publicRef.routeKind, "hub_web")
        XCTAssertTrue(projectedHealth.ready)
        XCTAssertTrue(status.descriptorRef.contains("pages.health"))
        XCTAssertEqual(try surface.projectManifest(fixture("surface-manifest.v4.json")).pageID, "docs")
        XCTAssertEqual(try surface.projectHealth(fixture("surface-health.v4.json")).pageCount, 1)

        expectSyncSDKError(.invalidArgument) {
            _ = try SurfaceListPagesRequest(base: surfaceCarrierBase(metadata: [:]), limit: 501)
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try SurfaceCreatePageRequest(
                base: surfaceCarrierBase(metadata: [:]),
                projectID: "docs",
                folder: "tmp/easynet-pages-docs"
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try SurfaceHealthRequest(
                base: surfaceCarrierBase(metadata: [:]),
                surfaceRef: "https://example/web/alice/docs/"
            )
        }
        try await surface.close()
        await expectSDKError(.invalidHandle) {
            _ = try await surface.listPages(list)
        }
    }

    func testWrapperProfileProjectsRuntimeRecords() async throws {
        let wrappers = WrapperClient(transport: FixtureWrapperTransport())

        let file = try await wrappers.projectFileRecord(fixture("wrapper-file-record.v4.json"))
        XCTAssertEqual(file.fileRef, "easynet:///r/example/resource/alice.files/report.txt")
        XCTAssertEqual(file.sizeBytes, 42)

        let terminal = try await wrappers.projectTerminalSession(fixture("wrapper-terminal-session.v4.json"))
        let desktop = try await wrappers.projectRemoteDesktopSession(fixture("wrapper-remote-desktop-session.v4.json"))
        let browser = try await wrappers.projectBrowserSession(fixture("wrapper-browser-session.v4.json"))
        let media = try await wrappers.projectMediaSession(fixture("wrapper-media-session.v4.json"))
        XCTAssertEqual(terminal.terminalRef, "terminal-main")
        XCTAssertEqual(desktop.displayRef, "display-main")
        XCTAssertEqual(browser.browserRef, "browser-main")
        XCTAssertEqual(media.mediaKind, "voice")
        XCTAssertEqual(media.streamRef, "stream-voice-1")

        let projectedFile = try await wrappers.projectFileRecord(file)
        let projectedTerminal = try await wrappers.projectTerminalSession(terminal)
        XCTAssertEqual(projectedFile.ownerURA, file.ownerURA)
        XCTAssertEqual(projectedTerminal.state, "active")

        expectSyncSDKError(.invalidArgument) {
            _ = try WrapperFileRecord.fromJSON(Data("""
            {"profile":"wrappers","kind":"file_record","file_ref":"not-a-ura","owner_ura":"easynet:///r/example/agent/alice.sdk","content_type":"text/plain","metadata":{}}
            """.utf8))
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try WrapperTerminalSession.fromJSON(Data("""
            {"profile":"wrappers","kind":"terminal_session","session_id":"term-1","owner_ura":"easynet:///r/example/agent/alice.sdk","terminal_ref":"terminal-main","metadata":{}}
            """.utf8))
        }

        try await wrappers.close()
        await expectSDKError(.invalidHandle) {
            _ = try await wrappers.projectFileRecord(fixture("wrapper-file-record.v4.json"))
        }
    }

    func testCompatibilityProfileDelegatesCarriersAndProjections() async throws {
        let compatibility = CompatibilityClient(transport: FixtureCompatibilityTransport())
        let list = try CompatibilityListModelsRequest.fromJSON(fixture("compatibility-list-models-request.v4.json"))
        let chat = try CompatibilityChatCompletionRequest.fromJSON(fixture("compatibility-chat-completion-request.v4.json"))
        let stream = try CompatibilityStreamChatCompletionRequest.fromJSON(fixture("compatibility-stream-chat-completion-request.v4.json"))
        let upload = try CompatibilityFileUploadRequest.fromJSON(fixture("compatibility-file-upload-request.v4.json"))
        let file = try CompatibilityFileRequest.fromJSON(fixture("compatibility-file-request.v4.json"))
        let delete = try CompatibilityFileDeleteRequest.fromJSON(fixture("compatibility-file-delete-request.v4.json"))

        let listCarrier = try await compatibility.buildListModelsInvocation(list)
        let chatCarrier = try await compatibility.buildChatCompletionInvocation(chat)
        let streamCarrier = try await compatibility.buildStreamChatCompletionInvocation(stream)
        XCTAssertEqual(listCarrier, try decodeObject(fixture("compatibility-list-models-invocation.v4.json"), label: "compatibility list carrier"))
        XCTAssertEqual(chatCarrier, try decodeObject(fixture("compatibility-chat-completion-invocation.v4.json"), label: "compatibility chat carrier"))
        XCTAssertEqual(streamCarrier, try decodeObject(fixture("compatibility-stream-chat-completion-invocation.v4.json"), label: "compatibility stream carrier"))

        let modelPage = try await compatibility.listModels(list)
        let completion = try await compatibility.chatCompletions(chat)
        let streamResult = try await compatibility.streamChatCompletions(stream)
        let uploaded = try await compatibility.uploadFile(upload)
        let fetched = try await compatibility.getFile(file)
        let deleted = try await compatibility.deleteFile(delete)
        XCTAssertEqual(modelPage.data.count, 1)
        XCTAssertEqual(completion.choices.count, 1)
        XCTAssertEqual(streamResult.doneSentinel, "[DONE]")
        XCTAssertEqual(uploaded.bytes, 19)
        XCTAssertEqual(fetched.filename, "prompt.jsonl")
        XCTAssertTrue(deleted.deleted)

        let projectedModels = try await compatibility.projectModelPage(fixture("compatibility-model-page.v4.json"))
        let projectedCompletion = try await compatibility.projectChatCompletion(fixture("compatibility-chat-completion.v4.json"))
        let projectedStream = try await compatibility.projectChatStream(fixture("compatibility-chat-stream.v4.json"))
        let projectedUpload = try await compatibility.projectFileUpload(upload)
        let projectedFile = try await compatibility.projectFile(file)
        let projectedDelete = try await compatibility.projectFileDeleteResult(delete)
        XCTAssertEqual(projectedModels.data.count, 1)
        XCTAssertEqual(projectedCompletion.object, "chat.completion")
        XCTAssertEqual(projectedStream.items.count, 1)
        XCTAssertEqual(projectedUpload.status, "processed")
        XCTAssertEqual(projectedFile.purpose, "batch")
        XCTAssertEqual(projectedDelete.id, "file-easynet-docs-1")

        expectSyncSDKError(.invalidArgument) {
            _ = try CompatibilityChatCompletionRequest.fromJSON(
                replacingFixture(
                    "compatibility-chat-completion-request.v4.json",
                    "easynet:///r/example/ability/alice.codex.chat",
                    "gpt-4o"
                )
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try CompatibilityChatCompletionRequest.fromJSON(
                replacingFixture("compatibility-chat-completion-request.v4.json", "\"temperature\": 0.2", "\"stream\": true")
            )
        }
        try await compatibility.close()
        try await compatibility.close()
        await expectSDKError(.invalidHandle) {
            _ = try await compatibility.listModels(list)
        }
    }

    func testCompanionProfileProjectsStateMachineAndLifecycleActions() async throws {
        let transport = FixtureCompanionTransport()
        let companion = CompanionClient(transport: transport)

        let listed = try await companion.list()
        XCTAssertEqual(listed.companions.count, 1)
        XCTAssertEqual(listed.companions[0].projectedState, .running)

        let status = try await companion.status(packageID: " easynet.desktop.menubar ", packageVersion: " 0.1.0 ")
        XCTAssertEqual(status.packageID, "easynet.desktop.menubar")
        XCTAssertEqual(status.bootPolicy, .ensureRunningAfterDaemonReady)
        XCTAssertEqual(transport.statusInputs.last?.packageVersion, "0.1.0")

        let result = try await companion.disable(packageID: "easynet.desktop.menubar")
        XCTAssertEqual(result.action, "disable")
        XCTAssertEqual(result.statusAfter?.health, .statusFile)

        await expectSDKError(.invalidArgument) {
            _ = try await companion.status(packageID: " ")
        }
        try await companion.close()
        await expectSDKError(.invalidHandle) {
            _ = try await companion.list()
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

    private func publicationDeployRequest(_ ref: ResourceRef) throws -> AbilityDeployRequest {
        try AbilityDeployRequest(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            resourceRef: ref,
            nodeID: "local",
            metadata: ["request_id": .string("publication-deploy-1")]
        )
    }

    private func missionCarrier(requestID: String = "mission-run-1") throws -> MissionCarrierBase {
        try MissionCarrierBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            metadata: ["request_id": .string(requestID)]
        )
    }

    private func missionRunRequest() throws -> MissionRunRequest {
        try MissionRunRequest(
            base: missionCarrier(),
            source: "mission weather\nlet r = local.observe_health()",
            label: "weather"
        )
    }

    private func missionRunFileRequest() throws -> MissionRunFileRequest {
        try MissionRunFileRequest(
            base: missionCarrier(requestID: "mission-run-file-1"),
            path: "/tmp/easynet-sdk-demo.eal",
            label: "file-weather"
        )
    }

    private func missionTrackRequest() throws -> MissionTrackRequest {
        try MissionTrackRequest(
            base: missionCarrier(requestID: "mission-track-1"),
            missionID: "2026-07-04_010203_weather"
        )
    }

    private func missionCancelRequest() throws -> MissionCancelRequest {
        try MissionCancelRequest(
            base: missionCarrier(requestID: "mission-cancel-1"),
            missionID: "2026-07-04_010203_weather"
        )
    }

    private func missionEventsRequest() throws -> MissionEventsRequest {
        try MissionEventsRequest(
            base: missionCarrier(requestID: "mission-events-1"),
            missionID: "2026-07-04_010203_weather",
            cursorSequence: 4,
            limit: 100
        )
    }

    private func adminCarrier(requestID: String) throws -> AdminCarrierBase {
        try AdminCarrierBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/device/dev-a",
            subjectURA: "easynet:///r/example/device/dev-a",
            descriptorVersion: "1.0.0",
            nonceBase64: "AQIDBAUGBwgJCgsMDQ4PEA==",
            causalContext: ["form": .string("none")],
            metadata: ["request_id": .string(requestID)]
        )
    }

    private func adminAgentListRequest() throws -> AdminAgentListRequest {
        try AdminAgentListRequest(base: adminCarrier(requestID: "admin-agent-list-1"))
    }

    private func adminAgentStartRequest() throws -> AdminAgentStartRequest {
        try AdminAgentStartRequest(
            base: adminCarrier(requestID: "admin-agent-start-1"),
            name: "codex",
            agentType: "codex",
            model: "gpt-5",
            label: "primary"
        )
    }

    private func adminAgentStopRequest() throws -> AdminAgentStopRequest {
        try AdminAgentStopRequest(base: adminCarrier(requestID: "admin-agent-stop-1"), name: "codex")
    }

    private func adminAgentRefreshRequest() throws -> AdminAgentRefreshRequest {
        try AdminAgentRefreshRequest(base: adminCarrier(requestID: "admin-agent-refresh-1"), name: "codex")
    }

    private func adminSessionListRequest() throws -> AdminSessionListRequest {
        try AdminSessionListRequest(base: adminCarrier(requestID: "admin-session-list-1"), includeTerminated: false)
    }

    private func pairingPreflightRequest() throws -> PairingPreflightRequest {
        try PairingPreflightRequest(
            base: adminCarrier(requestID: "admin-pairing-preflight-1"),
            hubURA: "easynet:///r/example/hub/main",
            deviceURA: "easynet:///r/example/device/dev-a",
            requestedScopes: ["invoke", "events"]
        )
    }

    private func createPairingRequest() throws -> CreatePairingRequest {
        try CreatePairingRequest(
            base: adminCarrier(requestID: "admin-pairing-create-1"),
            hubURA: "easynet:///r/example/hub/main",
            deviceURA: "easynet:///r/example/device/dev-a",
            expiresUnixMS: 1_893_456_000_000,
            scopes: ["invoke", "events"]
        )
    }

    private func validatePairingRequest() throws -> ValidatePairingRequest {
        try ValidatePairingRequest(
            base: adminCarrier(requestID: "admin-pairing-validate-1"),
            token: "pair-token-value",
            deviceURA: "easynet:///r/example/device/dev-a"
        )
    }

    private func createDeviceSessionRequest() throws -> CreateDeviceSessionRequest {
        try CreateDeviceSessionRequest(
            base: adminCarrier(requestID: "admin-device-session-create-1"),
            deviceURA: "easynet:///r/example/device/dev-a",
            hubURA: "easynet:///r/example/hub/main",
            sessionKind: "remote_desktop",
            expiresUnixMS: 1_893_456_000_000
        )
    }

    private func deleteDeviceSessionRequest() throws -> DeleteDeviceSessionRequest {
        try DeleteDeviceSessionRequest(
            base: adminCarrier(requestID: "admin-device-session-delete-1"),
            sessionID: "dev-session-1",
            reason: "done"
        )
    }

    private func surfaceCarrierBase(metadata: [String: JSONValue]) throws -> SurfaceCarrierBase {
        try SurfaceCarrierBase(
            callerURA: "easynet:///r/example/agent/alice.sdk",
            calleeURA: "easynet:///r/example/agent/alice.pages",
            subjectURA: "easynet:///r/example/agent/alice.pages",
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

final class MissionEventStreamSource: StreamSource, @unchecked Sendable {
    private var events: [StreamEvent] = [
        try! .data(
            7,
            payloadJSON: """
            {"profile":"mission","kind":"mission_event","mission_id":"2026-07-04_010203_weather","sequence":7,"event_type":"progress","occurred_unix_ms":1783126928000,"terminal":false,"payload":{"delta":"stream"},"receipt":{},"metadata":{"profile":"mission","carrier_owner":"daemon_sdk"}}
            """
        ),
    ]

    func next() async throws -> StreamEvent {
        if events.isEmpty {
            return try .terminal(8, state: "Completed")
        }
        return events.removeFirst()
    }

    func cancel(reason: String) async throws -> StreamEvent {
        try .terminal(8, state: "Cancelled")
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

final class FixtureCompanionTransport: CompanionTransport, @unchecked Sendable {
    var statusInputs: [(packageID: String, packageVersion: String)] = []
    var closed = false

    func companionList() async throws -> Data {
        if closed {
            throw SDKError.closed("desktop_companion_transport")
        }
        return Data("""
        {
          "kind": "desktop_companion_list",
          "companions": [
            \(companionStatusJSON())
          ]
        }
        """.utf8)
    }

    func companionStatus(packageID: String, packageVersion: String) async throws -> Data {
        statusInputs.append((packageID, packageVersion))
        return Data(companionStatusJSON(packageID: packageID, packageVersion: packageVersion.isEmpty ? "0.1.0" : packageVersion).utf8)
    }

    func companionEnable(packageID: String, packageVersion: String) async throws -> Data {
        action("enable", packageID: packageID, packageVersion: packageVersion)
    }

    func companionDisable(packageID: String, packageVersion: String) async throws -> Data {
        action("disable", packageID: packageID, packageVersion: packageVersion)
    }

    func companionStart(packageID: String, packageVersion: String) async throws -> Data {
        action("start", packageID: packageID, packageVersion: packageVersion)
    }

    func companionStop(packageID: String, packageVersion: String) async throws -> Data {
        action("stop", packageID: packageID, packageVersion: packageVersion)
    }

    func close() async throws {
        closed = true
    }

    private func action(_ name: String, packageID: String, packageVersion: String) -> Data {
        Data("""
        {
          "profile": "desktop_companion",
          "kind": "desktop_companion_action_result",
          "package_id": "\(packageID)",
          "action": "\(name)",
          "changed": true,
          "status_before": null,
          "status_after": \(companionStatusJSON(packageID: packageID, packageVersion: packageVersion.isEmpty ? "0.1.0" : packageVersion)),
          "error": null,
          "metadata": {}
        }
        """.utf8)
    }

    private func companionStatusJSON(packageID: String = "easynet.desktop.menubar", packageVersion: String = "0.1.0") -> String {
        """
        {
          "profile": "desktop_companion",
          "kind": "desktop_companion_status",
          "package_id": "\(packageID)",
          "package_version": "\(packageVersion)",
          "display_name": "EasyNet Menu Bar",
          "platform": "macos",
          "desired_state": "enabled",
          "supervisor_state": "installed_enabled",
          "observed_state": "running",
          "projected_state": "running",
          "boot_policy": "ensure_running_after_daemon_ready",
          "stop_policy": "keep_running",
          "health": "status_file",
          "pid": 123,
          "version": "0.1.0",
          "last_seen_unix_ms": 1783411200000,
          "launch_method": "launch_agent",
          "error": null,
          "metadata": {}
        }
        """
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

    func buildURA(_ requestJSON: Data) async throws -> Data {
        let request = try decodeDirectoryObject(requestJSON, label: "identity build_ura request JSON")
        XCTAssertEqual(try optionalDirectoryJSONString(request["kind"], "kind"), "resource")
        XCTAssertEqual(
            try optionalDirectoryJSONString(request["owner_ura"], "owner_ura"),
            "easynet:///r/example/user/alice"
        )
        XCTAssertEqual(
            try optionalDirectoryJSONString(request["path"], "path"),
            "invoke/meta.list_resources"
        )
        return Data("""
        {
          "kind": "resource",
          "valid": true,
          "resource_ura": "easynet:///r/example/resource/user.alice/invoke/meta.list_resources",
          "profile": "directory_identity",
          "components": {},
          "metadata": {}
        }
        """.utf8)
    }
}

final class FixtureMissionTransport: MissionTransport, @unchecked Sendable {
    func buildRunEALInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-run-request.v4.json")
        return fixture("mission-run-invocation.v4.json")
    }

    func buildRunFileInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-run-file-request.v4.json")
        return fixture("mission-run-invocation.v4.json")
    }

    func buildTrackInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-track-request.v4.json")
        return fixture("mission-track-invocation.v4.json")
    }

    func buildCancelInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-cancel-request.v4.json")
        return fixture("mission-cancel-invocation.v4.json")
    }

    func buildEventsInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-events-request.v4.json")
        return Data(
            """
            {
              "caller_ura": "easynet:///r/example/agent/alice.sdk",
              "callee_ura": "easynet:///r/example/device/dev-a",
              "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0",
              "subject_ura": "easynet:///r/example/device/dev-a",
              "descriptor_version": "1.0.0",
              "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
              "causal_context": {"form": "none"},
              "args": {
                "run_id": "2026-07-04_010203_weather",
                "cursor_sequence": 4,
                "limit": 100
              },
              "metadata": {"request_id": "mission-events-1", "profile": "mission", "system_ability": "mission.events", "carrier_owner": "daemon_sdk"}
            }
            """.utf8
        )
    }

    func runEAL(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-run-request.v4.json")
        return fixture("mission-status.v4.json")
    }

    func runFile(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-run-file-request.v4.json")
        return fixture("mission-status.v4.json")
    }

    func track(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-track-request.v4.json")
        return fixture("mission-status.v4.json")
    }

    func cancel(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-cancel-request.v4.json")
        return fixture("mission-status.v4.json")
    }

    func events(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "mission-events-request.v4.json")
        return fixture("mission-event-page.v4.json")
    }

    func openEventStream(_ requestJSON: Data) async throws -> StreamSource {
        try expectJSON(requestJSON, equalsFixture: "mission-events-request.v4.json")
        return MissionEventStreamSource()
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureReceiptTransport: ReceiptTransport, @unchecked Sendable {
    func fetch(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "receipt fetch request")
        let expected = try decodeObject(fixture("receipt-fetch-request.v4.json"), label: "receipt fetch fixture")
        XCTAssertEqual(request, expected)
        return fixture("receipt.summary.v4.json")
    }

    func project(_ receiptJSON: Data) async throws -> Data {
        let request = try decodeObject(receiptJSON, label: "receipt projection request")
        let expected = try decodeObject(fixture("receipt.summary.v4.json"), label: "receipt summary fixture")
        XCTAssertEqual(request, expected)
        return fixture("receipt.summary.v4.json")
    }

    func verify(_ receiptJSON: Data) async throws -> Data {
        let request = try decodeObject(receiptJSON, label: "receipt verification request")
        let expected = try decodeObject(fixture("receipt-ref.v4.json"), label: "receipt ref fixture")
        XCTAssertEqual(request, expected)
        return Data("""
        {
          "verified": true,
          "method": "axon-signature-chain",
          "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
          "invocation_id": "inv-example-1",
          "reason": "",
          "metadata": {"assurance": "axon-cryptographic"}
        }
        """.utf8)
    }

    func verifyChain(_ requestJSON: Data) async throws -> Data {
        let request = try decodeObject(requestJSON, label: "receipt chain request")
        guard case let .array(receipts) = request["receipts"] else {
            XCTFail("receipt chain request missing receipts")
            return Data()
        }
        XCTAssertEqual(receipts.count, 1)
        XCTAssertEqual(request["metadata"], .object([:]))
        return Data("""
        {
          "verified": true,
          "method": "axon-cross-invocation-dag",
          "root_receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
          "terminal_receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
          "items": [
            {
              "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/receipt-1/receipt",
              "verified": true
            }
          ],
          "metadata": {"parent_dag_closed": true}
        }
        """.utf8)
    }
}

final class EmptyReceiptTransport: ReceiptTransport, @unchecked Sendable {}

final class FixturePublicationTransport: PublicationTransport, @unchecked Sendable {
    func buildResourceRef(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "local-resource-ref-request.v4.json")
        return fixture("resource-ref.local-fs.v4.json")
    }

    func validatePackage(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        let manifest = try decodedObject(fixture("ability-package-manifest.v4.json"))
        XCTAssertEqual(request as NSDictionary, ["manifest": manifest] as NSDictionary)
        return fixture("package-validation.v4.json")
    }

    func buildDeployInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "ability-deploy-request.v4.json")
        return fixture("publication-deploy-invocation.v4.json")
    }

    func buildUnpublishInvocation(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        XCTAssertEqual(request["ability_ura"] as? String, "easynet:///r/example/ability/device.dev-a.er.weather")
        return fixture("publication-unpublish-invocation.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureHostBindingTransport: HostBindingTransport, @unchecked Sendable {
    func buildHostStreamBinding(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "host-stream-binding-request.v4.json")
        return fixture("host-stream-binding.v4.json")
    }

    func decodeRequest(_ envelopeJSON: Data) async throws -> Data {
        let envelope = try decodedObject(envelopeJSON)
        let request = try XCTUnwrap(envelope["request"] as? [String: Any])
        XCTAssertEqual(request["fn"] as? String, "weather.stream")
        return fixture("host-stream-request.v4.json")
    }

    func encodeItem(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        XCTAssertEqual((request["seq"] as? NSNumber)?.intValue, 0)
        return fixture("host-stream-frame.v4.json")
    }

    func encodeError(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        let error = try XCTUnwrap(request["error"] as? [String: Any])
        XCTAssertEqual(error["code"] as? String, "INVALID_ARGUMENT")
        return Data("""
        {
          "frame_type": "error",
          "seq": null,
          "value": null,
          "error": {
            "code": "INVALID_ARGUMENT",
            "stage": "host",
            "message": "bad input",
            "retry": "never",
            "details": {}
          },
          "terminal": null,
          "output_hash": null
        }
        """.utf8)
    }

    func encodeTerminal(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        let summary = try XCTUnwrap(request["summary"] as? [String: Any])
        XCTAssertEqual(summary as NSDictionary, try decodedObject(fixture("host-stream-terminal.v4.json")) as NSDictionary)
        return try JSONSerialization.data(withJSONObject: [
            "frame_type": "terminal",
            "seq": summary["frames"] as Any,
            "value": NSNull(),
            "error": NSNull(),
            "terminal": summary,
            "output_hash": summary["output_hash"] as Any,
        ], options: [.sortedKeys])
    }

    func foldOutputHash(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        XCTAssertEqual((request["seq"] as? NSNumber)?.intValue, 0)
        return fixture("host-stream-hash-state.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureHostLifecycleProvider: HostStreamLifecycleProvider, @unchecked Sendable {
    var cleanupCalls = 0

    func checkReadiness(_ binding: HostStreamBinding) async throws -> HostStreamReadiness {
        try HostStreamReadiness(
            state: "ready",
            checked: true,
            endpointReady: true,
            metadata: ["endpoint": .string(binding.endpoint)]
        )
    }

    func cleanup(_ binding: HostStreamBinding) async throws -> HostStreamCleanup {
        cleanupCalls += 1
        return try HostStreamCleanup(mode: "unlink_socket", metadata: ["cleaned": .bool(true)])
    }
}

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

final class FixtureAdminTransport: AdminTransport, @unchecked Sendable {
    func buildAgentListInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-list-request.v4.json")
        return fixture("admin-agent-list-invocation.v4.json")
    }

    func buildAgentStartInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-start-request.v4.json")
        return fixture("admin-agent-start-invocation.v4.json")
    }

    func buildAgentStopInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-stop-request.v4.json")
        return fixture("admin-agent-stop-invocation.v4.json")
    }

    func buildAgentRefreshInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-refresh-request.v4.json")
        return fixture("admin-agent-refresh-invocation.v4.json")
    }

    func buildSessionListInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-session-list-request.v4.json")
        return fixture("admin-session-list-invocation.v4.json")
    }

    func gatewayStatus(_ requestJSON: Data) async throws -> Data {
        XCTAssertTrue(try decodedObject(requestJSON).isEmpty)
        return fixture("gateway-status.v4.json")
    }

    func listAgents(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-list-request.v4.json")
        return fixture("admin-agent-records.v4.json")
    }

    func agentStart(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-start-request.v4.json")
        return fixture("admin-agent-lifecycle-result.v4.json")
    }

    func agentStop(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-stop-request.v4.json")
        return fixture("admin-agent-lifecycle-result.v4.json")
    }

    func agentRefresh(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-agent-refresh-request.v4.json")
        return fixture("admin-agent-lifecycle-result.v4.json")
    }

    func pairingPreflight(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-pairing-preflight-request.v4.json")
        return fixture("admin-pairing-preflight.v4.json")
    }

    func createPairing(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-pairing-create-request.v4.json")
        return fixture("admin-pairing-token.v4.json")
    }

    func validatePairing(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-pairing-validate-request.v4.json")
        return fixture("admin-device-credential.v4.json")
    }

    func createDeviceSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-device-session-create-request.v4.json")
        return fixture("admin-device-session.v4.json")
    }

    func listDeviceSessions(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-session-list-request.v4.json")
        return fixture("admin-device-session-page.v4.json")
    }

    func deleteDeviceSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "admin-device-session-delete-request.v4.json")
        return fixture("admin-device-session-delete-result.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureSurfaceTransport: SurfaceTransport, @unchecked Sendable {
    func buildListPagesInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-list-pages-request.v4.json")
        return fixture("surface-list-pages-invocation.v4.json")
    }

    func buildCreatePageInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-create-page-request.v4.json")
        return fixture("surface-create-page-invocation.v4.json")
    }

    func buildDeletePageInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-delete-page-request.v4.json")
        return fixture("surface-delete-page-invocation.v4.json")
    }

    func buildManifestInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-manifest-request.v4.json")
        return fixture("surface-manifest-invocation.v4.json")
    }

    func buildHealthInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-health-request.v4.json")
        return fixture("surface-health-invocation.v4.json")
    }

    func listPages(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-list-pages-request.v4.json")
        return fixture("surface-page-page.v4.json")
    }

    func createPage(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-create-page-request.v4.json")
        return fixture("surface-page-record.v4.json")
    }

    func deletePage(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-delete-page-request.v4.json")
        return fixture("surface-mutation-result.v4.json")
    }

    func surfaceManifest(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-manifest-request.v4.json")
        return fixture("surface-manifest.v4.json")
    }

    func publicPageRef(_ pageJSON: Data) async throws -> Data {
        let page = try decodedObject(pageJSON)
        XCTAssertEqual(page["page_id"] as? String, "docs")
        return fixture("surface-public-page-ref.v4.json")
    }

    func surfaceHealth(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "surface-health-request.v4.json")
        return fixture("surface-health.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureWrapperTransport: WrapperTransport, @unchecked Sendable {
    func projectFileRecord(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "wrapper-file-record.v4.json")
        return fixture("wrapper-file-record.v4.json")
    }

    func projectTerminalSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "wrapper-terminal-session.v4.json")
        return fixture("wrapper-terminal-session.v4.json")
    }

    func projectRemoteDesktopSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "wrapper-remote-desktop-session.v4.json")
        return fixture("wrapper-remote-desktop-session.v4.json")
    }

    func projectBrowserSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "wrapper-browser-session.v4.json")
        return fixture("wrapper-browser-session.v4.json")
    }

    func projectMediaSession(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "wrapper-media-session.v4.json")
        return fixture("wrapper-media-session.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }
}

final class FixtureAuthorityTransport: AuthorityTransport, @unchecked Sendable {
    private let delegationValue: String
    private let sessionValue: String

    init(delegationValue: String, sessionValue: String) {
        self.delegationValue = delegationValue
        self.sessionValue = sessionValue
    }

    func mintDelegationProof(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        XCTAssertEqual(request["issuer_ura"] as? String, "easynet:///r/example/user/alice")
        XCTAssertEqual(request["caller_ura"] as? String, "easynet:///r/example/agent/backend")
        XCTAssertEqual((request["scopes"] as? [Any])?.count, 1)
        return try JSONSerialization.data(withJSONObject: ["metadata_value": delegationValue], options: [.sortedKeys])
    }

    func mintSessionAuthority(_ requestJSON: Data) async throws -> Data {
        let request = try decodedObject(requestJSON)
        XCTAssertEqual(request["issuer_ura"] as? String, "easynet:///r/example/agent/backend")
        XCTAssertEqual(request["session_id"] as? String, "session-1")
        XCTAssertEqual(request["creator_principal_id"] as? String, "easynet:///r/example/agent/backend")
        XCTAssertEqual(request["audience"] as? String, "easynet:///r/example/device/dev-a")
        return try JSONSerialization.data(
            withJSONObject: ["metadata": [sessionAuthorityMetadataKey: sessionValue]],
            options: [.sortedKeys]
        )
    }
}

final class FixtureCompatibilityTransport: CompatibilityTransport, @unchecked Sendable {
    func buildListModelsInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-list-models-request.v4.json")
        return fixture("compatibility-list-models-invocation.v4.json")
    }

    func buildChatCompletionInvocation(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-chat-completion-request.v4.json")
        return fixture("compatibility-chat-completion-invocation.v4.json")
    }

    func buildStreamChatCompletionInvocation(_ requestJSON: Data) async throws -> Data {
        try expectStreamJSON(requestJSON)
        return fixture("compatibility-stream-chat-completion-invocation.v4.json")
    }

    func listModels(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-list-models-request.v4.json")
        return fixture("compatibility-model-page.v4.json")
    }

    func chatCompletions(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-chat-completion-request.v4.json")
        return fixture("compatibility-chat-completion.v4.json")
    }

    func streamChatCompletions(_ requestJSON: Data) async throws -> Data {
        try expectStreamJSON(requestJSON)
        return fixture("compatibility-chat-stream.v4.json")
    }

    func uploadFile(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-file-upload-request.v4.json")
        return fixture("compatibility-file.v4.json")
    }

    func getFile(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-file-request.v4.json")
        return fixture("compatibility-file.v4.json")
    }

    func deleteFile(_ requestJSON: Data) async throws -> Data {
        try expectJSON(requestJSON, equalsFixture: "compatibility-file-delete-request.v4.json")
        return fixture("compatibility-file-delete-result.v4.json")
    }

    func projectModelPage(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-model-page.v4.json")
        return valueJSON
    }

    func projectChatCompletion(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-chat-completion.v4.json")
        return valueJSON
    }

    func projectChatStream(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-chat-stream.v4.json")
        return valueJSON
    }

    func projectFileUpload(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-file-upload-request.v4.json")
        return fixture("compatibility-file.v4.json")
    }

    func projectFile(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-file-request.v4.json")
        return fixture("compatibility-file.v4.json")
    }

    func projectFileDeleteResult(_ valueJSON: Data) async throws -> Data {
        try expectJSON(valueJSON, equalsFixture: "compatibility-file-delete-request.v4.json")
        return fixture("compatibility-file-delete-result.v4.json")
    }

    private func expectJSON(_ data: Data, equalsFixture fixtureName: String) throws {
        let request = try decodedObject(data)
        let expected = try decodedObject(fixture(fixtureName))
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
    }

    private func expectStreamJSON(_ data: Data) throws {
        var request = try decodedObject(data)
        var expected = try decodedObject(fixture("compatibility-stream-chat-completion-request.v4.json"))
        var expectedRequest = expected["request"] as? [String: Any] ?? [:]
        expectedRequest["stream"] = true
        expected["request"] = expectedRequest
        XCTAssertEqual(request as NSDictionary, expected as NSDictionary)
        request.removeAll()
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
