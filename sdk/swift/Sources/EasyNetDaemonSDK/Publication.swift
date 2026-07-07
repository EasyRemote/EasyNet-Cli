import Foundation

public let publicationProfile = "publication"

public struct LocalResourceRefRequest: Sendable, Equatable {
    public let path: String
    public let capability: String

    public init(path: String, capability: String) throws {
        self.path = try cleanPublicationAbsolutePath(path)
        self.capability = try cleanPublicationCapability(capability)
    }

    func jsonData() throws -> Data {
        try encodeJSONObject(["path": .string(path), "capability": .string(capability)])
    }
}

public struct ResourceRef: Sendable, Equatable {
    public let resourceURA: String
    public let ownerURA: String
    public let namespace: String
    public let displayPath: String
    public let capability: String
    public let expiresUnixMS: Int64
    public let revision: String

    public init(
        resourceURA: String,
        ownerURA: String,
        namespace: String,
        displayPath: String = "",
        capability: String,
        expiresUnixMS: Int64,
        revision: String
    ) throws {
        self.resourceURA = try cleanPublicationString(resourceURA, "resource_ura")
        self.ownerURA = try cleanPublicationString(ownerURA, "owner_ura")
        self.namespace = try cleanPublicationString(namespace, "namespace")
        guard self.namespace == "fs" else {
            throw invalidPublication("resource_ref namespace is unsupported")
        }
        self.displayPath = try optionalPublicationString(displayPath, "display_path")
        self.capability = try cleanPublicationCapability(capability)
        guard expiresUnixMS >= 0 else {
            throw invalidPublication("expires_unix_ms must be non-negative")
        }
        self.expiresUnixMS = expiresUnixMS
        self.revision = try cleanPublicationString(revision, "revision")
    }

    public static func fromJSON(_ raw: Data) throws -> ResourceRef {
        try fromObject(decodePublicationObject(raw, label: "resource ref JSON"))
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> ResourceRef {
        try ResourceRef(
            resourceURA: requiredPublicationString(object, "resource_ura"),
            ownerURA: requiredPublicationString(object, "owner_ura"),
            namespace: requiredPublicationString(object, "namespace"),
            displayPath: optionalPublicationJSONString(object["display_path"], "display_path") ?? "",
            capability: requiredPublicationString(object, "capability"),
            expiresUnixMS: Int64(requiredPublicationInt(object, "expires_unix_ms")),
            revision: requiredPublicationString(object, "revision")
        )
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "resource_ura": .string(resourceURA),
            "owner_ura": .string(ownerURA),
            "namespace": .string(namespace),
            "capability": .string(capability),
            "expires_unix_ms": .number(Double(expiresUnixMS)),
            "revision": .string(revision),
        ]
        if !displayPath.isEmpty { object["display_path"] = .string(displayPath) }
        return object
    }
}

public struct AbilityPackageManifest: Sendable, Equatable {
    public let name: String
    public let namespace: String
    public let description: String
    public let inputSchema: [String: JSONValue]
    public let outputSchema: JSONValue?
    public let descriptorVersion: String
    public let exec: [String: JSONValue]

    public init(
        name: String,
        namespace: String,
        description: String,
        inputSchema: [String: JSONValue],
        outputSchema: JSONValue? = nil,
        descriptorVersion: String = "",
        exec: [String: JSONValue] = [:]
    ) throws {
        self.name = try cleanPublicationString(name, "name")
        self.namespace = try cleanPublicationString(namespace, "namespace")
        self.description = description
        guard !inputSchema.isEmpty else {
            throw invalidPublication("input_schema is required")
        }
        self.inputSchema = inputSchema
        self.outputSchema = outputSchema
        self.descriptorVersion = try optionalPublicationString(descriptorVersion, "descriptor_version")
        self.exec = exec
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "name": .string(name),
            "namespace": .string(namespace),
            "description": .string(description),
            "input_schema": .object(inputSchema),
        ]
        if let outputSchema { object["output_schema"] = outputSchema }
        if !descriptorVersion.isEmpty { object["descriptor_version"] = .string(descriptorVersion) }
        if !exec.isEmpty { object["exec"] = .object(exec) }
        return object
    }
}

public struct ValidatePackageOptions: Sendable, Equatable {
    public let manifest: AbilityPackageManifest?
    public let metadata: [String: JSONValue]

    public init(manifest: AbilityPackageManifest? = nil, metadata: [String: JSONValue] = [:]) {
        self.manifest = manifest
        self.metadata = metadata
    }

    func jsonData(packagePath: String) throws -> Data {
        var object: [String: JSONValue] = [:]
        let cleaned = try optionalPublicationString(packagePath, "package_path")
        if !cleaned.isEmpty { object["package_path"] = .string(cleaned) }
        if let manifest { object["manifest"] = .object(manifest.jsonObject()) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        if object.isEmpty {
            throw invalidPublication("package path or manifest is required")
        }
        return try encodeJSONObject(object)
    }
}

public struct PackageValidation: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let valid: Bool
    public let packagePath: String
    public let manifestPath: String
    public let manifestHash: String
    public let manifest: Manifest
    public let errors: [JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> PackageValidation {
        let object = try decodePublicationObject(raw, label: "package validation JSON")
        let validation = try PackageValidation(
            profile: requiredPublicationString(object, "profile"),
            kind: requiredPublicationString(object, "kind"),
            valid: requiredPublicationBool(object, "valid"),
            packagePath: requiredPublicationString(object, "package_path"),
            manifestPath: requiredPublicationString(object, "manifest_path"),
            manifestHash: requiredPublicationString(object, "manifest_hash"),
            manifest: Manifest.fromObject(requiredPublicationObject(object, "manifest")),
            errors: requiredPublicationArray(object, "errors"),
            metadata: requiredPublicationObject(object, "metadata")
        )
        guard validation.profile == publicationProfile, validation.kind == "package_validation" else {
            throw invalidPublication("invalid package validation projection")
        }
        return validation
    }

    public struct Manifest: Sendable, Equatable {
        public let name: String
        public let namespace: String
        public let wireKey: String
        public let descriptorVersion: String
        public let description: String
        public let execKind: String
        public let timeoutSeconds: Int?
        public let inputSchema: [String: JSONValue]
        public let outputSchema: JSONValue?

        static func fromObject(_ object: [String: JSONValue]) throws -> Manifest {
            try Manifest(
                name: requiredPublicationString(object, "name"),
                namespace: requiredPublicationString(object, "namespace"),
                wireKey: requiredPublicationString(object, "wire_key"),
                descriptorVersion: requiredPublicationString(object, "descriptor_version"),
                description: requiredPublicationString(object, "description"),
                execKind: requiredPublicationString(object, "exec_kind"),
                timeoutSeconds: optionalPublicationInt(object["timeout_seconds"], "timeout_seconds"),
                inputSchema: requiredPublicationObject(object, "input_schema"),
                outputSchema: object["output_schema"]
            )
        }
    }
}

public struct AbilityDeployRequest: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let resourceRef: ResourceRef
    public let nodeID: String
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        resourceRef: ResourceRef,
        nodeID: String,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try cleanPublicationString(callerURA, "caller_ura")
        self.calleeURA = try cleanPublicationString(calleeURA, "callee_ura")
        self.subjectURA = try cleanPublicationString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanPublicationString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanPublicationString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidPublication("causal_context is required")
        }
        self.causalContext = causalContext
        self.resourceRef = resourceRef
        self.nodeID = try cleanPublicationString(nodeID, "node_id")
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "caller_ura": .string(callerURA),
            "callee_ura": .string(calleeURA),
            "subject_ura": .string(subjectURA),
            "descriptor_version": .string(descriptorVersion),
            "nonce_base64": .string(nonceBase64),
            "causal_context": .object(causalContext),
            "resource_ref": .object(resourceRef.jsonObject()),
            "node_id": .string(nodeID),
        ]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public struct UnpublishAbilityRequest: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let abilityURA: String
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        abilityURA: String,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try cleanPublicationString(callerURA, "caller_ura")
        self.calleeURA = try cleanPublicationString(calleeURA, "callee_ura")
        self.subjectURA = try cleanPublicationString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanPublicationString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanPublicationString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidPublication("causal_context is required")
        }
        self.causalContext = causalContext
        self.abilityURA = try cleanPublicationString(abilityURA, "ability_ura")
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "caller_ura": .string(callerURA),
            "callee_ura": .string(calleeURA),
            "subject_ura": .string(subjectURA),
            "descriptor_version": .string(descriptorVersion),
            "nonce_base64": .string(nonceBase64),
            "causal_context": .object(causalContext),
            "ability_ura": .string(abilityURA),
        ]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public protocol PublicationTransport: AnyObject, Sendable {
    func buildResourceRef(_ requestJSON: Data) async throws -> Data
    func validatePackage(_ requestJSON: Data) async throws -> Data
    func buildDeployInvocation(_ requestJSON: Data) async throws -> Data
    func buildUnpublishInvocation(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension PublicationTransport {
    func buildResourceRef(_ requestJSON: Data) async throws -> Data { throw publicationUnsupported("publication resource-ref transport is not available") }
    func validatePackage(_ requestJSON: Data) async throws -> Data { throw publicationUnsupported("publication package validation transport is not available") }
    func buildDeployInvocation(_ requestJSON: Data) async throws -> Data { throw publicationUnsupported("publication deploy invocation transport is not available") }
    func buildUnpublishInvocation(_ requestJSON: Data) async throws -> Data { throw publicationUnsupported("publication unpublish invocation transport is not available") }
    func close() async throws {}
}

public final class PublicationClient: @unchecked Sendable {
    private let transport: PublicationTransport
    private var closed = false

    public init(transport: PublicationTransport) {
        self.transport = transport
    }

    public func buildLocalResourceRef(_ request: LocalResourceRefRequest) async throws -> ResourceRef {
        try await ResourceRef.fromJSON(raw { try await transport.buildResourceRef(request.jsonData()) })
    }

    public func validatePackage(_ packagePath: String = "", options: ValidatePackageOptions = ValidatePackageOptions()) async throws -> PackageValidation {
        try await PackageValidation.fromJSON(raw { try await transport.validatePackage(options.jsonData(packagePath: packagePath)) })
    }

    public func buildDeployInvocation(_ request: AbilityDeployRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildDeployInvocation(request.jsonData()) }
    }

    public func buildUnpublishInvocation(_ request: UnpublishAbilityRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildUnpublishInvocation(request.jsonData()) }
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodePublicationObject(try await raw(call), label: "publication invocation JSON")
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: "publication transport failed", details: ["profile": publicationProfile])
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("publication")
        }
    }
}

private func decodePublicationObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidPublication("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

private func requiredPublicationString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty { return value }
    throw invalidPublication("\(name) must be a non-empty string")
}

private func requiredPublicationBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] { return value }
    throw invalidPublication("\(name) must be a boolean")
}

private func requiredPublicationInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if case let .number(value) = object[name], value >= 0, value.rounded() == value, value <= Double(Int.max) {
        return Int(value)
    }
    throw invalidPublication("\(name) must be a non-negative integer")
}

private func optionalPublicationInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .number(number) where number >= 0 && number.rounded() == number && number <= Double(Int.max):
        return Int(number)
    default:
        throw invalidPublication("\(name) must be a non-negative integer or null")
    }
}

private func requiredPublicationArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    if case let .array(value) = object[name] { return value }
    throw invalidPublication("\(name) must be a list")
}

private func requiredPublicationObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] { return value }
    throw invalidPublication("\(name) must be an object")
}

private func optionalPublicationJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .string(string): return string
    default: throw invalidPublication("\(name) must be a string or null")
    }
}

private func cleanPublicationString(_ value: String, _ field: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidPublication("\(field) is required")
    }
    return value
}

private func optionalPublicationString(_ value: String, _ field: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidPublication("\(field) must not contain surrounding whitespace")
    }
    return value
}

private func cleanPublicationAbsolutePath(_ value: String) throws -> String {
    let cleaned = try cleanPublicationString(value, "path")
    guard cleaned.hasPrefix("/") else {
        throw invalidPublication("absolute resource path is required")
    }
    return cleaned
}

private func cleanPublicationCapability(_ value: String) throws -> String {
    let cleaned = try cleanPublicationString(value, "capability")
    guard ["list", "stat", "read", "write"].contains(cleaned) else {
        throw invalidPublication("invalid resource capability")
    }
    return cleaned
}

private func invalidPublication(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: publicationProfile, message: message, details: ["profile": publicationProfile])
}

private func publicationUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": publicationProfile])
}
