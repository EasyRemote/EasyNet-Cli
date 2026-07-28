import Foundation

public struct InvocationTuple: Sendable, Equatable {
    public let caller: String
    public let callee: String
    public let descriptorRef: String
    public let subject: String
    public let nonce: String
    public let causalContext: String
    public let argsJSON: String
    public let metadata: [String: JSONValue]

    public init(
        caller: String?,
        callee: String?,
        descriptorRef: String?,
        subject: String?,
        nonce: String?,
        causalContext: String?,
        argsJSON: String?,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.caller = try InvocationTuple.requiredPrincipal(caller, "caller")
        self.callee = try InvocationTuple.requiredPrincipal(callee, "callee")
        self.descriptorRef = try InvocationTuple.required(descriptorRef, "descriptorRef")
        self.subject = try InvocationTuple.requiredPrincipal(subject, "subject")
        self.nonce = try InvocationTuple.required(nonce, "nonce")
        self.causalContext = try InvocationTuple.required(causalContext, "causalContext")
        self.argsJSON = try InvocationTuple.required(argsJSON, "argsJSON")
        self.metadata = metadata
    }

    private static func required(_ value: String?, _ field: String) throws -> String {
        guard let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("invocation", "\(field) is required")
        }
        return value
    }

    private static func requiredPrincipal(_ value: String?, _ field: String) throws -> String {
        let cleaned = try required(value, field)
        if RuntimePrincipals.containsAllZeroPrincipal(cleaned) {
            throw SDKError.validation("invocation", "\(field) must not be all-zero")
        }
        return cleaned
    }

    func wireObject() throws -> [String: Any] {
        [
            "caller_ura": caller,
            "callee_ura": callee,
            "descriptor_ref": descriptorRef,
            "subject_ura": subject,
            "nonce_base64": nonce,
            "causal_context": try decodeJSONValue(causalContext, "causal_context"),
            "args": try decodeJSONValue(argsJSON, "args"),
            "content_type": "application/json",
            "metadata": try metadata.mapValues(jsonPlainValue)
        ]
    }

    static func fromWireObject(_ object: [String: Any]) throws -> InvocationTuple {
        try InvocationTuple(
            caller: requiredString(object, "caller_ura", "invocation"),
            callee: requiredString(object, "callee_ura", "invocation"),
            descriptorRef: requiredString(object, "descriptor_ref", "invocation"),
            subject: requiredString(object, "subject_ura", "invocation"),
            nonce: requiredString(object, "nonce_base64", "invocation"),
            causalContext: encodeJSONString(requiredValue(object, "causal_context", "invocation")),
            argsJSON: encodeJSONString(requiredValue(object, "args", "invocation")),
            metadata: try invocationMetadataObject(object["metadata"])
        )
    }
}

public struct InvocationDraft: Sendable, Equatable {
    public let tuple: InvocationTuple

    public init(tuple: InvocationTuple) {
        self.tuple = tuple
    }

    public func inspectTuple() -> InvocationTuple {
        tuple
    }

    func jsonData() throws -> Data {
        try encodeJSONData(tuple.wireObject())
    }

    static func fromWireObject(_ object: [String: Any]) throws -> InvocationDraft {
        InvocationDraft(tuple: try InvocationTuple.fromWireObject(object))
    }
}

public struct SigningMaterial: Sendable, Equatable {
    public let algorithm: String
    public let canonicalBytesBase64: String
    public let argsDigestHex: String
    public let descriptorRef: String
    public let expiresAtUnixMS: Int64
    public let signerPolicy: SignerPolicy?

    public init(
        algorithm: String,
        canonicalBytesBase64: String,
        argsDigestHex: String,
        descriptorRef: String,
        expiresAtUnixMS: Int64,
        signerPolicy: SignerPolicy? = nil
    ) throws {
        self.algorithm = try requiredNonEmpty(algorithm, "algorithm", "signing_material")
        self.canonicalBytesBase64 = try requiredNonEmpty(canonicalBytesBase64, "canonical_bytes_base64", "signing_material")
        self.argsDigestHex = try requiredNonEmpty(argsDigestHex, "args_digest_hex", "signing_material")
        self.descriptorRef = try requiredNonEmpty(descriptorRef, "descriptor_ref", "signing_material")
        guard expiresAtUnixMS >= 0 else {
            throw SDKError.validation("signing_material", "expires_at_unix_ms must be non-negative")
        }
        self.expiresAtUnixMS = expiresAtUnixMS
        self.signerPolicy = signerPolicy
    }

    static func fromObject(_ object: [String: Any]) throws -> SigningMaterial {
        try rejectUnknownFields(
            object,
            allowed: ["algorithm", "canonical_bytes_base64", "args_digest_hex", "descriptor_ref", "expires_at_unix_ms", "signer_policy"],
            label: "signing_material"
        )
        return try SigningMaterial(
            algorithm: requiredString(object, "algorithm", "signing_material"),
            canonicalBytesBase64: requiredString(object, "canonical_bytes_base64", "signing_material"),
            argsDigestHex: requiredString(object, "args_digest_hex", "signing_material"),
            descriptorRef: requiredString(object, "descriptor_ref", "signing_material"),
            expiresAtUnixMS: requiredInt64(object, "expires_at_unix_ms", "signing_material"),
            signerPolicy: optionalObject(object, "signer_policy", "signing_material").map { try SignerPolicy.fromObject($0) }
        )
    }

    func object() -> [String: Any] {
        var value: [String: Any] = [
            "algorithm": algorithm,
            "canonical_bytes_base64": canonicalBytesBase64,
            "args_digest_hex": argsDigestHex,
            "descriptor_ref": descriptorRef,
            "expires_at_unix_ms": expiresAtUnixMS
        ]
        if let signerPolicy {
            value["signer_policy"] = signerPolicy.object()
        }
        return value
    }
}

public struct SignerPolicy: Sendable, Equatable {
    public let mode: String
    public let signerId: String
    public let policyRef: String
    public let expiresAtUnixMS: Int64

    public init(
        mode: String = "",
        signerId: String = "",
        policyRef: String = "",
        expiresAtUnixMS: Int64 = 0
    ) throws {
        guard expiresAtUnixMS >= 0 else {
            throw SDKError.validation("signer_policy", "expires_at_unix_ms must be non-negative")
        }
        if mode.trimmingCharacters(in: .whitespacesAndNewlines) == "provider_managed_signing" {
            if signerId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                throw SDKError.validation("signer_policy", "provider-managed signer_policy requires signer_id")
            }
            if policyRef.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                throw SDKError.validation("signer_policy", "provider-managed signer_policy requires policy_ref")
            }
        }
        self.mode = mode
        self.signerId = signerId
        self.policyRef = policyRef
        self.expiresAtUnixMS = expiresAtUnixMS
    }

    static func fromObject(_ object: [String: Any]) throws -> SignerPolicy {
        try rejectUnknownFields(
            object,
            allowed: ["mode", "signer_id", "policy_ref", "expires_at_unix_ms"],
            label: "signer_policy"
        )
        return try SignerPolicy(
            mode: optionalString(object, "mode", "signer_policy") ?? "",
            signerId: optionalString(object, "signer_id", "signer_policy") ?? "",
            policyRef: optionalString(object, "policy_ref", "signer_policy") ?? "",
            expiresAtUnixMS: optionalInt64(object, "expires_at_unix_ms", "signer_policy") ?? 0
        )
    }

    func object() -> [String: Any] {
        [
            "mode": mode,
            "signer_id": signerId,
            "policy_ref": policyRef,
            "expires_at_unix_ms": expiresAtUnixMS
        ]
    }
}

public struct InvocationSignature: Sendable, Equatable {
    public let algorithm: String
    public let signatureBase64: String
    public let keyIdHint: String
    public let signerPublicKeyBase64: String

    public init(
        algorithm: String,
        signatureBase64: String,
        keyIdHint: String = "",
        signerPublicKeyBase64: String = ""
    ) throws {
        self.algorithm = try requiredNonEmpty(algorithm, "algorithm", "signature")
        self.signatureBase64 = try requiredNonEmpty(signatureBase64, "signature_base64", "signature")
        self.keyIdHint = keyIdHint
        self.signerPublicKeyBase64 = signerPublicKeyBase64
    }

    static func fromObject(_ object: [String: Any]) throws -> InvocationSignature {
        try rejectUnknownFields(
            object,
            allowed: ["algorithm", "signature_base64", "key_id_hint", "signer_public_key_base64"],
            label: "signature"
        )
        return try InvocationSignature(
            algorithm: requiredString(object, "algorithm", "signature"),
            signatureBase64: requiredString(object, "signature_base64", "signature"),
            keyIdHint: optionalString(object, "key_id_hint", "signature") ?? "",
            signerPublicKeyBase64: optionalString(object, "signer_public_key_base64", "signature") ?? ""
        )
    }

    func object() -> [String: Any] {
        [
            "algorithm": algorithm,
            "signature_base64": signatureBase64,
            "key_id_hint": keyIdHint,
            "signer_public_key_base64": signerPublicKeyBase64
        ]
    }
}

public final class PreparedInvocation: @unchecked Sendable {
    public let preparedId: String
    public let requestId: String
    private let draft: InvocationDraft
    public let signingMaterial: SigningMaterial
    public let descriptorRef: String
    public let descriptorHashHex: String
    public let schemaHashHex: String
    public let canonicalHashHex: String
    public let expiresAtUnixMS: Int64
    private weak var runtime: RuntimeClient?

    public init(
        preparedId: String = "",
        requestId: String = "",
        draft: InvocationDraft,
        signingMaterial: SigningMaterial,
        descriptorRef: String? = nil,
        descriptorHashHex: String = "",
        schemaHashHex: String = "",
        canonicalHashHex: String = "",
        expiresAtUnixMS: Int64? = nil,
        submitReady: Bool = false
    ) throws {
        guard !submitReady else {
            throw SDKError.validation("prepared_invocation", "PreparedInvocation must not be submit-ready")
        }
        guard !preparedId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("prepared_invocation", "prepared_id is required")
        }
        guard let boundDescriptor = descriptorRef,
              !boundDescriptor.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("prepared_invocation", "descriptor_ref is required")
        }
        guard boundDescriptor == signingMaterial.descriptorRef,
              draft.inspectTuple().descriptorRef == signingMaterial.descriptorRef else {
            throw SDKError.validation(
                "prepared_invocation",
                "signing_material.descriptor_ref must match tuple descriptor_ref"
            )
        }
        self.preparedId = preparedId
        self.requestId = requestId
        self.draft = draft
        self.signingMaterial = signingMaterial
        self.descriptorRef = boundDescriptor
        self.descriptorHashHex = descriptorHashHex
        self.schemaHashHex = schemaHashHex
        self.canonicalHashHex = canonicalHashHex
        self.expiresAtUnixMS = expiresAtUnixMS ?? signingMaterial.expiresAtUnixMS
    }

    public static func fromJSON(_ raw: Data) throws -> PreparedInvocation {
        try fromObject(decodeJSONObject(raw, "prepared invocation"))
    }

    static func fromObject(_ object: [String: Any]) throws -> PreparedInvocation {
        try rejectUnknownFields(
            object,
            allowed: [
                "prepared_id", "request_id", "tuple", "signing_material", "descriptor_ref",
                "descriptor_hash_hex", "schema_hash_hex", "canonical_hash_hex",
                "expires_at_unix_ms", "submit_ready"
            ],
            label: "prepared_invocation"
        )
        if let ready = object["submit_ready"], !(ready is NSNull) {
            guard let bool = ready as? Bool, bool == false else {
                throw SDKError.validation("prepared_invocation", "PreparedInvocation must not be submit-ready")
            }
        }
        let material = try SigningMaterial.fromObject(requiredObject(object, "signing_material", "prepared_invocation"))
        return try PreparedInvocation(
            preparedId: optionalString(object, "prepared_id", "prepared_invocation") ?? "",
            requestId: optionalString(object, "request_id", "prepared_invocation") ?? "",
            draft: InvocationDraft.fromWireObject(requiredObject(object, "tuple", "prepared_invocation")),
            signingMaterial: material,
            descriptorRef: requiredString(object, "descriptor_ref", "prepared_invocation"),
            descriptorHashHex: optionalString(object, "descriptor_hash_hex", "prepared_invocation") ?? "",
            schemaHashHex: optionalString(object, "schema_hash_hex", "prepared_invocation") ?? "",
            canonicalHashHex: optionalString(object, "canonical_hash_hex", "prepared_invocation") ?? "",
            expiresAtUnixMS: optionalInt64(object, "expires_at_unix_ms", "prepared_invocation"),
            submitReady: false
        )
    }

    @discardableResult
    func bindRuntime(_ runtime: RuntimeClient) -> PreparedInvocation {
        self.runtime = runtime
        return self
    }

    public func tuple() -> InvocationTuple {
        draft.inspectTuple()
    }

    public func submitReady() -> Bool {
        false
    }

    public func signWithCallerSignature(_ signature: InvocationSignature) throws -> SignedInvocation {
        var signerId = signature.keyIdHint.isEmpty ? signature.signerPublicKeyBase64 : signature.keyIdHint
        if let policySigner = signingMaterial.signerPolicy?.signerId,
           !policySigner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            signerId = policySigner
        }
        guard !signerId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("prepared_invocation", "signer id is required")
        }
        return try SignedInvocation(
            prepared: self,
            signature: signature,
            signerId: signerId,
            policy: signingMaterial.signerPolicy
        ).bindRuntime(runtime)
    }

    func object() throws -> [String: Any] {
        [
            "prepared_id": preparedId,
            "request_id": requestId,
            "tuple": try tuple().wireObject(),
            "signing_material": signingMaterial.object(),
            "descriptor_ref": descriptorRef,
            "descriptor_hash_hex": descriptorHashHex,
            "schema_hash_hex": schemaHashHex,
            "canonical_hash_hex": canonicalHashHex,
            "expires_at_unix_ms": expiresAtUnixMS,
            "submit_ready": false
        ]
    }
}

public final class SignedInvocation: @unchecked Sendable {
    public let prepared: PreparedInvocation
    public let signature: InvocationSignature
    public let signerId: String
    public let policy: SignerPolicy?
    private weak var runtime: RuntimeClient?

    public init(
        prepared: PreparedInvocation,
        signature: InvocationSignature,
        signerId: String,
        policy: SignerPolicy? = nil
    ) throws {
        self.prepared = prepared
        self.signature = signature
        self.signerId = try requiredNonEmpty(signerId, "signer_id", "signed_invocation")
        self.policy = policy
        guard submitReady() else {
            throw SDKError.validation("signed_invocation", "signed invocation is not submit-ready")
        }
    }

    @discardableResult
    func bindRuntime(_ runtime: RuntimeClient?) -> SignedInvocation {
        self.runtime = runtime
        return self
    }

    public func submitReady() -> Bool {
        !signerId.isEmpty &&
            !signature.algorithm.isEmpty &&
            !signature.signatureBase64.isEmpty &&
            !prepared.descriptorRef.isEmpty &&
            !prepared.signingMaterial.canonicalBytesBase64.isEmpty
    }

    public func submit() async throws -> InvocationHandle {
        guard let runtime else {
            throw SDKError.validation("signed_invocation", "runtime binding is required")
        }
        return try await runtime.submitSigned(self)
    }

    func jsonData() throws -> Data {
        try encodeJSONData(object())
    }

    func object() throws -> [String: Any] {
        var value: [String: Any] = [
            "signer_id": signerId,
            "prepared": [
                "prepared_id": prepared.preparedId,
                "request_id": prepared.requestId,
                "descriptor_ref": prepared.descriptorRef,
                "canonical_hash_hex": prepared.canonicalHashHex,
                "expires_at_unix_ms": prepared.expiresAtUnixMS,
                "canonical_bytes_base64": prepared.signingMaterial.canonicalBytesBase64,
                "tuple": try prepared.tuple().wireObject()
            ],
            "signature": signature.object()
        ]
        if let policy {
            value["policy"] = policy.object()
        }
        return value
    }
}

public final class InvocationHandle: @unchecked Sendable {
    public private(set) var controlCapability: InvocationControlCapability
    public let state: String
    public let terminal: Bool
    private weak var runtime: RuntimeClient?

    init(handleId: Int64, state: String, terminal: Bool) throws {
        self.controlCapability = try InvocationControlCapability.runtimeBound(handleId: handleId)
        self.state = try requiredNonEmpty(state, "state", "invocation_handle")
        self.terminal = terminal
    }

    public static func fromJSON(_ raw: Data) throws -> InvocationHandle {
        try fromJSON(raw, expectedControl: nil, runtimeBound: false)
    }

    static func fromRuntimeJSON(_ raw: Data) throws -> InvocationHandle {
        try fromJSON(raw, expectedControl: nil, runtimeBound: true)
    }

    static func fromJSON(_ raw: Data, expectedControl: InvocationControlCapability) throws -> InvocationHandle {
        try fromJSON(raw, expectedControl: expectedControl, runtimeBound: false)
    }

    private static func fromJSON(
        _ raw: Data,
        expectedControl: InvocationControlCapability?,
        runtimeBound: Bool
    ) throws -> InvocationHandle {
        let object = try decodeJSONObject(raw, "invocation handle")
        try rejectUnknownFields(
            object,
            allowed: ["handle_id", "state", "terminal", "events", "result"],
            label: "invocation_handle"
        )
        let handleId = try requiredInt64(object, "handle_id", "invocation_handle")
        let handle = try InvocationHandle(
            handleId: handleId,
            state: requiredString(object, "state", "invocation_handle"),
            terminal: requiredBool(object, "terminal", "invocation_handle")
        )
        if let expectedControl {
            guard expectedControl.rawHandleId() == handleId else {
                throw SDKError.validation(
                    "invocation_handle",
                    "handle_id does not match invocation control capability"
                )
            }
            handle.controlCapability = expectedControl
        } else if !runtimeBound {
            handle.controlCapability = try InvocationControlCapability.snapshot(handleId: handleId)
        }
        return handle
    }

    @discardableResult
    func bindRuntime(_ runtime: RuntimeClient) -> InvocationHandle {
        self.runtime = runtime
        return self
    }
}

public final class InvocationBuilder {
    private var caller: String?
    private var callee: String?
    private var descriptorRef: String?
    private var subject: String?
    private var nonce: String?
    private var causalContext: String?
    private var argsJSON: String?
    private var metadata: [String: JSONValue] = [:]
    private var allowsRuntimeGovernanceRead = false

    public init() {}

    @discardableResult
    public func withCallerURA(_ value: String) -> InvocationBuilder {
        caller = value
        return self
    }

    @discardableResult
    public func withCalleeURA(_ value: String) -> InvocationBuilder {
        callee = value
        return self
    }

    @discardableResult
    public func withDescriptorRef(_ value: String) -> InvocationBuilder {
        descriptorRef = value
        return self
    }

    @discardableResult
    public func withSubjectURA(_ value: String) -> InvocationBuilder {
        subject = value
        return self
    }

    @discardableResult
    public func withNonce(_ value: String) -> InvocationBuilder {
        nonce = value
        return self
    }

    @discardableResult
    public func withCausalContext(_ value: String) -> InvocationBuilder {
        causalContext = value
        return self
    }

    @discardableResult
    public func withArgsJSON(_ value: String) -> InvocationBuilder {
        argsJSON = value
        return self
    }

    @discardableResult
    public func withMetadata(_ value: [String: JSONValue]) -> InvocationBuilder {
        metadata = value
        return self
    }

    @discardableResult
    public func withAuthorityMetadata(_ value: AuthorityMetadata) throws -> InvocationBuilder {
        metadata = try value.mergeInto(metadata)
        return self
    }

    @discardableResult
    func runtimeGovernanceRead() -> InvocationBuilder {
        allowsRuntimeGovernanceRead = true
        return self
    }

    public func inspect() throws -> InvocationDraft {
        let tuple = try InvocationTuple(
            caller: caller,
            callee: callee,
            descriptorRef: descriptorRef,
            subject: subject,
            nonce: nonce,
            causalContext: causalContext,
            argsJSON: argsJSON,
            metadata: metadata
        )
        if !allowsRuntimeGovernanceRead {
            try rejectGovernanceReadPublicInvocation(tuple)
        }
        try validateInvocationAuthorityBinding(tuple)
        return InvocationDraft(tuple: tuple)
    }

    public func build() throws -> InvocationDraft {
        try inspect()
    }
}

private func rejectGovernanceReadPublicInvocation(_ tuple: InvocationTuple) throws {
    guard let governanceAbility = try RuntimeAbilityProjection.runtimeGovernanceReadAbility(
        calleeURA: tuple.callee,
        descriptorRef: tuple.descriptorRef
    ) else {
        return
    }
    throw SDKError.validation(
        "invocation",
        "runtime governance read ability `\(governanceAbility)` is not a public invocation action; use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider as the canonical runtime provider path"
    )
}

private func decodeJSONValue(_ raw: String, _ field: String) throws -> Any {
    let data = Data(raw.utf8)
    do {
        return try JSONSerialization.jsonObject(with: data, options: [])
    } catch {
        throw SDKError.validation("invocation", "\(field) must be valid JSON")
    }
}

private func decodeJSONObject(_ raw: Data, _ label: String) throws -> [String: Any] {
    do {
        guard let object = try JSONSerialization.jsonObject(with: raw, options: []) as? [String: Any] else {
            throw SDKError.validation(label, "must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError.validation(label, "must be valid JSON")
    }
}

private func encodeJSONData(_ object: Any) throws -> Data {
    do {
        return try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    } catch {
        throw SDKError.validation("json", "value must be encodable")
    }
}

private func encodeJSONString(_ object: Any) throws -> String {
    String(data: try encodeJSONData(object), encoding: .utf8) ?? ""
}

private func rejectUnknownFields(_ object: [String: Any], allowed: Set<String>, label: String) throws {
    for key in object.keys where !allowed.contains(key) {
        throw SDKError.validation(label, "\(key) is not supported")
    }
}

private func requiredValue(_ object: [String: Any], _ field: String, _ label: String) throws -> Any {
    guard let value = object[field], !(value is NSNull) else {
        throw SDKError.validation(label, "\(field) is required")
    }
    return value
}

private func requiredObject(_ object: [String: Any], _ field: String, _ label: String) throws -> [String: Any] {
    guard let value = try requiredValue(object, field, label) as? [String: Any] else {
        throw SDKError.validation(label, "\(field) must be an object")
    }
    return value
}

private func optionalObject(_ object: [String: Any], _ field: String, _ label: String) throws -> [String: Any]? {
    guard let value = object[field], !(value is NSNull) else {
        return nil
    }
    guard let object = value as? [String: Any] else {
        throw SDKError.validation(label, "\(field) must be an object")
    }
    return object
}

private func requiredString(_ object: [String: Any], _ field: String, _ label: String) throws -> String {
    guard let value = try requiredValue(object, field, label) as? String,
          !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        throw SDKError.validation(label, "\(field) is required")
    }
    return value
}

private func optionalString(_ object: [String: Any], _ field: String, _ label: String) throws -> String? {
    guard let value = object[field], !(value is NSNull) else {
        return nil
    }
    guard let string = value as? String else {
        throw SDKError.validation(label, "\(field) must be a string")
    }
    return string
}

private func requiredInt64(_ object: [String: Any], _ field: String, _ label: String) throws -> Int64 {
    guard let number = try requiredValue(object, field, label) as? NSNumber else {
        throw SDKError.validation(label, "\(field) must be an integer")
    }
    return number.int64Value
}

private func optionalInt64(_ object: [String: Any], _ field: String, _ label: String) throws -> Int64? {
    guard let value = object[field], !(value is NSNull) else {
        return nil
    }
    guard let number = value as? NSNumber else {
        throw SDKError.validation(label, "\(field) must be an integer")
    }
    return number.int64Value
}

private func requiredBool(_ object: [String: Any], _ field: String, _ label: String) throws -> Bool {
    guard let bool = try requiredValue(object, field, label) as? Bool else {
        throw SDKError.validation(label, "\(field) must be a boolean")
    }
    return bool
}

private func requiredNonEmpty(_ value: String, _ field: String, _ label: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        throw SDKError.validation(label, "\(field) is required")
    }
    return value
}
