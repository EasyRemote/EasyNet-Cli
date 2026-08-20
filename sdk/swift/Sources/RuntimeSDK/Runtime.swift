import CryptoKit
import Foundation

public enum InvocationTerminalState: String, Sendable {
    case completed = "Completed"
    case failed = "Failed"
    case cancelled = "Cancelled"
    case timedOut = "TimedOut"
}

public struct InvocationResult: Sendable {
    public let ok: Bool
    public let terminalState: InvocationTerminalState
    public let outputJSON: String
    public let error: SDKError?
    public let terminalReceiptProjection: [String: JSONValue]
    public let terminalReceipt: [String: String]

    public init(
        ok: Bool,
        terminalState: InvocationTerminalState,
        outputJSON: String = "",
        error: SDKError? = nil,
        terminalReceiptProjection: [String: JSONValue] = [:],
        terminalReceipt: [String: String] = [:]
    ) throws {
        if ok && error != nil {
            throw SDKError.validation("runtime", "ok result must not carry error")
        }
        if !ok && error == nil {
            throw SDKError.validation("runtime", "failed result must carry error")
        }
        self.ok = ok
        self.terminalState = terminalState
        self.outputJSON = outputJSON
        self.error = error
        self.terminalReceiptProjection = terminalReceiptProjection
        self.terminalReceipt = terminalReceipt
    }

    static func fromJSON(_ raw: Data) throws -> InvocationResult {
        let object = try runtimeJSONObject(raw, "invocation_result")
        if object.keys.contains("receipt") {
            throw SDKError.validation("invocation_result", "retired receipt alias is not accepted")
        }
        let terminalState = try runtimeInvocationTerminalState(
            runtimeRequiredString(object, "terminal_state", "invocation_result")
        )
        let terminalReceipt = try runtimeRequiredTerminalReceipt(object, terminalState: terminalState)
        return try InvocationResult(
            ok: try runtimeRequiredBool(object, "ok", "invocation_result"),
            terminalState: terminalState,
            outputJSON: runtimeOptionalJSONObjectString(object["output_json"]),
            error: nil,
            terminalReceiptProjection: terminalReceipt.rawProjection,
            terminalReceipt: terminalReceipt.summary
        )
    }
}

private func runtimeInvocationTerminalState(_ value: String) throws -> InvocationTerminalState {
    guard let state = InvocationTerminalState(rawValue: value) else {
        throw SDKError.validation("invocation_result", "unknown terminal state \(value)")
    }
    return state
}

public struct RuntimeReceipt {
    public let invocationId: String
    public let receiptType: String
    public let state: String
    private let raw: [String: Any]
    private let rawProjectionValue: [String: JSONValue]

    public init(_ raw: [String: Any]) throws {
        self.raw = raw
        guard let projection = try jsonValue(raw).objectValue else {
            throw SDKError.validation("runtime_receipt", "runtime receipt must be an object")
        }
        rawProjectionValue = projection
        invocationId = try runtimeRequiredString(raw, "invocation_id", "runtime_receipt")
        receiptType = try runtimeRequiredString(raw, "receipt_type", "runtime_receipt")
        state = try runtimeRequiredString(raw, "state", "runtime_receipt")
        try validateSummary()
    }

    public func lifecycleState() throws -> String {
        try RuntimeReceipt.canonicalLifecycleState(state)
    }

    public func rawProjection() -> [String: JSONValue] {
        rawProjectionValue
    }

    public func projection() -> [String: String] {
        rawProjectionValue.compactMapValues { value in
            if case let .string(text) = value {
                return text
            }
            return nil
        }
    }

    private func validateSummary() throws {
        let lifecycleState = try lifecycleState()
        guard lifecycleState != "UNSPECIFIED" else {
            throw SDKError.validation("runtime_receipt", "runtime receipt lifecycle state must not be UNSPECIFIED")
        }
        guard receiptType == (try RuntimeReceipt.canonicalReceiptType(lifecycleState)) else {
            throw SDKError.validation("runtime_receipt", "runtime receipt receipt_type does not match its lifecycle state")
        }
        _ = try runtimeReceiptHash(raw, "prev_receipt_hash_hex", allowZero: true)
        _ = try runtimeReceiptHash(raw, "self_hash_hex", allowZero: false)
        try RuntimeReceiptProofFacts.validate(raw)
    }

    static func canonicalLifecycleState(_ value: String) throws -> String {
        switch value {
        case "Accepted": return "ACCEPTED"
        case "Admitted": return "ADMITTED"
        case "Dispatched": return "DISPATCHED"
        case "Running": return "RUNNING"
        case "Completed": return "COMPLETED"
        case "Failed": return "FAILED"
        case "TimedOut": return "TIMED_OUT"
        case "Cancelled": return "CANCELLED"
        case "Unspecified": return "UNSPECIFIED"
        default: throw SDKError.validation("runtime_receipt", "unknown receipt state \(value)")
        }
    }

    static func canonicalReceiptType(_ lifecycleState: String) throws -> String {
        switch lifecycleState {
        case "ACCEPTED": return "accepted"
        case "ADMITTED": return "admitted"
        case "DISPATCHED": return "dispatched"
        case "RUNNING": return "running"
        case "COMPLETED": return "completed"
        case "FAILED": return "failed"
        case "TIMED_OUT": return "timed_out"
        case "CANCELLED": return "cancelled"
        default: throw SDKError.validation("runtime_receipt", "unknown canonical receipt lifecycle state \(lifecycleState)")
        }
    }
}

private enum RuntimeReceiptProofFacts {
    static func validate(_ raw: [String: Any]) throws {
        try runtimeRequireExactKeys(
            raw,
            "runtime_receipt",
            [
                "receipt_ura",
                "invocation_id",
                "receipt_type",
                "state",
                "index",
                "timestamp_unix_ms",
                "prev_receipt_hash_hex",
                "self_hash_hex",
                "payload_base64",
                "payload_content_type",
                "cleanup_complete",
                "caller_binding",
                "callee_binding",
                "subject_binding",
                "invocation_nonce_base64",
                "causal_binding_kind",
                "causal_binding",
                "callee_signature",
                "signer_binding",
                "host_attestation_base64",
                "authority_binding_kind",
                "authority_binding",
                "ability_binding",
                "usage",
                "subject_ref",
                "descriptor_version",
                "schema_hash_hex",
                "impl_hash_hex",
                "runtime_env",
                "authority_proof",
                "input_hash_hex",
                "output_hash_hex",
                "parent_receipts",
            ]
        )
        try runtimeRequireRequiredKeys(
            raw,
            "runtime_receipt",
            [
                "receipt_ura",
                "invocation_id",
                "receipt_type",
                "state",
                "index",
                "timestamp_unix_ms",
                "prev_receipt_hash_hex",
                "self_hash_hex",
                "payload_base64",
                "payload_content_type",
                "cleanup_complete",
                "caller_binding",
                "callee_binding",
                "subject_binding",
                "invocation_nonce_base64",
                "causal_binding_kind",
                "causal_binding",
                "callee_signature",
                "signer_binding",
                "host_attestation_base64",
                "authority_binding_kind",
                "authority_binding",
                "ability_binding",
                "usage",
                "subject_ref",
                "descriptor_version",
                "schema_hash_hex",
                "impl_hash_hex",
                "runtime_env",
                "authority_proof",
                "input_hash_hex",
                "output_hash_hex",
                "parent_receipts",
            ]
        )
        _ = try runtimeBase64(
            runtimeRequiredStringAllowEmpty(raw, "payload_base64", "runtime_receipt"),
            "payload_base64",
            expectedLength: nil,
            allowEmpty: true
        )
        _ = try runtimeRequiredString(raw, "payload_content_type", "runtime_receipt")
        _ = try runtimeRequiredStringAllowEmpty(raw, "host_attestation_base64", "runtime_receipt")
        let usage = try runtimeRequiredObject(raw, "usage", "runtime_receipt")
        try runtimeValidateUsage(usage)
        _ = try runtimeReceiptAgentBinding(raw["caller_binding"], "caller_binding")
        let calleeBinding = try runtimeReceiptAgentBinding(raw["callee_binding"], "callee_binding")
        _ = try runtimeReceiptAgentBinding(raw["subject_binding"], "subject_binding")
        _ = try runtimeBase64(
            runtimeRequiredString(raw, "invocation_nonce_base64", "runtime_receipt"),
            "invocation_nonce_base64",
            expectedLength: 16,
            allowEmpty: false
        )
        let causalKind = try runtimeRequiredString(raw, "causal_binding_kind", "runtime_receipt")
        try validateCausalBinding(causalKind, runtimeRequiredObject(raw, "causal_binding", "runtime_receipt"))
        try runtimeReceiptSignature(raw["callee_signature"], "callee_signature", required: true)
        let signerBinding = try runtimeReceiptAgentBinding(raw["signer_binding"], "signer_binding")
        try validateSigningModel(
            calleeBinding: calleeBinding,
            signerBinding: signerBinding,
            hostAttestationBase64: runtimeOptionalString(raw, "host_attestation_base64", "runtime_receipt") ?? ""
        )

        let authorityKind = try runtimeRequiredString(raw, "authority_binding_kind", "runtime_receipt")
        let authorityBinding = try RuntimeAuthorityBinding(raw["authority_binding"], "authority_binding")
        guard authorityBinding.kind == authorityKind else {
            throw SDKError.validation("runtime_receipt", "runtime receipt authority_binding kind does not match authority_binding_kind")
        }
        _ = try runtimeRequiredString(raw, "ability_binding", "runtime_receipt")
        try runtimeReceiptEntityRef(raw["subject_ref"], "subject_ref")
        _ = try runtimeRequiredString(raw, "descriptor_version", "runtime_receipt")
        _ = try runtimeReceiptHash(raw, "schema_hash_hex", allowZero: false)
        _ = try runtimeReceiptHash(raw, "impl_hash_hex", allowZero: false)
        _ = try runtimeRequiredString(raw, "runtime_env", "runtime_receipt")

        let proof = try runtimeRequiredObject(raw, "authority_proof", "runtime_receipt")
        try runtimeRequireExactKeys(
            proof,
            "authority_proof",
            [
                "proof_type",
                "binding_kind",
                "binding",
                "proof_payload_base64",
                "proof_hash_hex",
                "issuer",
                "signature",
                "admission_hook",
            ]
        )
        _ = try runtimeRequiredString(proof, "proof_type", "runtime_receipt")
        let proofBindingKind = try runtimeRequiredString(proof, "binding_kind", "runtime_receipt")
        guard proofBindingKind == authorityKind else {
            throw SDKError.validation("runtime_receipt", "runtime receipt authority_proof binding_kind does not match authority_binding_kind")
        }
        let proofBinding = try RuntimeAuthorityBinding(proof["binding"], "authority_proof.binding")
        guard proofBinding.canonicalBytes == authorityBinding.canonicalBytes else {
            throw SDKError.validation("runtime_receipt", "runtime receipt authority_proof binding does not match authority_binding")
        }
        let proofPayload = try runtimeBase64(
            runtimeRequiredStringAllowEmpty(proof, "proof_payload_base64", "authority_proof"),
            "authority_proof.proof_payload_base64",
            expectedLength: nil,
            allowEmpty: true
        )
        let proofHash = try runtimeReceiptHash(proof, "proof_hash_hex", allowZero: false)
        try validateAuthorityProofHash(proofPayload: proofPayload, proofBinding: proofBinding, proofHash: proofHash)
        let issuer = try runtimeReceiptAgentBinding(proof["issuer"], "authority_proof.issuer")
        guard issuer == calleeBinding else {
            throw SDKError.validation("runtime_receipt", "runtime receipt authority_proof issuer does not match callee_binding")
        }
        try runtimeReceiptSignature(proof["signature"], "authority_proof.signature", required: true)
        _ = try runtimeRequiredString(proof, "admission_hook", "runtime_receipt")

        _ = try runtimeReceiptHash(raw, "input_hash_hex", allowZero: false)
        _ = try runtimeReceiptHash(raw, "output_hash_hex", allowZero: false)
        try runtimeParentReceipts(raw["parent_receipts"])
    }

    private static func validateAuthorityProofHash(
        proofPayload: Data,
        proofBinding: RuntimeAuthorityBinding,
        proofHash: Data
    ) throws {
        let expected = proofPayload.isEmpty
            ? Data(SHA256.hash(data: proofBinding.canonicalBytes))
            : Data(SHA256.hash(data: proofPayload))
        guard !expected.allSatisfy({ $0 == 0 }),
              !proofHash.allSatisfy({ $0 == 0 }),
              proofHash == expected else {
            throw SDKError.validation("runtime_receipt", "runtime receipt proof facts are not canonical: authority_proof_hash_mismatch")
        }
    }

    private static func validateSigningModel(
        calleeBinding: RuntimeAgentBinding,
        signerBinding: RuntimeAgentBinding,
        hostAttestationBase64: String
    ) throws {
        if signerBinding.ura == calleeBinding.ura {
            if !hostAttestationBase64.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                throw SDKError.validation("runtime_receipt", "self-signed runtime receipt must not carry host_attestation_base64")
            }
            return
        }
        guard !hostAttestationBase64.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("runtime_receipt", "hosted runtime receipt is missing host_attestation_base64")
        }
        _ = try runtimeBase64(hostAttestationBase64, "host_attestation_base64", expectedLength: 64, allowEmpty: false)
    }

    private static func validateCausalBinding(_ kind: String, _ binding: [String: Any]) throws {
        let form = try runtimeRequiredString(binding, "form", "runtime_receipt")
        guard form == kind else {
            throw SDKError.validation("runtime_receipt", "runtime receipt causal_binding form does not match causal_binding_kind")
        }
        switch form {
        case "none":
            try runtimeRequireExactKeys(binding, "causal_binding", ["form"])
            return
        case "scalar":
            try runtimeRequireExactKeys(binding, "causal_binding", ["form", "receipt"])
            try runtimeReceiptRef(binding["receipt"], "causal_binding.receipt")
        case "list":
            try runtimeRequireExactKeys(binding, "causal_binding", ["form", "prior"])
            guard let prior = binding["prior"] as? [Any], !prior.isEmpty else {
                throw SDKError.validation("runtime_receipt", "causal_binding.prior must be a non-empty array")
            }
            for (index, receipt) in prior.enumerated() {
                try runtimeReceiptRef(receipt, "causal_binding.prior[\(index)]")
            }
        case "merkle":
            try runtimeRequireExactKeys(binding, "causal_binding", ["form", "root_hex", "proof_ura"])
            _ = try runtimeReceiptHash(binding, "root_hex", allowZero: false)
            _ = try runtimeRequiredString(binding, "proof_ura", "runtime_receipt")
        default:
            throw SDKError.validation("runtime_receipt", "unsupported causal_binding form \(form)")
        }
    }
}

public struct InvocationControlCapability: Sendable {
    private let handleId: Int64
    private let runtimeBound: Bool

    init(handleId: Int64, runtimeBound: Bool = true) throws {
        guard handleId > 0 else {
            throw SDKError.validation("invocation_control", "control capability is required")
        }
        self.handleId = handleId
        self.runtimeBound = runtimeBound
    }

    static func runtimeBound(handleId: Int64) throws -> InvocationControlCapability {
        try InvocationControlCapability(handleId: handleId, runtimeBound: true)
    }

    static func snapshot(handleId: Int64) throws -> InvocationControlCapability {
        try InvocationControlCapability(handleId: handleId, runtimeBound: false)
    }

    func adapterHandleId() throws -> Int64 {
        guard runtimeBound else {
            throw SDKError.validation(
                "invocation_control",
                "runtime-bound invocation control capability is required"
            )
        }
        return handleId
    }

    func rawHandleId() -> Int64 {
        handleId
    }
}

public struct InvocationCancel: Sendable {
    public let controlCapability: InvocationControlCapability
    public let requestAccepted: Bool
    public let deduplicated: Bool
    public let cancelled: Bool
    public let state: String
    public let terminal: Bool

    static func fromJSON(_ raw: Data) throws -> InvocationCancel {
        try fromJSON(raw, expectedControl: nil)
    }

    static func fromJSON(_ raw: Data, expectedControl: InvocationControlCapability?) throws -> InvocationCancel {
        let object = try runtimeJSONObject(raw, "invocation_cancel")
        try runtimeRequireExactProjectionKeys(
            object,
            "invocation cancel",
            [
                "handle_id",
                "request_accepted",
                "deduplicated",
                "cancelled",
                "state",
                "terminal",
            ],
            "invocation_cancel"
        )
        let handleId = try runtimeRequiredInt64(object, "handle_id", "invocation_cancel")
        let control: InvocationControlCapability
        if let expectedControl {
            guard expectedControl.rawHandleId() == handleId else {
                throw SDKError.validation(
                    "invocation_cancel",
                    "handle_id does not match invocation control capability"
                )
            }
            control = expectedControl
        } else {
            control = try InvocationControlCapability.snapshot(handleId: handleId)
        }
        return try InvocationCancel(
            controlCapability: control,
            requestAccepted: try runtimeRequiredBool(object, "request_accepted", "invocation_cancel"),
            deduplicated: try runtimeRequiredBool(object, "deduplicated", "invocation_cancel"),
            cancelled: try runtimeRequiredBool(object, "cancelled", "invocation_cancel"),
            state: runtimeRequiredString(object, "state", "invocation_cancel"),
            terminal: try runtimeRequiredBool(object, "terminal", "invocation_cancel")
        )
    }
}

public protocol RuntimeTransport: AnyObject, Sendable {
    func invoke(_ draft: InvocationDraft) async throws -> InvocationResult
    func resolveDescriptorRef(_ requestJSON: Data) async throws -> Data
    func prepare(_ draftJSON: Data, optionsJSON: Data) async throws -> Data
    func submitSigned(_ signedJSON: Data) async throws -> Data
    func awaitHandle(_ control: InvocationControlCapability) async throws -> Data
    func cancelHandle(_ control: InvocationControlCapability, reason: String) async throws -> Data
    func handleEvents(_ control: InvocationControlCapability) async throws -> Data
    func freeHandle(_ control: InvocationControlCapability) async throws
    func openStream(_ draft: InvocationDraft) async throws -> StreamSource
    func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) async throws -> BidiSource
    func close() async throws
}

public extension RuntimeTransport {
    func resolveDescriptorRef(_ requestJSON: Data) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime descriptor resolver transport is not implemented"
        )
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

    func awaitHandle(_ control: InvocationControlCapability) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime await-handle transport is not implemented"
        )
    }

    func cancelHandle(_ control: InvocationControlCapability, reason: String) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime cancel-handle transport is not implemented"
        )
    }

    func handleEvents(_ control: InvocationControlCapability) async throws -> Data {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime handle-events transport is not implemented"
        )
    }

    func freeHandle(_ control: InvocationControlCapability) async throws {
        throw SDKError(
            code: .notImplemented,
            stage: "runtime",
            retryHint: .never,
            retryable: false,
            message: "runtime free-handle transport is not implemented"
        )
    }

    func close() async throws {}
}

public final class RuntimeClient: @unchecked Sendable {
    private let transport: RuntimeTransport
    private var closed = false

    public init(transport: RuntimeTransport) {
        self.transport = transport
    }

    public func newInvocation() throws -> InvocationBuilder {
        try requireOpen()
        return InvocationBuilder()
    }

    public func invoke(_ draft: InvocationDraft) async throws -> InvocationResult {
        try requireOpen()
        return try await transport.invoke(draft)
    }

    public func resolveDescriptorRef(_ request: RuntimeDescriptorRefRequest) async throws -> String {
        try requireOpen()
        let raw = try await transport.resolveDescriptorRef(try request.jsonData())
        return try RuntimeDescriptorRefResponse.fromJSON(raw)
    }

    public func prepare(_ draft: InvocationDraft, options: [String: Any] = [:]) async throws -> PreparedInvocation {
        try requireOpen()
        let optionsJSON = try JSONSerialization.data(withJSONObject: options, options: [.sortedKeys])
        let raw = try await transport.prepare(try draft.jsonData(), optionsJSON: optionsJSON)
        return try PreparedInvocation.fromJSON(raw).bindRuntime(self)
    }

    public func submitSigned(_ signed: SignedInvocation) async throws -> InvocationHandle {
        try requireOpen()
        let raw = try await transport.submitSigned(try signed.jsonData())
        return try InvocationHandle.fromRuntimeJSON(raw).bindRuntime(self)
    }

    public func submitSigned(_ prepared: PreparedInvocation) async throws -> InvocationHandle {
        try requireOpen()
        _ = prepared
        throw SDKError.validation("runtime", "signed invocation is required")
    }

    public func awaitResult(_ handle: InvocationHandle) async throws -> InvocationResult {
        try requireOpen()
        _ = try handle.controlCapability.adapterHandleId()
        return try InvocationResult.fromJSON(
            try await transport.awaitHandle(handle.controlCapability)
        )
    }

    public func cancel(_ handle: InvocationHandle, reason: String = "") async throws -> InvocationCancel {
        try requireOpen()
        _ = try handle.controlCapability.adapterHandleId()
        return try InvocationCancel.fromJSON(
            try await transport.cancelHandle(handle.controlCapability, reason: reason),
            expectedControl: handle.controlCapability
        )
    }

    public func events(_ handle: InvocationHandle) async throws -> InvocationHandle {
        try requireOpen()
        _ = try handle.controlCapability.adapterHandleId()
        return try InvocationHandle.fromJSON(
            try await transport.handleEvents(handle.controlCapability),
            expectedControl: handle.controlCapability
        ).bindRuntime(self)
    }

    public func closeHandle(_ handle: InvocationHandle) async throws {
        try requireOpen()
        _ = try handle.controlCapability.adapterHandleId()
        try await transport.freeHandle(handle.controlCapability)
    }

    public func openStream(_ draft: InvocationDraft) async throws -> StreamHandle {
        try requireOpen()
        return StreamHandle(source: try await transport.openStream(draft))
    }

    public func openBidi(_ draft: InvocationDraft, frame0: BidiFrame?) async throws -> BidiSession {
        try requireOpen()
        return BidiSession(source: try await transport.openBidi(draft, frame0: try Self.requireBidiFrameZero(frame0)))
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await transport.close()
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("runtime")
        }
    }

    static func requireBidiFrameZero(_ frame0: BidiFrame?) throws -> BidiFrame {
        guard let frame0 else {
            throw SDKError.validation("runtime", "bidi frame0 is required")
        }
        return frame0
    }
}

private func runtimeJSONObject(_ raw: Data, _ label: String) throws -> [String: Any] {
    let value = try JSONSerialization.jsonObject(with: raw)
    guard let object = value as? [String: Any] else {
        throw SDKError.validation(label, "JSON must be an object")
    }
    return object
}

private func runtimeRequiredString(_ object: [String: Any], _ field: String, _ label: String) throws -> String {
    guard let value = object[field] as? String, !value.isEmpty else {
        throw SDKError.validation(label, "\(field) is required")
    }
    return value
}

private func runtimeRequiredBool(_ object: [String: Any], _ field: String, _ label: String) throws -> Bool {
    guard let value = object[field] as? Bool else {
        throw SDKError.validation(label, "\(field) must be a boolean")
    }
    return value
}

private func runtimeRequiredInt64(_ object: [String: Any], _ field: String, _ label: String) throws -> Int64 {
    if let number = object[field] as? NSNumber {
        return number.int64Value
    }
    if let value = object[field] as? Int64 {
        return value
    }
    throw SDKError.validation(label, "\(field) must be an integer")
}

private struct RuntimeTerminalReceiptProjection {
    let summary: [String: String]
    let rawProjection: [String: JSONValue]
}

private func runtimeRequiredTerminalReceipt(
    _ object: [String: Any],
    terminalState: InvocationTerminalState
) throws -> RuntimeTerminalReceiptProjection {
    guard let value = object["terminal_receipt"] else {
        throw SDKError.validation("invocation_result", "terminal_receipt is required")
    }
    guard let map = value as? [String: Any] else {
        throw SDKError.validation("invocation_result", "terminal_receipt must be an object")
    }
    let receipt = try RuntimeReceipt(map)
    guard try receipt.lifecycleState() == runtimeCanonicalTerminalState(terminalState) else {
        throw SDKError.validation("invocation_result", "terminal_receipt state does not match invocation terminal_state")
    }
    return RuntimeTerminalReceiptProjection(
        summary: receipt.projection(),
        rawProjection: receipt.rawProjection()
    )
}

private struct RuntimeAgentBinding: Equatable {
    let ura: String
    let profile: String
}

private struct RuntimeAuthorityBinding {
    let kind: String
    let canonicalBytes: Data

    init(_ value: Any?, _ field: String) throws {
        let object = try runtimeObject(value, field)
        kind = try runtimeRequiredText(object, "kind", "runtime_receipt")
        var bytes = Data()
        switch kind {
        case "self":
            try runtimeRequireExactKeys(object, field, ["kind", "principal_ura"])
            bytes.append(0x01)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "principal_ura", "runtime_receipt"))
        case "delegation":
            try runtimeRequireExactKeys(
                object,
                field,
                [
                    "kind",
                    "issuer_ura",
                    "subject_ura",
                    "caller_ura",
                    "audience",
                    "scopes",
                    "issued_at_ms",
                    "expires_at_ms",
                    "signature_base64",
                ]
            )
            bytes.append(0x02)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "issuer_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "subject_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "caller_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "audience", "runtime_receipt"))
            let scopes = try runtimeStringList(object["scopes"], "\(field).scopes")
            bytes.appendUInt32(UInt32(scopes.count))
            for scope in scopes {
                bytes.appendLengthPrefixed(scope)
            }
            bytes.appendInt64(try runtimeNonNegativeInt64(object["issued_at_ms"], "\(field).issued_at_ms"))
            bytes.appendInt64(try runtimeNonNegativeInt64(object["expires_at_ms"], "\(field).expires_at_ms"))
            bytes.appendLengthPrefixed(
                try runtimeBase64(
                    try runtimeRequiredText(object, "signature_base64", "runtime_receipt"),
                    "\(field).signature_base64",
                    expectedLength: 64,
                    allowEmpty: false
                )
            )
        case "capability":
            try runtimeRequireExactKeys(object, field, ["kind", "capability_ura"])
            bytes.append(0x03)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "capability_ura", "runtime_receipt"))
        case "policy":
            try runtimeRequireExactKeys(object, field, ["kind", "policy_ura"])
            bytes.append(0x04)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "policy_ura", "runtime_receipt"))
        case "session":
            try runtimeRequireExactKeys(
                object,
                field,
                [
                    "kind",
                    "issuer_ura",
                    "subject_ura",
                    "session_id",
                    "scopes",
                    "audiences",
                    "issued_at_ms",
                    "expires_at_ms",
                    "signature_base64",
                ]
            )
            bytes.append(0x05)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "issuer_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "subject_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "session_id", "runtime_receipt"))
            let scopes = try runtimeStringList(object["scopes"], "\(field).scopes")
            bytes.appendUInt32(UInt32(scopes.count))
            for scope in scopes {
                bytes.appendLengthPrefixed(scope)
            }
            let audiences = try runtimeStringList(object["audiences"], "\(field).audiences")
            bytes.appendUInt32(UInt32(audiences.count))
            for audience in audiences {
                bytes.appendLengthPrefixed(audience)
            }
            bytes.appendInt64(try runtimeNonNegativeInt64(object["issued_at_ms"], "\(field).issued_at_ms"))
            bytes.appendInt64(try runtimeNonNegativeInt64(object["expires_at_ms"], "\(field).expires_at_ms"))
            bytes.appendLengthPrefixed(
                try runtimeBase64(
                    try runtimeRequiredText(object, "signature_base64", "runtime_receipt"),
                    "\(field).signature_base64",
                    expectedLength: 64,
                    allowEmpty: false
                )
            )
        case "bootstrap":
            try runtimeRequireExactKeys(object, field, ["kind", "principal_ura", "realm", "ability"])
            bytes.append(0x06)
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "principal_ura", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "realm", "runtime_receipt"))
            bytes.appendLengthPrefixed(try runtimeRequiredText(object, "ability", "runtime_receipt"))
        default:
            throw SDKError.validation("runtime_receipt", "\(field).kind is not canonical: \(kind)")
        }
        canonicalBytes = bytes
    }
}

private func runtimeValidateUsage(_ usage: [String: Any]) throws {
    let fields: Set<String> = ["tokens_in", "tokens_out", "duration_ms", "external_calls"]
    try runtimeRequireExactKeys(usage, "usage", fields)
    try runtimeRequireRequiredKeys(usage, "usage", fields)
    for field in fields {
        _ = try runtimeRequiredUsageCounter(usage[field], "usage.\(field)")
    }
}

private func runtimeRequiredUsageCounter(_ value: Any?, _ field: String) throws -> Int64 {
    if let number = value as? NSNumber {
        if String(cString: number.objCType) == "c" {
            throw SDKError.validation("runtime_receipt", "\(field) must be a non-negative integer")
        }
        return try runtimeNonNegativeInt64(number, field)
    }
    if value is Bool {
        throw SDKError.validation("runtime_receipt", "\(field) must be a non-negative integer")
    }
    return try runtimeNonNegativeInt64(value, field)
}

private func runtimeRequiredObject(_ object: [String: Any], _ field: String, _ label: String) throws -> [String: Any] {
    try runtimeObject(object[field], "\(label).\(field)")
}

private func runtimeObject(_ value: Any?, _ field: String) throws -> [String: Any] {
    guard let object = value as? [String: Any] else {
        throw SDKError.validation("runtime_receipt", "\(field) must be an object")
    }
    return object
}

private func runtimeRequiredStringAllowEmpty(
    _ object: [String: Any],
    _ field: String,
    _ objectName: String
) throws -> String {
    guard let value = object[field] as? String, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        if objectName == "authority_proof", field == "proof_payload_base64" {
            throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing authority_proof.proof_payload_base64")
        }
        throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing \(objectName).\(field)")
    }
    return value
}

private func runtimeRequireExactKeys(
    _ object: [String: Any],
    _ field: String,
    _ allowedKeys: Set<String>
) throws {
    for key in object.keys.sorted() where !allowedKeys.contains(key) {
        throw SDKError.validation("runtime_receipt", "\(field) contains noncanonical field \(key)")
    }
}

private func runtimeRequireExactProjectionKeys(
    _ object: [String: Any],
    _ field: String,
    _ allowedKeys: Set<String>,
    _ stage: String
) throws {
    for key in object.keys.sorted() where !allowedKeys.contains(key) {
        throw SDKError.validation(stage, "\(field) contains noncanonical field \(key)")
    }
}

private func runtimeRequireRequiredKeys(
    _ object: [String: Any],
    _ field: String,
    _ requiredKeys: Set<String>
) throws {
    for key in requiredKeys where !object.keys.contains(key) {
        throw SDKError.validation("runtime_receipt", "runtime receipt summary is missing \(field).\(key)")
    }
}

private func runtimeOptionalString(_ object: [String: Any], _ field: String, _ label: String) throws -> String? {
    guard let value = object[field], !(value is NSNull) else {
        return nil
    }
    guard let string = value as? String else {
        throw SDKError.validation(label, "\(field) must be a string")
    }
    return string
}

private func runtimeRequiredText(_ object: [String: Any], _ field: String, _ label: String) throws -> String {
    let value = try runtimeRequiredString(object, field, label)
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
        throw SDKError.validation(label, "\(field) is required")
    }
    return trimmed
}

private func runtimeStringList(_ value: Any?, _ field: String) throws -> [String] {
    guard let raw = value as? [Any], !raw.isEmpty else {
        throw SDKError.validation("runtime_receipt", "\(field) must be a non-empty array")
    }
    return try raw.enumerated().map { index, item in
        guard let string = item as? String else {
            throw SDKError.validation("runtime_receipt", "\(field)[\(index)] must be a non-empty string")
        }
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw SDKError.validation("runtime_receipt", "\(field)[\(index)] must be a non-empty string")
        }
        return trimmed
    }
}

private func runtimeNonNegativeInt64(_ value: Any?, _ field: String) throws -> Int64 {
    let integer: Int64
    if let number = value as? NSNumber {
        integer = number.int64Value
    } else if let int = value as? Int {
        integer = Int64(int)
    } else if let int64 = value as? Int64 {
        integer = int64
    } else {
        throw SDKError.validation("runtime_receipt", "\(field) must be a non-negative integer")
    }
    guard integer >= 0 else {
        throw SDKError.validation("runtime_receipt", "\(field) must be a non-negative integer")
    }
    return integer
}

private func runtimeReceiptAgentBinding(_ value: Any?, _ field: String) throws -> RuntimeAgentBinding {
    let object = try runtimeObject(value, field)
    try runtimeRequireExactKeys(object, field, ["ura", "profile"])
    let binding = RuntimeAgentBinding(
        ura: try runtimeRequiredText(object, "ura", "runtime_receipt"),
        profile: try runtimeRequiredText(object, "profile", "runtime_receipt")
    )
    try runtimeValidateURAProfile(binding.profile, "\(field).profile")
    return binding
}

private func runtimeReceiptEntityRef(_ value: Any?, _ field: String) throws {
    let object = try runtimeObject(value, field)
    try runtimeRequireExactKeys(object, field, ["kind", "ura", "profile"])
    let kind = try runtimeNonNegativeInt64(object["kind"], "\(field).kind")
    guard (1...7).contains(kind) else {
        throw SDKError.validation("runtime_receipt", "\(field).kind is not canonical")
    }
    _ = try runtimeRequiredText(object, "ura", "runtime_receipt")
    try runtimeValidateURAProfile(try runtimeRequiredText(object, "profile", "runtime_receipt"), "\(field).profile")
}

private func runtimeReceiptSignature(_ value: Any?, _ field: String, required: Bool) throws {
    guard let value, !(value is NSNull) else {
        if required {
            throw SDKError.validation("runtime_receipt", "\(field) must be an object")
        }
        return
    }
    let object = try runtimeObject(value, field)
    try runtimeRequireExactKeys(object, field, ["algorithm", "signature_base64", "key_id_hint"])
    _ = try runtimeRequiredText(object, "algorithm", "runtime_receipt")
    _ = try runtimeBase64(
        try runtimeRequiredText(object, "signature_base64", "runtime_receipt"),
        "\(field).signature_base64",
        expectedLength: nil,
        allowEmpty: false
    )
    _ = try runtimeOptionalString(object, "key_id_hint", "runtime_receipt")
}

private func runtimeReceiptRef(_ value: Any?, _ field: String) throws {
    let object = try runtimeObject(value, field)
    try runtimeRequireExactKeys(object, field, ["receipt_hash_hex", "receipt_ura"])
    _ = try runtimeReceiptHash(object, "receipt_hash_hex", allowZero: false)
    _ = try runtimeRequiredText(object, "receipt_ura", "runtime_receipt")
}

private func runtimeParentReceipts(_ value: Any?) throws {
    guard let parents = value as? [Any] else {
        throw SDKError.validation("runtime_receipt", "parent_receipts must be an array")
    }
    for (index, parent) in parents.enumerated() {
        try runtimeReceiptRef(parent, "parent_receipts[\(index)]")
    }
}

private func runtimeReceiptHash(_ object: [String: Any], _ field: String, allowZero: Bool) throws -> Data {
    let value = try runtimeRequiredText(object, field, "runtime_receipt")
    guard value.count == 64, value.allSatisfy({ $0.isHexDigit }) else {
        throw SDKError.validation("runtime_receipt", "\(field) must be exactly 32 bytes hex")
    }
    var bytes = Data()
    var index = value.startIndex
    while index < value.endIndex {
        let next = value.index(index, offsetBy: 2)
        guard let byte = UInt8(value[index..<next], radix: 16) else {
            throw SDKError.validation("runtime_receipt", "\(field) must be hexadecimal")
        }
        bytes.append(byte)
        index = next
    }
    if !allowZero, bytes.allSatisfy({ $0 == 0 }) {
        throw SDKError.validation("runtime_receipt", "\(field) must not be all-zero")
    }
    return bytes
}

private func runtimeBase64(
    _ value: String,
    _ field: String,
    expectedLength: Int?,
    allowEmpty: Bool
) throws -> Data {
    let data = try canonicalBase64Data(value, stage: "runtime_receipt", field: field)
    if data.isEmpty, !allowEmpty {
        throw SDKError.validation("runtime_receipt", "\(field) must decode to non-empty bytes")
    }
    if let expectedLength, data.count != expectedLength {
        throw SDKError.validation("runtime_receipt", "\(field) must decode to exactly \(expectedLength) bytes")
    }
    return data
}

private func runtimeValidateURAProfile(_ profile: String, _ field: String) throws {
    guard profile == "axon-strict-v2" else {
        throw SDKError.validation("runtime_receipt", "\(field) is not canonical")
    }
}

private func runtimeCanonicalTerminalState(_ terminalState: InvocationTerminalState) -> String {
    switch terminalState {
    case .completed: return "COMPLETED"
    case .failed: return "FAILED"
    case .cancelled: return "CANCELLED"
    case .timedOut: return "TIMED_OUT"
    }
}

private extension Data {
    mutating func appendLengthPrefixed(_ string: String) {
        appendLengthPrefixed(Data(string.utf8))
    }

    mutating func appendLengthPrefixed(_ data: Data) {
        appendUInt32(UInt32(data.count))
        append(data)
    }

    mutating func appendUInt32(_ value: UInt32) {
        var bigEndian = value.bigEndian
        Swift.withUnsafeBytes(of: &bigEndian) { append(contentsOf: $0) }
    }

    mutating func appendInt64(_ value: Int64) {
        var bigEndian = UInt64(value).bigEndian
        Swift.withUnsafeBytes(of: &bigEndian) { append(contentsOf: $0) }
    }
}

private func runtimeOptionalJSONObjectString(_ value: Any?) throws -> String {
    guard let value else {
        return ""
    }
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    return String(data: data, encoding: .utf8) ?? ""
}
