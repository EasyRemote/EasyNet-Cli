import CryptoKit
import Foundation
import XCTest
@testable import RuntimeSDK

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
        let sourceDirectory = packageRoot.appendingPathComponent("Sources/RuntimeSDK")
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
                "RuntimeAbilityProjection.swift",
                "SDKError.swift",
                "Stream.swift",
            ])
        )

        let downstreamProfileSymbols = [
            "WorkflowClient",
            "WorkflowTransport",
            "ApplicationLifecycleClient",
            "ApplicationDirectoryView",
            "ApplicationReceiptPage",
            "ApplicationEventClient",
            "HostIntegrationClient",
            "PublicationWorkflowClient",
            "TranslationLayer",
            "ConvenienceWrapperClient",
            "ProfileBundle",
            "ServiceLocator",
        ]
        for sourceName in sourceNames {
            let source = try String(
                contentsOf: sourceDirectory.appendingPathComponent(sourceName),
                encoding: .utf8
            )
            for symbol in downstreamProfileSymbols {
                XCTAssertFalse(source.contains(symbol), "\(sourceName) exports \(symbol)")
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
        let transport = MemoryRuntimeTransport(callee: callee, descriptor: descriptor)
        let runtime = RuntimeClient(transport: transport)
        let draft = try completeDraft(runtime)

        let result = try await runtime.invoke(draft)
        XCTAssertTrue(result.ok)
        XCTAssertEqual(result.terminalReceipt["invocation_id"], "inv-direct")
        XCTAssertEqual(result.terminalReceiptProjection["invocation_id"], .string("inv-direct"))

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
        XCTAssertEqual(signed.policy?.mode, "provider_managed_signing")
        XCTAssertEqual(signed.policy?.signerId, "policy-signer-1")
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
        XCTAssertEqual(submittedSigner, "policy-signer-1")
        let submittedPolicyRef = await transport.submittedPolicyRef()
        XCTAssertEqual(submittedPolicyRef, "policy/local")
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
        let terminal = canonicalRuntimeReceipt(
            invocationId: "inv-result",
            receiptType: "completed",
            state: "Completed",
            index: 1,
            callee: callee,
            descriptor: descriptor
        )
        let canonical = try InvocationResult.fromJSON(
            jsonData([
                "ok": true,
                "terminal_state": "Completed",
                "terminal_receipt": terminal,
            ])
        )
        XCTAssertEqual(canonical.terminalReceipt["invocation_id"], "inv-result")
        XCTAssertEqual(canonical.terminalReceiptProjection["invocation_id"], .string("inv-result"))
        guard case let .object(projectedAuthorityProof)? = canonical.terminalReceiptProjection["authority_proof"] else {
            XCTFail("InvocationResult terminalReceiptProjection must retain authority_proof")
            return
        }
        XCTAssertEqual(projectedAuthorityProof["proof_type"], .string("self"))
        guard case let .object(projectedProofBinding)? = projectedAuthorityProof["binding"] else {
            XCTFail("InvocationResult terminalReceiptProjection must retain authority_proof.binding")
            return
        }
        XCTAssertEqual(projectedProofBinding["principal_ura"], .string(callee))
        guard case let .object(projectedAuthorityBinding)? = canonical.terminalReceiptProjection["authority_binding"] else {
            XCTFail("InvocationResult terminalReceiptProjection must retain authority_binding")
            return
        }
        XCTAssertEqual(projectedAuthorityBinding["principal_ura"], .string(callee))

        for retiredState in ["completed", "COMPLETED", "TIMED_OUT", " Completed "] {
            var legacyStateReceipt = terminal
            legacyStateReceipt["state"] = retiredState
            expectSyncSDKError(.invalidArgument, "unknown receipt state") {
                _ = try RuntimeReceipt(legacyStateReceipt)
            }
        }

        var unspecifiedStateReceipt = terminal
        unspecifiedStateReceipt["state"] = "Unspecified"
        expectSyncSDKError(.invalidArgument, "runtime receipt lifecycle state must not be UNSPECIFIED") {
            _ = try RuntimeReceipt(unspecifiedStateReceipt)
        }

        var topLevelLegacyField = terminal
        topLevelLegacyField["legacy_receipt_canonicalizer"] = "java-compatible-raw"
        expectSyncSDKError(.invalidArgument, "runtime_receipt contains noncanonical field legacy_receipt_canonicalizer") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": topLevelLegacyField,
                ])
            )
        }

        var proofLegacyField = terminal
        var proofWithLegacyField = proofLegacyField["authority_proof"] as! [String: Any]
        proofWithLegacyField["legacy_signature_payload"] = "opaque"
        proofLegacyField["authority_proof"] = proofWithLegacyField
        expectSyncSDKError(.invalidArgument, "authority_proof contains noncanonical field legacy_signature_payload") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": proofLegacyField,
                ])
            )
        }

        var missingProofPayload = terminal
        var proofWithoutPayload = missingProofPayload["authority_proof"] as! [String: Any]
        proofWithoutPayload.removeValue(forKey: "proof_payload_base64")
        missingProofPayload["authority_proof"] = proofWithoutPayload
        expectSyncSDKError(.invalidArgument, "runtime receipt summary is missing authority_proof.proof_payload_base64") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": missingProofPayload,
                ])
            )
        }

        for missingField in ["payload_base64", "payload_content_type", "host_attestation_base64", "usage"] {
            var missingTopLevelFact = terminal
            missingTopLevelFact.removeValue(forKey: missingField)
            expectSyncSDKError(.invalidArgument, "runtime receipt summary is missing runtime_receipt.\(missingField)") {
                _ = try InvocationResult.fromJSON(
                    jsonData([
                        "ok": true,
                        "terminal_state": "Completed",
                        "terminal_receipt": missingTopLevelFact,
                    ])
                )
            }
        }

        var causalLegacyField = terminal
        causalLegacyField["causal_binding"] = ["form": "none", "legacy_parent": "opaque"]
        expectSyncSDKError(.invalidArgument, "causal_binding contains noncanonical field legacy_parent") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": causalLegacyField,
                ])
            )
        }

        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationResult.fromJSON(
                Data(
                    #"{"ok":true,"terminal_state":"Completed","receipt":{"receipt_ref":"legacy-only"}}"#
                        .utf8
                )
            )
        }
        expectSyncSDKError(.invalidArgument, "unknown terminal state BackpressureTerminated") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": false,
                    "terminal_state": "BackpressureTerminated",
                    "terminal_receipt": canonicalRuntimeReceipt(
                        invocationId: "inv-backpressure",
                        receiptType: "failed",
                        state: "Failed",
                        index: 1,
                        callee: callee,
                        descriptor: descriptor
                    ),
                ])
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationResult.fromJSON(
                Data(#"{"ok":true,"terminal_state":"Completed"}"#.utf8)
            )
        }
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationResult.fromJSON(
                Data(#"{"ok":true,"terminal_state":"Completed","terminal_receipt":"bad"}"#.utf8)
            )
        }
        var mismatchedProofHash = terminal
        var authorityProof = mismatchedProofHash["authority_proof"] as! [String: Any]
        authorityProof["proof_hash_hex"] = String(repeating: "ff", count: 32)
        mismatchedProofHash["authority_proof"] = authorityProof
        expectSyncSDKError(.invalidArgument, "authority_proof_hash_mismatch") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": mismatchedProofHash,
                ])
            )
        }

        var bindingHashReceipt = terminal
        var bindingHashProof = bindingHashReceipt["authority_proof"] as! [String: Any]
        bindingHashProof["proof_payload_base64"] = ""
        bindingHashProof["proof_hash_hex"] = authorityBindingProofHashSelf(callee)
        bindingHashProof.removeValue(forKey: "signature")
        bindingHashReceipt["authority_proof"] = bindingHashProof
        let bindingHash = try InvocationResult.fromJSON(
            jsonData([
                "ok": true,
                "terminal_state": "Completed",
                "terminal_receipt": bindingHashReceipt,
            ])
        )
        XCTAssertEqual(bindingHash.terminalReceipt["invocation_id"], "inv-result")

        let sessionBinding: [String: Any] = [
            "kind": "session",
            "issuer_ura": "easynet:///r/example/agent/backend",
            "subject_ura": "easynet:///r/example/agent/alice",
            "session_id": "session-1",
            "scopes": ["invoke"],
            "audiences": [descriptor],
            "issued_at_ms": 1,
            "expires_at_ms": 2,
            "signature_base64": Data(repeating: 0x73, count: 64).base64EncodedString(),
        ]
        var sessionReceipt = terminal
        sessionReceipt["authority_binding_kind"] = "session"
        sessionReceipt["authority_binding"] = sessionBinding
        var sessionProof = sessionReceipt["authority_proof"] as! [String: Any]
        sessionProof["proof_type"] = "session"
        sessionProof["binding_kind"] = "session"
        sessionProof["binding"] = sessionBinding
        sessionProof["proof_payload_base64"] = ""
        sessionProof["proof_hash_hex"] = authorityBindingProofHashSession(sessionBinding)
        sessionProof.removeValue(forKey: "signature")
        sessionReceipt["authority_proof"] = sessionProof
        let sessionResult = try InvocationResult.fromJSON(
            jsonData([
                "ok": true,
                "terminal_state": "Completed",
                "terminal_receipt": sessionReceipt,
            ])
        )
        XCTAssertEqual(sessionResult.terminalReceipt["authority_binding_kind"], "session")
        guard case let .object(sessionProjection)? = sessionResult.terminalReceiptProjection["authority_binding"] else {
            XCTFail("InvocationResult terminalReceiptProjection must retain session authority_binding")
            return
        }
        XCTAssertEqual(sessionProjection["session_id"], .string("session-1"))

        let retiredSessionBinding: [String: Any] = [
            "kind": "session",
            "backend_ura": "easynet:///r/example/agent/backend",
            "user_ura": "easynet:///r/example/agent/alice",
            "session_id": "session-1",
            "scopes": ["invoke"],
            "audiences": [descriptor],
            "issued_at_ms": 1,
            "expires_at_ms": 2,
            "signature_base64": Data(repeating: 0x73, count: 64).base64EncodedString(),
        ]
        var retiredSessionReceipt = terminal
        retiredSessionReceipt["authority_binding_kind"] = "session"
        retiredSessionReceipt["authority_binding"] = retiredSessionBinding
        var retiredSessionProof = retiredSessionReceipt["authority_proof"] as! [String: Any]
        retiredSessionProof["proof_type"] = "session"
        retiredSessionProof["binding_kind"] = "session"
        retiredSessionProof["binding"] = retiredSessionBinding
        retiredSessionProof["proof_payload_base64"] = ""
        retiredSessionReceipt["authority_proof"] = retiredSessionProof
        expectSyncSDKError(.invalidArgument, "authority_binding contains noncanonical field backend_ura") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": retiredSessionReceipt,
                ])
            )
        }

        var wrongIssuer = terminal
        var wrongIssuerProof = wrongIssuer["authority_proof"] as! [String: Any]
        wrongIssuerProof["issuer"] = [
            "ura": "easynet:///r/example/device/other",
            "profile": "axon-strict-v2",
        ]
        wrongIssuer["authority_proof"] = wrongIssuerProof
        expectSyncSDKError(.invalidArgument, "authority_proof issuer does not match callee_binding") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": wrongIssuer,
                ])
            )
        }

        for retiredProfile in ["axon-legacy-v1", "opaque"] {
            var retiredCalleeProfile = terminal
            var calleeBinding = retiredCalleeProfile["callee_binding"] as! [String: Any]
            calleeBinding["profile"] = retiredProfile
            retiredCalleeProfile["callee_binding"] = calleeBinding
            expectSyncSDKError(.invalidArgument, "callee_binding.profile is not canonical") {
                _ = try InvocationResult.fromJSON(
                    jsonData([
                        "ok": true,
                        "terminal_state": "Completed",
                        "terminal_receipt": retiredCalleeProfile,
                    ])
                )
            }
        }

        var hostedSignerWithoutAttestation = terminal
        hostedSignerWithoutAttestation["signer_binding"] = [
            "ura": "easynet:///r/example/device/runtime-host",
            "profile": "axon-strict-v2",
        ]
        expectSyncSDKError(.invalidArgument, "hosted runtime receipt is missing host_attestation_base64") {
            _ = try InvocationResult.fromJSON(
                jsonData([
                    "ok": true,
                    "terminal_state": "Completed",
                    "terminal_receipt": hostedSignerWithoutAttestation,
                ])
            )
        }
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

        let mismatchedDelegation = try DelegationProof.fromMetadata(authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": "easynet:///r/example/user/alice",
            "caller_ura": caller,
            "audience": callee,
            "scopes": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ]))
        expectSyncSDKError(.authoritySubjectMismatch, "delegation authority subject does not match invocation subject_ura") {
            _ = try completeBuilder()
                .withAuthorityMetadata(mismatchedDelegation.metadata())
                .inspect()
        }

        let scopedSession = try SessionAuthority.fromMetadata(sessionMetadataValue(scopes: ["observe.health"]))
        _ = try completeBuilder()
            .withSubjectURA(try runtimeStateReadSubjectURA(realm: "example", userID: "alice"))
            .withAuthorityMetadata(scopedSession.metadata())
            .inspect()
        _ = try completeBuilder()
            .withSubjectURA("easynet:///r/example/resource/agent.alice.sdk/runtime-state/read")
            .withAuthorityMetadata(scopedSession.metadata())
            .inspect()
        expectSyncSDKError(.authoritySubjectMismatch, "session authority subject does not admit invocation subject_ura") {
            _ = try completeBuilder()
                .withSubjectURA("not-a-ura/resource/user.alice/runtime-state/read")
                .withAuthorityMetadata(scopedSession.metadata())
                .inspect()
        }
        expectSyncSDKError(.authoritySubjectMismatch, "session authority subject does not admit invocation subject_ura") {
            _ = try completeBuilder()
                .withSubjectURA("easynet:///r/example/device/dev-a/resource/user.alice/runtime-state/read")
                .withAuthorityMetadata(scopedSession.metadata())
                .inspect()
        }

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

    func testRuntimeAbilityProjectionIsCanonical() throws {
        let admittedScopes = [
            "observe.health",
            "easynet:///r/example/ability/device.dev-a.observe.health",
            "easynet:///r/example/ability/device.dev-a.*",
        ]
        for scope in admittedScopes {
            let proof = try DelegationProof.fromMetadata(delegationMetadataValue(scopes: [scope]))
            _ = try completeBuilder()
                .withAuthorityMetadata(proof.metadata())
                .inspect()
        }

        let ownerQualifiedProof = try DelegationProof.fromMetadata(
            delegationMetadataValue(scopes: ["device.dev-a.observe.health"])
        )
        expectSyncSDKError(.authorityDenied, "delegation authority scopes do not admit invocation ability") {
            _ = try completeBuilder()
                .withAuthorityMetadata(ownerQualifiedProof.metadata())
                .inspect()
        }

        let nestedDeviceCallee = "easynet:///r/example/resource/user.alice/archive/device/dev-a"
        let nestedDeviceProof = try DelegationProof.fromMetadata(authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": nestedDeviceCallee,
            "caller_ura": caller,
            "audience": nestedDeviceCallee,
            "scopes": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ]))
        expectSyncSDKError(.authorityDenied, "delegation authority scopes do not admit invocation ability") {
            _ = try completeBuilder()
                .withCalleeURA(nestedDeviceCallee)
                .withSubjectURA(nestedDeviceCallee)
                .withAuthorityMetadata(nestedDeviceProof.metadata())
                .inspect()
        }

        let authoritySubject = "easynet:///r/example/resource/user.alice/invoke/namespace.resolve"
        let authorityProof = try DelegationProof.fromMetadata(authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": authoritySubject,
            "caller_ura": caller,
            "audience": "easynet:///r/example/authority",
            "scopes": ["namespace.resolve"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ]))
        _ = try completeBuilder()
            .withCalleeURA("easynet:///r/example/authority")
            .withDescriptorRef(
                "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
            )
            .withSubjectURA(authoritySubject)
            .withAuthorityMetadata(authorityProof.metadata())
            .inspect()

        let mismatchedOwnerProof = try DelegationProof.fromMetadata(authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": callee,
            "caller_ura": caller,
            "audience": callee,
            "scopes": ["namespace.resolve"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ]))
        expectSyncSDKError(.authorityDenied, "delegation authority scopes do not admit invocation ability") {
            _ = try completeBuilder()
                .withDescriptorRef(
                    "easynet:///r/example/ability/authority.namespace.resolve@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
                )
                .withAuthorityMetadata(mismatchedOwnerProof.metadata())
                .inspect()
        }

        let proof = try DelegationProof.fromMetadata(delegationMetadataValue(scopes: ["observe.health"]))
        expectSyncSDKError(.invalidArgument, "descriptor_ref must contain a canonical Ability URA") {
            _ = try completeBuilder()
                .withDescriptorRef("observe.health")
                .withAuthorityMetadata(proof.metadata())
                .inspect()
        }
    }

    func testRuntimeStateReadSubjectHelperBuildsUserOwnedResourceSubject() throws {
        XCTAssertEqual(
            try runtimeStateReadSubjectURA(realm: "example", userID: "alice"),
            "easynet:///r/example/resource/user.alice/runtime-state/read"
        )
        expectSyncSDKError(.invalidArgument, "user_id must not be all-zero") {
            _ = try runtimeStateReadSubjectURA(
                realm: "example",
                userID: "00000000-0000-0000-0000-000000000000"
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

    func testAuthorityMetadataBindsSessionAuthoritySubjects() throws {
        let userMismatch = try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "alice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/user/bob",
            "audience": callee,
            "scopes": ["invoke"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])

        expectSyncSDKError(.invalidArgument, "session authority user subject must match session_owner_user_id") {
            _ = try SessionAuthority.fromMetadata(userMismatch)
        }

        let sessionMismatch = try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "alice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/resource/user.alice/session/session-2",
            "audience": callee,
            "scopes": ["invoke"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])

        expectSyncSDKError(.invalidArgument, "session authority subject_ura owner/session must match session_owner_user_id and session_id") {
            _ = try SessionAuthority.fromMetadata(sessionMismatch)
        }

        let dottedOwner = try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "teamalice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/resource/user.team.alice/session/session-1",
            "audience": callee,
            "scopes": ["invoke"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])

        expectSyncSDKError(.invalidArgument, "session authority subject_ura must be a canonical user or session subject") {
            _ = try SessionAuthority.fromMetadata(dottedOwner)
        }

        expectSyncSDKError(.invalidArgument, "session authority subject_ura must be a canonical user or session subject") {
            _ = try SessionAuthorityRequest(
                issuerURA: caller,
                sessionID: "session-1",
                sessionOwnerUserID: "alice",
                creatorPrincipalID: caller,
                calleeURA: callee,
                subjectURA: callee,
                audience: callee,
                scopes: ["invoke"],
                allowedActions: ["invoke"],
                allowedFollowupAbilities: ["observe.health"],
                issuedAtMS: 10,
                expiresAtMS: 20
            )
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
        let transport = MemoryRuntimeTransport(callee: callee, descriptor: descriptor)
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
            .routing
        )
        XCTAssertEqual(
            SDKError(code: .callerIdentityUnavailable, stage: "caller_identity", message: "missing identity").errorClass,
            .permission
        )
        XCTAssertEqual(
            SDKError(code: .callerSignerUnavailable, stage: "caller_identity", message: "missing signer").errorClass,
            .admission
        )
        XCTAssertEqual(
            SDKError(code: .descriptorNotFound, stage: "routing", message: "missing descriptor").errorClass,
            .routing
        )
        XCTAssertEqual(
            SDKError(code: .runtimeOffline, stage: "transport", message: "offline").errorClass,
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
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport(callee: callee, descriptor: descriptor))
        let prepared = try await runtime.prepare(completeDraft(runtime), options: ["deadline_ms": 1000])
        XCTAssertEqual(prepared.signingMaterial.descriptorRef, descriptor)
        XCTAssertEqual(Data(base64Encoded: prepared.signingMaterial.canonicalBytesBase64), Data("canonical".utf8))
    }

    func testPreparedInvocationRequiresExplicitDescriptorRef() throws {
        var prepared = try preparedInvocationWire()
        prepared.removeValue(forKey: "descriptor_ref")
        expectSyncSDKError(.invalidArgument, "descriptor_ref is required") {
            _ = try PreparedInvocation.fromJSON(jsonData(prepared))
        }
    }

    func testPreparedInvocationRejectsRequestIDOnlyAlias() throws {
        var prepared = try preparedInvocationWire()
        prepared.removeValue(forKey: "prepared_id")
        expectSyncSDKError(.invalidArgument, "prepared_id is required") {
            _ = try PreparedInvocation.fromJSON(jsonData(prepared))
        }
        expectSyncSDKError(.invalidArgument, "prepared_id is required") {
            _ = try PreparedInvocation(
                preparedId: "",
                requestId: "request-1",
                draft: completeBuilder().build(),
                signingMaterial: SigningMaterial(
                    algorithm: "ed25519",
                    canonicalBytesBase64: "Y2Fub25pY2Fs",
                    argsDigestHex: String(repeating: "a", count: 64),
                    descriptorRef: descriptor,
                    expiresAtUnixMS: 4_102_444_800_000
                ),
                descriptorRef: descriptor,
                expiresAtUnixMS: 4_102_444_800_000
            )
        }
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

    func testCompleteTupleRejectsAllZeroPrincipals() {
        let placeholder = "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/session/invocation_history"
        for mutate in [
            { self.completeBuilder().withCallerURA(placeholder) },
            { self.completeBuilder().withCalleeURA(placeholder) },
            { self.completeBuilder().withSubjectURA(placeholder) },
        ] {
            expectSyncSDKError(.invalidArgument) {
                _ = try mutate().inspect()
            }
        }
    }

    func testCompleteTupleRejectsRetiredInvocationHistorySubjectAuthorityCarrier() throws {
        let retiredSubject = "easynet:///r/example/resource/user.alice/session/invocation_history"
        let metadata = try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "invocation_history",
            "session_owner_user_id": "alice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": retiredSubject,
            "audience": callee,
            "scopes": ["observe.health"],
            "allowed_actions": ["invoke"],
            "allowed_followup_abilities": ["observe.health"],
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])
        expectSyncSDKError(.invalidArgument, "retired invocation-history subject") {
            _ = try completeBuilder()
                .withSubjectURA(retiredSubject)
                .withMetadata([sessionAuthorityMetadataKey: .string(metadata)])
                .inspect()
        }
    }

    func testCompleteTupleRejectsReceiptHistoryPublicInvocation() {
        let historyDescriptor = "easynet:///r/example/ability/device.dev-a.invocation.history.list@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
        expectSyncSDKError(.invalidArgument, "RuntimeReceiptProvider") {
            _ = try completeBuilder()
                .withDescriptorRef(historyDescriptor)
                .inspect()
        }
    }

    func testCompleteTupleRejectsCatalogueReadPublicInvocation() {
        let catalogueDescriptor = "easynet:///r/example/ability/authority.meta.list_abilities@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
        expectSyncSDKError(.invalidArgument, "RuntimeAbilityDescriptorProvider") {
            _ = try completeBuilder()
                .withCalleeURA("easynet:///r/example/authority")
                .withSubjectURA("easynet:///r/example/authority")
                .withDescriptorRef(catalogueDescriptor)
                .inspect()
        }
    }

    func testPreparedInvocationCannotBeSubmitted() async throws {
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport(callee: callee, descriptor: descriptor))
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
        try delegationMetadataValue(scopes: ["observe.health"])
    }

    private func delegationMetadataValue(scopes: [String]) throws -> String {
        try authorityMetadataValue([
            "issuer_ura": "easynet:///r/example/user/alice",
            "subject_ura": callee,
            "caller_ura": caller,
            "audience": callee,
            "scopes": scopes,
            "issued_at_ms": 10,
            "expires_at_ms": 20,
        ])
    }

    private func sessionMetadataValue() throws -> String {
        try sessionMetadataValue(scopes: ["invoke"])
    }

    private func sessionMetadataValue(scopes: [String]) throws -> String {
        try authorityMetadataValue([
            "issuer_ura": caller,
            "session_id": "session-1",
            "session_owner_user_id": "alice",
            "creator_principal_id": caller,
            "callee_ura": callee,
            "subject_ura": "easynet:///r/example/user/alice",
            "audience": callee,
            "scopes": scopes,
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

    private func preparedInvocationWire() throws -> [String: Any] {
        [
            "prepared_id": "prepared-1",
            "request_id": "request-1",
            "tuple": try completeBuilder().build().inspectTuple().wireObject(),
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
        ]
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
    private var policyRef = ""
    private var eventHandleId: Int64 = 7
    private var openedBidi = 0

    private let callee: String

    init(callee: String, descriptor: String) {
        self.callee = callee
        self.descriptor = descriptor
    }

    func invoke(_ draft: InvocationDraft) throws -> InvocationResult {
        let receipt = try RuntimeReceipt(
            canonicalRuntimeReceipt(
                invocationId: "inv-direct",
                receiptType: "completed",
                state: "Completed",
                index: 1,
                callee: callee,
                descriptor: descriptor
            )
        )
        return try InvocationResult(
            ok: true,
            terminalState: .completed,
            outputJSON: "{\"ok\":true}",
            terminalReceiptProjection: receipt.rawProjection(),
            terminalReceipt: receipt.projection()
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
                    "signer_policy": [
                        "mode": "provider_managed_signing",
                        "signer_id": "policy-signer-1",
                        "policy_ref": "policy/local",
                        "expires_at_unix_ms": 4_102_444_800_000,
                    ],
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
        let policy = signed["policy"] as? [String: Any]
        policyRef = policy?["policy_ref"] as? String ?? ""
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
                "terminal_receipt": canonicalRuntimeReceipt(
                    invocationId: "inv-await",
                    receiptType: "completed",
                    state: "Completed",
                    index: 1,
                    callee: callee,
                    descriptor: descriptor
                ),
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

    func submittedPolicyRef() -> String {
        policyRef
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

private func jsonData(_ object: [String: Any]) throws -> Data {
    try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
}

private func canonicalRuntimeReceipt(
    invocationId: String,
    receiptType: String,
    state: String,
    index: Int,
    callee: String,
    descriptor: String
) -> [String: Any] {
    let proofPayload = Data("canonical-runtime-test-proof".utf8)
    return [
        "receipt_ura": "easynet:///r/example/resource/runtime/invocation/\(invocationId)/receipt/\(index)",
        "invocation_id": invocationId,
        "receipt_type": receiptType,
        "state": state,
        "index": index,
        "timestamp_unix_ms": 1_783_100_000_000 + index,
        "prev_receipt_hash_hex": String(repeating: "00", count: 32),
        "self_hash_hex": String(format: "%064x", index + 1),
        "payload_base64": "",
        "payload_content_type": "application/json",
        "cleanup_complete": !["admitted", "Admitted", "ADMITTED"].contains(state),
        "caller_binding": agentBinding("easynet:///r/example/agent/alice.sdk"),
        "callee_binding": agentBinding(callee),
        "subject_binding": agentBinding(callee),
        "invocation_nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
        "causal_binding_kind": "none",
        "causal_binding": ["form": "none"],
        "callee_signature": [
            "algorithm": "ed25519",
            "signature_base64": Data(repeating: 0x71, count: 64).base64EncodedString(),
        ],
        "signer_binding": agentBinding(callee),
        "authority_binding_kind": "self",
        "authority_binding": ["kind": "self", "principal_ura": callee],
        "ability_binding": descriptor,
        "host_attestation_base64": "",
        "usage": [String: Any](),
        "subject_ref": ["kind": 1, "ura": callee, "profile": "axon-strict-v2"],
        "descriptor_version": "1.0.0",
        "schema_hash_hex": String(repeating: "11", count: 32),
        "impl_hash_hex": String(repeating: "22", count: 32),
        "runtime_env": "swift-test",
        "authority_proof": [
            "proof_type": "self",
            "binding_kind": "self",
            "binding": ["kind": "self", "principal_ura": callee],
            "proof_payload_base64": proofPayload.base64EncodedString(),
            "proof_hash_hex": sha256Hex(proofPayload),
            "issuer": agentBinding(callee),
            "signature": [
                "algorithm": "ed25519",
                "signature_base64": Data(repeating: 0x72, count: 64).base64EncodedString(),
            ],
            "admission_hook": "test.runtime.admission",
        ],
        "input_hash_hex": String(repeating: "33", count: 32),
        "output_hash_hex": String(repeating: "44", count: 32),
        "parent_receipts": [],
    ]
}

private func agentBinding(_ ura: String) -> [String: String] {
    ["ura": ura, "profile": "axon-strict-v2"]
}

private func authorityBindingProofHashSelf(_ principalURA: String) -> String {
    var canonical = Data()
    canonical.append(0x01)
    canonical.appendLengthPrefixed(Data(principalURA.utf8))
    return sha256Hex(canonical)
}

private func authorityBindingProofHashSession(_ binding: [String: Any]) -> String {
    var canonical = Data()
    canonical.append(0x05)
    canonical.appendLengthPrefixed(Data((binding["issuer_ura"] as! String).utf8))
    canonical.appendLengthPrefixed(Data((binding["subject_ura"] as! String).utf8))
    canonical.appendLengthPrefixed(Data((binding["session_id"] as! String).utf8))
    let scopes = binding["scopes"] as! [String]
    canonical.appendUInt32(UInt32(scopes.count))
    for scope in scopes {
        canonical.appendLengthPrefixed(Data(scope.utf8))
    }
    let audiences = binding["audiences"] as! [String]
    canonical.appendUInt32(UInt32(audiences.count))
    for audience in audiences {
        canonical.appendLengthPrefixed(Data(audience.utf8))
    }
    canonical.appendInt64(Int64(binding["issued_at_ms"] as! Int))
    canonical.appendInt64(Int64(binding["expires_at_ms"] as! Int))
    let signature = Data(base64Encoded: binding["signature_base64"] as! String)!
    canonical.appendUInt32(UInt32(signature.count))
    canonical.append(signature)
    return sha256Hex(canonical)
}

private func sha256Hex(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

private extension Data {
    mutating func appendLengthPrefixed(_ data: Data) {
        appendUInt32(UInt32(data.count))
        append(data)
    }

    mutating func appendUInt32(_ value: UInt32) {
        var encoded = value.bigEndian
        Swift.withUnsafeBytes(of: &encoded) { append(contentsOf: $0) }
    }

    mutating func appendInt64(_ value: Int64) {
        var encoded = value.bigEndian
        Swift.withUnsafeBytes(of: &encoded) { append(contentsOf: $0) }
    }
}

private func expectSyncSDKError(
    _ code: SDKErrorCode,
    _ messageFragment: String = "",
    operation: () throws -> Void
) {
    do {
        try operation()
        XCTFail("expected SDKError \(code.rawValue)")
    } catch let error as SDKError {
        XCTAssertEqual(error.code, code)
        if !messageFragment.isEmpty {
            XCTAssertTrue(
                error.message.contains(messageFragment),
                "expected error message to contain \(messageFragment), got \(error.message)"
            )
        }
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
