import Foundation

public struct RuntimeDescriptorRefRequest: Sendable, Equatable {
    static let abilityDescriptorProvider = "ability_descriptor"
    static let receiptHistoryProvider = "receipt_history"

    public let calleeURA: String
    public let ability: String
    public let callMode: String
    public let callerURA: String
    public let subjectURA: String
    public let provider: String

    public init(
        calleeURA: String,
        ability: String,
        callMode: String,
        callerURA: String = "",
        subjectURA: String = "",
        provider: String = ""
    ) throws {
        self.calleeURA = try Self.required(calleeURA, "callee_ura")
        self.ability = try Self.required(ability, "ability")
        self.callMode = try Self.required(callMode, "call_mode")
        self.callerURA = try Self.optionalPrincipal(callerURA, "caller_ura")
        self.subjectURA = try Self.optionalPrincipal(subjectURA, "subject_ura")
        self.provider = provider.trimmingCharacters(in: .whitespacesAndNewlines)
        try Self.validateProvider(ability: self.ability, provider: self.provider)
        try Self.validateProviderSubject(
            calleeURA: self.calleeURA,
            callerURA: self.callerURA,
            subjectURA: self.subjectURA,
            provider: self.provider
        )
    }

    func jsonData() throws -> Data {
        var object: [String: Any] = [
            "callee_ura": calleeURA,
            "ability": ability,
            "call_mode": callMode,
        ]
        if !callerURA.isEmpty {
            object["caller_ura"] = callerURA
        }
        if !subjectURA.isEmpty {
            object["subject_ura"] = subjectURA
        }
        if !provider.isEmpty {
            object["provider"] = provider
        }
        return try runtimeDescriptorJSONData(object, stage: "runtime")
    }

    private static func validateProvider(ability: String, provider: String) throws {
        let expected = RuntimeAbilityProjection.runtimeGovernanceDescriptorProvider(forAbility: ability)
        if expected.isEmpty {
            if !provider.isEmpty {
                throw SDKError.validation(
                    "runtime",
                    "descriptor_ref provider \(provider) cannot resolve non-governance ability \(ability)"
                )
            }
            return
        }
        if provider.isEmpty {
            throw SDKError.validation(
                "runtime",
                "descriptor_ref provider request for ability \(ability) requires provider \(expected)"
            )
        }
        if provider != expected {
            throw SDKError.validation(
                "runtime",
                "descriptor_ref provider \(provider) cannot resolve ability \(ability); use provider \(expected)"
            )
        }
    }

    private static func validateProviderSubject(
        calleeURA: String,
        callerURA: String,
        subjectURA: String,
        provider: String
    ) throws {
        guard !provider.isEmpty else {
            return
        }
        if callerURA.isEmpty || subjectURA.isEmpty {
            throw SDKError.validation(
                "runtime",
                "descriptor_ref provider requests require caller_ura and subject_ura"
            )
        }
        if !RuntimeSubjects.isRuntimeGovernanceReadSubject(subjectURA, calleeURA: calleeURA) {
            throw SDKError.validation(
                "runtime",
                "descriptor_ref provider \(provider) subject_ura must be a runtime governance read subject"
            )
        }
    }

    private static func required(_ value: String, _ field: String) throws -> String {
        let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else {
            throw SDKError.validation("runtime", "\(field) is required")
        }
        return clean
    }

    private static func optionalPrincipal(_ value: String, _ field: String) throws -> String {
        let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if !clean.isEmpty, RuntimePrincipals.containsAllZeroPrincipal(clean) {
            throw SDKError.validation("runtime", "\(field) must not be all-zero")
        }
        return clean
    }
}

enum RuntimeDescriptorRefResponse {
    static func fromJSON(_ raw: Data) throws -> String {
        let text = String(data: raw, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !text.isEmpty else {
            throw SDKError.validation("runtime", "descriptor_ref resolution omitted descriptor_ref")
        }
        if !text.hasPrefix("{") {
            return try requiredText(text, "descriptor_ref", stage: "runtime")
        }
        let object = try runtimeDescriptorJSONObject(raw, stage: "runtime")
        return try requiredText(object["descriptor_ref"]?.stringValue ?? "", "descriptor_ref", stage: "runtime")
    }
}

public struct RuntimeCallContext: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try Self.requiredPrincipal(callerURA, "caller_ura")
        self.calleeURA = try Self.requiredPrincipal(calleeURA, "callee_ura")
        self.subjectURA = try Self.requiredPrincipal(subjectURA, "subject_ura")
        self.nonceBase64 = try requiredInvocationNonceBase64(nonceBase64, stage: "runtime")
        self.causalContext = try Self.copyObject(causalContext, "causal_context")
        self.metadata = try Self.copyObject(metadata, "metadata")
    }

    private static func copyObject(_ value: [String: JSONValue], _ field: String) throws -> [String: JSONValue] {
        for key in value.keys where key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw SDKError.validation("runtime", "\(field) keys must be non-empty strings")
        }
        return value
    }

    private static func required(_ value: String, _ field: String) throws -> String {
        let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else {
            throw SDKError.validation("runtime", "\(field) is required")
        }
        return clean
    }

    private static func requiredPrincipal(_ value: String, _ field: String) throws -> String {
        let clean = try required(value, field)
        if RuntimePrincipals.containsAllZeroPrincipal(clean) {
            throw SDKError.validation("runtime", "\(field) must not be all-zero")
        }
        return clean
    }
}

public final class RuntimeAbilityClient: @unchecked Sendable {
    private let runtime: RuntimeClient

    public init(runtime: RuntimeClient) {
        self.runtime = runtime
    }

    public func build(
        call: RuntimeCallContext,
        abilityName: String,
        arguments: [String: JSONValue] = [:]
    ) async throws -> InvocationDraft {
        try await buildWithPolicy(
            call: call,
            abilityName: abilityName,
            arguments: arguments,
            callMode: "rpc",
            policy: .publicAction
        )
    }

    public func invoke(
        call: RuntimeCallContext,
        abilityName: String,
        arguments: [String: JSONValue] = [:]
    ) async throws -> [String: JSONValue] {
        let draft = try await build(call: call, abilityName: abilityName, arguments: arguments)
        return try await runtimeAbilityObjectOutput(runtime.invoke(draft))
    }

    func buildCatalogueRead(
        call: RuntimeCallContext,
        abilityName: String,
        arguments: [String: JSONValue] = [:]
    ) async throws -> InvocationDraft {
        try await buildWithPolicy(
            call: call,
            abilityName: abilityName,
            arguments: arguments,
            callMode: "rpc",
            policy: .catalogueRead
        )
    }

    func invokeCatalogueRead(
        call: RuntimeCallContext,
        abilityName: String,
        arguments: [String: JSONValue] = [:]
    ) async throws -> [String: JSONValue] {
        let draft = try await buildCatalogueRead(call: call, abilityName: abilityName, arguments: arguments)
        return try await runtimeAbilityObjectOutput(runtime.invoke(draft))
    }

    private func buildWithPolicy(
        call: RuntimeCallContext,
        abilityName: String,
        arguments: [String: JSONValue],
        callMode: String,
        policy: RuntimeAbilityDispatchPolicy
    ) async throws -> InvocationDraft {
        let ability = try requiredText(abilityName, "ability name", stage: "runtime")
        if !policy.allowGovernanceRead,
           !RuntimeAbilityProjection.runtimeGovernanceDescriptorProvider(forAbility: ability).isEmpty {
            throw SDKError.validation(
                "runtime",
                "runtime governance receipt/history/catalogue abilities must use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider"
            )
        }
        let subjectURA = try policy.subjectURA(call, abilityName: ability)
        let descriptorRef = try await runtime.resolveDescriptorRef(
            try RuntimeDescriptorRefRequest(
                calleeURA: call.calleeURA,
                ability: ability,
                callMode: callMode,
                callerURA: call.callerURA,
                subjectURA: try policy.descriptorResolutionSubjectURA(call, selectedSubjectURA: subjectURA),
                provider: policy.descriptorProvider
            )
        )
        let projection = try RuntimeAbilityProjection(tuple: InvocationTuple(
            caller: call.callerURA,
            callee: call.calleeURA,
            descriptorRef: descriptorRef,
            subject: subjectURA,
            nonce: call.nonceBase64,
            causalContext: try runtimeDescriptorJSONString(call.causalContext, stage: "runtime"),
            argsJSON: try runtimeDescriptorJSONString(arguments, stage: "runtime"),
            metadata: call.metadata
        ))
        var metadata = call.metadata
        metadata["ability_ura"] = .string(projection.abilityURA)
        let builder = InvocationBuilder()
            .withCallerURA(call.callerURA)
            .withCalleeURA(call.calleeURA)
            .withDescriptorRef(descriptorRef)
            .withSubjectURA(subjectURA)
            .withNonce(call.nonceBase64)
            .withCausalContext(try runtimeDescriptorJSONString(call.causalContext, stage: "runtime"))
            .withArgsJSON(try runtimeDescriptorJSONString(arguments, stage: "runtime"))
            .withMetadata(metadata)
        if policy.allowGovernanceRead {
            builder.runtimeGovernanceRead()
        }
        return try builder.inspect()
    }

    private func runtimeAbilityObjectOutput(_ result: InvocationResult) throws -> [String: JSONValue] {
        if !result.ok {
            throw SDKError(
                code: .abilityFailed,
                stage: "runtime",
                retryHint: .never,
                retryable: false,
                message: "runtime ability invocation failed",
                details: ["terminal_state": result.terminalState.rawValue]
            )
        }
        return try decodeObject(Data(result.outputJSON.utf8), label: "runtime ability output")
    }

    private struct RuntimeAbilityDispatchPolicy {
        let allowGovernanceRead: Bool
        let subjectPolicy: SubjectPolicy
        let descriptorProvider: String

        static let publicAction = RuntimeAbilityDispatchPolicy(
            allowGovernanceRead: false,
            subjectPolicy: .descriptorBound,
            descriptorProvider: ""
        )
        static let catalogueRead = RuntimeAbilityDispatchPolicy(
            allowGovernanceRead: true,
            subjectPolicy: .runtimeOwner,
            descriptorProvider: RuntimeDescriptorRefRequest.abilityDescriptorProvider
        )

        func subjectURA(_ call: RuntimeCallContext, abilityName: String) throws -> String {
            switch subjectPolicy {
            case .descriptorBound:
                return try RuntimeSubjects.descriptorBoundSubjectURA(call.subjectURA, abilityName: abilityName)
            case .runtimeOwner:
                return call.calleeURA
            }
        }

        func descriptorResolutionSubjectURA(
            _ call: RuntimeCallContext,
            selectedSubjectURA: String
        ) throws -> String {
            if descriptorProvider == RuntimeDescriptorRefRequest.abilityDescriptorProvider {
                return selectedSubjectURA
            }
            if subjectPolicy == .runtimeOwner {
                return selectedSubjectURA
            }
            return call.subjectURA
        }

        enum SubjectPolicy {
            case descriptorBound
            case runtimeOwner
        }
    }
}

public struct AbilityDescriptorHints: Sendable, Equatable {
    public let readOnly: Bool
    public let destructive: Bool
    public let idempotent: Bool
    public let streamingOnly: Bool
    public let bidiOnly: Bool

    public init(
        readOnly: Bool = false,
        destructive: Bool = false,
        idempotent: Bool = false,
        streamingOnly: Bool = false,
        bidiOnly: Bool = false
    ) {
        self.readOnly = readOnly
        self.destructive = destructive
        self.idempotent = idempotent
        self.streamingOnly = streamingOnly
        self.bidiOnly = bidiOnly
    }

    static func fromObject(_ raw: [String: JSONValue], rowIndex: Int) throws -> AbilityDescriptorHints {
        try AbilityDescriptorHints(
            readOnly: optionalDescriptorBool(raw, "hints.read_only", rowIndex: rowIndex),
            destructive: optionalDescriptorBool(raw, "hints.destructive", rowIndex: rowIndex),
            idempotent: optionalDescriptorBool(raw, "hints.idempotent", rowIndex: rowIndex),
            streamingOnly: optionalDescriptorBool(raw, "hints.streaming_only", rowIndex: rowIndex),
            bidiOnly: optionalDescriptorBool(raw, "hints.bidi_only", rowIndex: rowIndex)
        )
    }
}

public struct AbilityDescriptorProjection: Sendable, Equatable {
    public let abilityURA: String
    public let descriptorRef: String
    public let name: String
    public let ownerURA: String
    public let version: String
    public let schemaHash: String
    public let descriptorHash: String
    public let callMode: String
    public let className: String
    public let receiptSemantics: [String: JSONValue]
    public let visibility: String
    public let source: String
    public let description: String
    public let hints: AbilityDescriptorHints
    public let schemaSummary: [String: JSONValue]
    public let inputSchema: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public init(
        abilityURA: String,
        descriptorRef: String,
        name: String,
        ownerURA: String,
        version: String,
        schemaHash: String = "",
        descriptorHash: String = "",
        callMode: String = "",
        className: String = "",
        receiptSemantics: [String: JSONValue] = [:],
        visibility: String = "",
        source: String = "",
        description: String = "",
        hints: AbilityDescriptorHints = AbilityDescriptorHints(),
        schemaSummary: [String: JSONValue] = [:],
        inputSchema: [String: JSONValue] = [:],
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.abilityURA = try requiredText(abilityURA, "ability_ura", stage: "ability_descriptor")
        self.descriptorRef = try requiredText(descriptorRef, "descriptor_ref", stage: "ability_descriptor")
        self.name = try requiredText(name, "name", stage: "ability_descriptor")
        self.ownerURA = try requiredText(ownerURA, "owner_ura", stage: "ability_descriptor")
        self.version = try requiredText(version, "version", stage: "ability_descriptor")
        self.schemaHash = schemaHash.trimmingCharacters(in: .whitespacesAndNewlines)
        self.descriptorHash = descriptorHash.trimmingCharacters(in: .whitespacesAndNewlines)
        self.callMode = callMode.trimmingCharacters(in: .whitespacesAndNewlines)
        self.className = className.trimmingCharacters(in: .whitespacesAndNewlines)
        self.receiptSemantics = receiptSemantics
        self.visibility = visibility.trimmingCharacters(in: .whitespacesAndNewlines)
        self.source = source.trimmingCharacters(in: .whitespacesAndNewlines)
        self.description = description.trimmingCharacters(in: .whitespacesAndNewlines)
        self.hints = hints
        self.schemaSummary = schemaSummary
        self.inputSchema = inputSchema
        self.metadata = metadata
        let projected = try RuntimeAbilityProjection.abilityURAForDescriptorRef(self.descriptorRef)
        guard projected == self.abilityURA else {
            throw SDKError.validation(
                "ability_descriptor",
                "ability descriptor descriptor_ref does not bind ability_ura"
            )
        }
    }

    static func fromObject(_ raw: [String: JSONValue], rowIndex: Int = 0) throws -> AbilityDescriptorProjection {
        let version = try requiredDescriptorString(raw, "descriptor_version", rowIndex: rowIndex)
        return try AbilityDescriptorProjection(
            abilityURA: try requiredDescriptorString(raw, "ability_ura", rowIndex: rowIndex),
            descriptorRef: try requiredDescriptorString(raw, "descriptor_ref", rowIndex: rowIndex),
            name: try requiredDescriptorString(raw, "name", rowIndex: rowIndex),
            ownerURA: try requiredDescriptorString(raw, "owner_ura", rowIndex: rowIndex),
            version: version,
            schemaHash: try optionalDescriptorString(raw, "schema_hash", rowIndex: rowIndex),
            descriptorHash: try optionalDescriptorString(raw, "descriptor_hash", rowIndex: rowIndex),
            callMode: try optionalDescriptorString(raw, "call_mode", rowIndex: rowIndex),
            className: try optionalDescriptorString(raw, "class", rowIndex: rowIndex),
            receiptSemantics: try optionalDescriptorObject(raw, "receipt_semantics", rowIndex: rowIndex),
            visibility: try optionalDescriptorString(raw, "visibility", rowIndex: rowIndex),
            source: try optionalDescriptorString(raw, "source", rowIndex: rowIndex),
            description: try optionalDescriptorString(raw, "description", rowIndex: rowIndex),
            hints: AbilityDescriptorHints.fromObject(
                try optionalDescriptorObject(raw, "hints", rowIndex: rowIndex),
                rowIndex: rowIndex
            ),
            schemaSummary: try optionalDescriptorObject(raw, "schema_summary", rowIndex: rowIndex),
            inputSchema: try optionalDescriptorObject(raw, "input_schema", rowIndex: rowIndex),
            metadata: try optionalDescriptorObject(raw, "metadata", rowIndex: rowIndex)
        )
    }
}

public struct AbilityDescriptorListRequest: Sendable, Equatable {
    public let call: RuntimeCallContext
    public let scope: String
    public let ownerURA: String
    public let abilityURA: String

    public init(
        call: RuntimeCallContext,
        scope: String = "",
        ownerURA: String = "",
        abilityURA: String = ""
    ) {
        self.call = call
        self.scope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
        self.ownerURA = ownerURA.trimmingCharacters(in: .whitespacesAndNewlines)
        self.abilityURA = abilityURA.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public struct AbilityDescriptorGetRequest: Sendable, Equatable {
    public let call: RuntimeCallContext
    public let abilityURA: String
    public let descriptorVersion: String
    public let callMode: String
    public let scope: String

    public init(
        call: RuntimeCallContext,
        abilityURA: String,
        descriptorVersion: String = "",
        callMode: String = "",
        scope: String = ""
    ) throws {
        self.call = call
        self.abilityURA = try requiredText(abilityURA, "ability_ura", stage: "ability_descriptor")
        self.descriptorVersion = descriptorVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        self.callMode = callMode.trimmingCharacters(in: .whitespacesAndNewlines)
        self.scope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public struct AbilityDescriptorPage: Sendable, Equatable {
    public let descriptors: [AbilityDescriptorProjection]

    public init(descriptors: [AbilityDescriptorProjection]) {
        self.descriptors = descriptors
    }
}

public protocol AbilityDescriptorProvider: Sendable {
    func list(_ request: AbilityDescriptorListRequest) async throws -> AbilityDescriptorPage
    func get(_ request: AbilityDescriptorGetRequest) async throws -> AbilityDescriptorProjection
}

public final class AbilityDescriptorClient: @unchecked Sendable {
    private let provider: any AbilityDescriptorProvider

    public init(provider: any AbilityDescriptorProvider) {
        self.provider = provider
    }

    public func list(_ request: AbilityDescriptorListRequest) async throws -> AbilityDescriptorPage {
        try await provider.list(request)
    }

    public func get(_ request: AbilityDescriptorGetRequest) async throws -> AbilityDescriptorProjection {
        try await provider.get(request)
    }
}

public final class RuntimeAbilityDescriptorProvider: AbilityDescriptorProvider, @unchecked Sendable {
    private static let listAbility = "meta.list_abilities"
    private static let rowsField = "abilities"

    private let ability: RuntimeAbilityClient

    public init(ability: RuntimeAbilityClient) {
        self.ability = ability
    }

    public func list(_ request: AbilityDescriptorListRequest) async throws -> AbilityDescriptorPage {
        var args: [String: JSONValue] = [:]
        if !request.scope.isEmpty {
            args["scope"] = .string(request.scope)
        }
        if !request.ownerURA.isEmpty {
            args["owner_ura"] = .string(request.ownerURA)
        }
        if !request.abilityURA.isEmpty {
            args["ability_ura"] = .string(request.abilityURA)
        }
        let output = try await ability.invokeCatalogueRead(
            call: request.call,
            abilityName: Self.listAbility,
            arguments: args
        )
        guard case let .array(rows)? = output[Self.rowsField] else {
            throw SDKError.validation(
                "ability_descriptor",
                "runtime descriptor catalog output must include descriptor rows"
            )
        }
        var descriptors: [AbilityDescriptorProjection] = []
        descriptors.reserveCapacity(rows.count)
        for (index, row) in rows.enumerated() {
            guard case let .object(object) = row else {
                throw SDKError.validation(
                    "ability_descriptor",
                    "ability descriptor row \(index) must be an object"
                )
            }
            descriptors.append(try AbilityDescriptorProjection.fromObject(object, rowIndex: index))
        }
        return AbilityDescriptorPage(descriptors: descriptors)
    }

    public func get(_ request: AbilityDescriptorGetRequest) async throws -> AbilityDescriptorProjection {
        let page = try await list(
            AbilityDescriptorListRequest(
                call: request.call,
                scope: request.scope,
                abilityURA: request.abilityURA
            )
        )
        let matches = page.descriptors.filter { descriptor in
            descriptor.abilityURA == request.abilityURA &&
                (request.descriptorVersion.isEmpty || descriptor.version == request.descriptorVersion) &&
                (request.callMode.isEmpty || descriptor.callMode == request.callMode)
        }
        if page.descriptors.contains(where: { $0.abilityURA != request.abilityURA }) {
            throw SDKError.validation(
                "ability_descriptor",
                "runtime returned descriptor outside requested ability_ura"
            )
        }
        if matches.isEmpty {
            throw SDKError(
                code: .descriptorNotFound,
                stage: "ability_descriptor",
                retryHint: .never,
                retryable: false,
                message: "ability descriptor not found",
                details: ["ability_ura": request.abilityURA]
            )
        }
        if matches.count > 1 {
            throw SDKError.validation(
                "ability_descriptor",
                "ability descriptor selection is ambiguous; specify descriptor_version or call_mode"
            )
        }
        return matches[0]
    }
}

private func runtimeDescriptorJSONObject(_ raw: Data, stage: String) throws -> [String: JSONValue] {
    try decodeObject(raw, label: stage)
}

private func runtimeDescriptorJSONData(_ value: Any, stage: String) throws -> Data {
    do {
        return try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    } catch {
        throw SDKError.validation(stage, "JSON value must be encodable")
    }
}

private func runtimeDescriptorJSONString(_ value: [String: JSONValue], stage: String) throws -> String {
    let plain = try value.mapValues(runtimeDescriptorPlainValue)
    let data = try runtimeDescriptorJSONData(plain, stage: stage)
    return String(data: data, encoding: .utf8) ?? ""
}

private func runtimeDescriptorPlainValue(_ value: JSONValue) throws -> Any {
    switch value {
    case .null:
        return NSNull()
    case let .bool(value):
        return value
    case let .number(value):
        return value
    case let .string(value):
        return value
    case let .array(values):
        return try values.map(runtimeDescriptorPlainValue)
    case let .object(values):
        return try values.mapValues(runtimeDescriptorPlainValue)
    }
}

private func requiredText(_ value: Any?, _ field: String, stage: String) throws -> String {
    guard let string = value as? String else {
        throw SDKError.validation(stage, "\(field) is required")
    }
    return try requiredText(string, field, stage: stage)
}

private func requiredText(_ value: String, _ field: String, stage: String) throws -> String {
    let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !clean.isEmpty else {
        throw SDKError.validation(stage, "\(field) is required")
    }
    return clean
}

private func requiredDescriptorString(
    _ raw: [String: JSONValue],
    _ field: String,
    rowIndex: Int
) throws -> String {
    guard let value = raw[field] else {
        return ""
    }
    guard case let .string(string) = value else {
        throw SDKError.validation(
            "ability_descriptor",
            "ability descriptor row \(rowIndex) field \(field) must be a string"
        )
    }
    return string.trimmingCharacters(in: .whitespacesAndNewlines)
}

private func optionalDescriptorString(
    _ raw: [String: JSONValue],
    _ field: String,
    rowIndex: Int
) throws -> String {
    guard let value = raw[field], value != .null else {
        return ""
    }
    guard case let .string(string) = value else {
        throw SDKError.validation(
            "ability_descriptor",
            "ability descriptor row \(rowIndex) field \(field) must be a string"
        )
    }
    return string.trimmingCharacters(in: .whitespacesAndNewlines)
}

private func optionalDescriptorObject(
    _ raw: [String: JSONValue],
    _ field: String,
    rowIndex: Int
) throws -> [String: JSONValue] {
    guard let value = raw[field], value != .null else {
        return [:]
    }
    guard case let .object(object) = value else {
        throw SDKError.validation(
            "ability_descriptor",
            "ability descriptor row \(rowIndex) field \(field) must be an object"
        )
    }
    return object
}

private func optionalDescriptorBool(
    _ raw: [String: JSONValue],
    _ field: String,
    rowIndex: Int
) throws -> Bool {
    let key = field.split(separator: ".").last.map(String.init) ?? field
    guard let value = raw[key], value != .null else {
        return false
    }
    guard case let .bool(bool) = value else {
        throw SDKError.validation(
            "ability_descriptor",
            "ability descriptor row \(rowIndex) field \(field) must be a boolean"
        )
    }
    return bool
}

private extension JSONValue {
    var stringValue: String? {
        if case let .string(value) = self {
            return value
        }
        return nil
    }

    var boolValue: Bool? {
        if case let .bool(value) = self {
            return value
        }
        return nil
    }
}
