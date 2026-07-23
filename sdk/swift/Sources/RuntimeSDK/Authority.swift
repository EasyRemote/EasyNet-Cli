import Foundation

public let authorityProfile = "authority"
public let delegationMetadataKey = "x-runtime-delegation"
public let sessionAuthorityMetadataKey = "x-runtime-session-authority"

public struct AuthorityMetadata: Sendable, Equatable {
    public let kind: String
    public let key: String
    public let value: String

    public init(kind: String, key: String, value: String) throws {
        self.kind = try requiredAuthorityString(kind, "kind")
        self.key = try requiredAuthorityString(key, "key")
        self.value = try requiredAuthorityString(value, "value")
        try validateAuthorityMetadataEnvelope(kind: self.kind, key: self.key)
    }

    public func toMetadata() -> [String: JSONValue] {
        [key: .string(value)]
    }

    public func mergeInto(_ metadata: [String: JSONValue]) throws -> [String: JSONValue] {
        var merged = metadata
        merged[key] = .string(value)
        try validateAuthorityMetadata(merged)
        return merged
    }
}

public struct DelegationProof: Sendable, Equatable {
    public let issuerURA: String
    public let subjectURA: String
    public let callerURA: String
    public let audience: String
    public let scopes: [String]
    public let issuedAtMS: Int64
    public let expiresAtMS: Int64
    public let signatureBase64: String
    public let metadataValue: String

    public init(
        issuerURA: String,
        subjectURA: String,
        callerURA: String,
        audience: String,
        scopes: [String],
        issuedAtMS: Int64,
        expiresAtMS: Int64,
        signatureBase64: String,
        metadataValue: String
    ) throws {
        self.issuerURA = try requiredAuthorityURA(issuerURA, "issuer_ura")
        self.subjectURA = try requiredAuthorityURA(subjectURA, "subject_ura")
        self.callerURA = try requiredAuthorityURA(callerURA, "caller_ura")
        self.audience = try requiredAuthorityURA(audience, "audience")
        self.scopes = try requiredAuthorityScopes(scopes)
        guard expiresAtMS > issuedAtMS else {
            throw invalidAuthority("delegation authority expires_at_ms must be greater than issued_at_ms")
        }
        self.issuedAtMS = issuedAtMS
        self.expiresAtMS = expiresAtMS
        self.signatureBase64 = try requiredAuthorityBase64(signatureBase64, "signature_base64")
        self.metadataValue = try requiredAuthorityString(metadataValue, "metadata_value")
    }

    public static func fromMetadata(_ value: String) throws -> DelegationProof {
        let decoded = try decodeAuthorityMetadata(value, label: "delegation")
        let payload = decoded.payload
        return try DelegationProof(
            issuerURA: requiredAuthorityString(payload, "issuer_ura"),
            subjectURA: requiredAuthorityString(payload, "subject_ura"),
            callerURA: requiredAuthorityString(payload, "caller_ura"),
            audience: requiredAuthorityString(payload, "audience"),
            scopes: requiredAuthorityStringArray(payload, "scopes"),
            issuedAtMS: requiredAuthorityInteger(payload["issued_at_ms"], "issued_at_ms"),
            expiresAtMS: requiredAuthorityInteger(payload["expires_at_ms"], "expires_at_ms"),
            signatureBase64: decoded.signatureBase64,
            metadataValue: value
        )
    }

    public func metadata() throws -> AuthorityMetadata {
        try AuthorityMetadata(kind: "delegation", key: delegationMetadataKey, value: metadataValue)
    }
}

public struct SessionAuthority: Sendable, Equatable {
    public let issuerURA: String
    public let sessionID: String
    public let sessionOwnerUserID: String
    public let creatorPrincipalID: String
    public let calleeURA: String
    public let subjectURA: String
    public let audience: String
    public let scopes: [String]
    public let allowedActions: [String]
    public let allowedFollowupAbilities: [String]
    public let issuedAtMS: Int64
    public let expiresAtMS: Int64
    public let signatureBase64: String
    public let metadataValue: String

    public init(
        issuerURA: String,
        sessionID: String,
        sessionOwnerUserID: String,
        creatorPrincipalID: String,
        calleeURA: String,
        subjectURA: String,
        audience: String,
        scopes: [String],
        allowedActions: [String],
        allowedFollowupAbilities: [String],
        issuedAtMS: Int64,
        expiresAtMS: Int64,
        signatureBase64: String,
        metadataValue: String
    ) throws {
        self.issuerURA = try requiredAuthorityURA(issuerURA, "issuer_ura")
        self.sessionID = try requiredAuthorityString(sessionID, "session_id")
        self.sessionOwnerUserID = try requiredAuthorityPrincipalID(sessionOwnerUserID, "session_owner_user_id")
        self.creatorPrincipalID = try requiredAuthorityURA(creatorPrincipalID, "creator_principal_id")
        self.calleeURA = try requiredAuthorityURA(calleeURA, "callee_ura")
        self.subjectURA = try requiredAuthorityURA(subjectURA, "subject_ura")
        self.audience = try requiredAuthorityURA(audience, "audience")
        self.scopes = try requiredAuthorityScopes(scopes)
        self.allowedActions = try requiredAuthorityScopes(allowedActions)
        self.allowedFollowupAbilities = try requiredAuthorityScopes(allowedFollowupAbilities)
        guard expiresAtMS > issuedAtMS else {
            throw invalidAuthority("session authority expires_at_ms must be greater than issued_at_ms")
        }
        try validateSessionAuthoritySubjectBinding(subjectURA: self.subjectURA, sessionOwnerUserID: self.sessionOwnerUserID, sessionID: self.sessionID)
        self.issuedAtMS = issuedAtMS
        self.expiresAtMS = expiresAtMS
        self.signatureBase64 = try requiredAuthorityBase64(signatureBase64, "signature_base64")
        self.metadataValue = try requiredAuthorityString(metadataValue, "metadata_value")
    }

    public static func fromMetadata(_ value: String) throws -> SessionAuthority {
        let decoded = try decodeAuthorityMetadata(value, label: "session authority")
        let payload = decoded.payload
        return try SessionAuthority(
            issuerURA: requiredAuthorityString(payload, "issuer_ura"),
            sessionID: requiredAuthorityString(payload, "session_id"),
            sessionOwnerUserID: requiredAuthorityPrincipalID(requiredAuthorityString(payload, "session_owner_user_id"), "session_owner_user_id"),
            creatorPrincipalID: requiredAuthorityString(payload, "creator_principal_id"),
            calleeURA: requiredAuthorityString(payload, "callee_ura"),
            subjectURA: requiredAuthorityString(payload, "subject_ura"),
            audience: requiredAuthorityString(payload, "audience"),
            scopes: requiredAuthorityStringArray(payload, "scopes"),
            allowedActions: requiredAuthorityStringArray(payload, "allowed_actions"),
            allowedFollowupAbilities: requiredAuthorityStringArray(payload, "allowed_followup_abilities"),
            issuedAtMS: requiredAuthorityInteger(payload["issued_at_ms"], "issued_at_ms"),
            expiresAtMS: requiredAuthorityInteger(payload["expires_at_ms"], "expires_at_ms"),
            signatureBase64: decoded.signatureBase64,
            metadataValue: value
        )
    }

    public func metadata() throws -> AuthorityMetadata {
        try AuthorityMetadata(kind: "session_authority", key: sessionAuthorityMetadataKey, value: metadataValue)
    }
}

public struct DelegationRequest: Sendable, Equatable {
    public let issuerURA: String
    public let subjectURA: String
    public let callerURA: String
    public let audience: String
    public let scopes: [String]
    public let issuedAtMS: Int64
    public let expiresAtMS: Int64
    public let metadata: [String: JSONValue]

    public init(issuerURA: String, subjectURA: String, callerURA: String, audience: String, scopes: [String], issuedAtMS: Int64, expiresAtMS: Int64, metadata: [String: JSONValue] = [:]) throws {
        self.issuerURA = try requiredAuthorityURA(issuerURA, "issuer_ura")
        self.subjectURA = try requiredAuthorityURA(subjectURA, "subject_ura")
        self.callerURA = try requiredAuthorityURA(callerURA, "caller_ura")
        self.audience = try requiredAuthorityURA(audience, "audience")
        self.scopes = try requiredAuthorityScopes(scopes)
        guard expiresAtMS > issuedAtMS else {
            throw invalidAuthority("delegation request expires_at_ms must be greater than issued_at_ms")
        }
        self.issuedAtMS = issuedAtMS
        self.expiresAtMS = expiresAtMS
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        try encodeAuthorityJSONObject([
            "issuer_ura": .string(issuerURA),
            "subject_ura": .string(subjectURA),
            "caller_ura": .string(callerURA),
            "audience": .string(audience),
            "scopes": .array(scopes.map(JSONValue.string)),
            "issued_at_ms": .number(Double(issuedAtMS)),
            "expires_at_ms": .number(Double(expiresAtMS)),
            "metadata": .object(metadata),
        ])
    }
}

public struct SessionAuthorityRequest: Sendable, Equatable {
    public let issuerURA: String
    public let sessionID: String
    public let sessionOwnerUserID: String
    public let creatorPrincipalID: String
    public let calleeURA: String
    public let subjectURA: String
    public let audience: String
    public let scopes: [String]
    public let allowedActions: [String]
    public let allowedFollowupAbilities: [String]
    public let issuedAtMS: Int64
    public let expiresAtMS: Int64
    public let metadata: [String: JSONValue]

    public init(issuerURA: String, sessionID: String, sessionOwnerUserID: String, creatorPrincipalID: String, calleeURA: String, subjectURA: String, audience: String, scopes: [String], allowedActions: [String], allowedFollowupAbilities: [String], issuedAtMS: Int64, expiresAtMS: Int64, metadata: [String: JSONValue] = [:]) throws {
        self.issuerURA = try requiredAuthorityURA(issuerURA, "issuer_ura")
        self.sessionID = try requiredAuthorityString(sessionID, "session_id")
        self.sessionOwnerUserID = try requiredAuthorityPrincipalID(sessionOwnerUserID, "session_owner_user_id")
        self.creatorPrincipalID = try requiredAuthorityURA(creatorPrincipalID, "creator_principal_id")
        self.calleeURA = try requiredAuthorityURA(calleeURA, "callee_ura")
        self.subjectURA = try requiredAuthorityURA(subjectURA, "subject_ura")
        self.audience = try requiredAuthorityURA(audience, "audience")
        self.scopes = try requiredAuthorityScopes(scopes)
        self.allowedActions = try requiredAuthorityScopes(allowedActions)
        self.allowedFollowupAbilities = try requiredAuthorityScopes(allowedFollowupAbilities)
        guard expiresAtMS > issuedAtMS else {
            throw invalidAuthority("session authority request expires_at_ms must be greater than issued_at_ms")
        }
        try validateSessionAuthoritySubjectBinding(subjectURA: self.subjectURA, sessionOwnerUserID: self.sessionOwnerUserID, sessionID: self.sessionID)
        self.issuedAtMS = issuedAtMS
        self.expiresAtMS = expiresAtMS
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        try encodeAuthorityJSONObject([
            "issuer_ura": .string(issuerURA),
            "session_id": .string(sessionID),
            "session_owner_user_id": .string(sessionOwnerUserID),
            "creator_principal_id": .string(creatorPrincipalID),
            "callee_ura": .string(calleeURA),
            "subject_ura": .string(subjectURA),
            "audience": .string(audience),
            "scopes": .array(scopes.map(JSONValue.string)),
            "allowed_actions": .array(allowedActions.map(JSONValue.string)),
            "allowed_followup_abilities": .array(allowedFollowupAbilities.map(JSONValue.string)),
            "issued_at_ms": .number(Double(issuedAtMS)),
            "expires_at_ms": .number(Double(expiresAtMS)),
            "metadata": .object(metadata),
        ])
    }
}

public protocol AuthorityTransport: AnyObject, Sendable {
    func mintDelegationProof(_ requestJSON: Data) async throws -> Data
    func mintSessionAuthority(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension AuthorityTransport {
    func close() async throws {}
}

public final class AuthorityClient: @unchecked Sendable {
    private let transport: AuthorityTransport
    private var closed = false

    public init(transport: AuthorityTransport) {
        self.transport = transport
    }

    public func mintDelegationProof(_ request: DelegationRequest) async throws -> DelegationProof {
        let value = try await raw { try await transport.mintDelegationProof(request.jsonData()) }
        return try DelegationProof.fromMetadata(value)
    }

    public func mintSessionAuthority(_ request: SessionAuthorityRequest) async throws -> SessionAuthority {
        let value = try await raw { try await transport.mintSessionAuthority(request.jsonData()) }
        return try SessionAuthority.fromMetadata(value)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func raw(_ call: () async throws -> Data) async throws -> String {
        try requireOpen()
        do {
            return try decodeAuthorityMetadataProjection(try await call())
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(code: .transport, stage: authorityProfile, retryHint: .safe, retryable: true, message: "authority transport failed")
        }
    }

    private func requireOpen() throws {
        if closed { throw SDKError.closed(authorityProfile) }
    }
}

func validateAuthorityMetadata(_ metadata: [String: JSONValue]) throws {
    let delegation = try authorityMetadataValue(metadata, delegationMetadataKey)
    let session = try authorityMetadataValue(metadata, sessionAuthorityMetadataKey)
    if !delegation.isEmpty && !session.isEmpty {
        throw invalidAuthority("invocation authority metadata is ambiguous")
    }
}

func invocationMetadataObject(_ value: Any?) throws -> [String: JSONValue] {
    guard let value, !(value is NSNull) else {
        return [:]
    }
    guard let object = try jsonValue(value).objectValue else {
        throw SDKError.validation("invocation", "metadata must be an object")
    }
    return object
}

func jsonPlainValue(_ value: JSONValue) throws -> Any {
    switch value {
    case .null:
        return NSNull()
    case let .bool(bool):
        return bool
    case let .number(number):
        guard number.isFinite else {
            throw SDKError.validation("invocation", "metadata number must be finite")
        }
        return number
    case let .string(string):
        return string
    case let .array(array):
        return try array.map(jsonPlainValue)
    case let .object(object):
        return try object.mapValues(jsonPlainValue)
    }
}

private func decodeAuthorityMetadataProjection(_ raw: Data) throws -> String {
    let object = try decodeObject(raw, label: "authority projection JSON")
    if let value = try? authorityMetadataValue(object, "metadata_value"), !value.isEmpty {
        return value
    }
    if case let .object(metadata) = object["metadata"] {
        let delegation = try authorityMetadataValue(metadata, delegationMetadataKey)
        if !delegation.isEmpty { return delegation }
        let session = try authorityMetadataValue(metadata, sessionAuthorityMetadataKey)
        if !session.isEmpty { return session }
    }
    throw invalidAuthority("authority metadata projection missing metadata_value")
}

private struct DecodedAuthority {
    let payload: [String: JSONValue]
    let signatureBase64: String
}

private func decodeAuthorityMetadata(_ value: String, label: String) throws -> DecodedAuthority {
    let cleaned = try requiredAuthorityString(value, "metadata_value")
    guard let data = Data(base64Encoded: cleaned) else {
        throw invalidAuthority("\(label) metadata must be base64 JSON")
    }
    let object = try decodeObject(data, label: "\(label) authority metadata")
    let payload = try requiredAuthorityObject(object, "payload")
    let signature = try requiredAuthorityBase64(requiredAuthorityString(object, "signature"), "signature")
    return DecodedAuthority(payload: payload, signatureBase64: signature)
}

private func validateAuthorityMetadataEnvelope(kind: String, key: String) throws {
    switch (kind, key) {
    case ("delegation", delegationMetadataKey):
        return
    case ("session_authority", sessionAuthorityMetadataKey):
        return
    case ("delegation", _), ("session_authority", _):
        throw invalidAuthority("authority kind and metadata key mismatch")
    default:
        throw invalidAuthority("authority kind is not supported")
    }
}

private func authorityMetadataValue(_ metadata: [String: JSONValue], _ key: String) throws -> String {
    guard let value = metadata[key] else {
        return ""
    }
    switch value {
    case .null:
        return ""
    case let .string(string):
        return string
    default:
        throw invalidAuthority("\(key) must be a string metadata value")
    }
}

private func requiredAuthorityString(_ value: String, _ field: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidAuthority("\(field) is required")
    }
    return value
}

private func requiredAuthorityString(_ object: [String: JSONValue], _ field: String) throws -> String {
    if case let .string(value) = object[field] {
        return try requiredAuthorityString(value, field)
    }
    throw invalidAuthority("\(field) is required")
}

private func requiredAuthorityURA(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredAuthorityString(value, field)
    try rejectAllZeroAuthorityField(cleaned, field)
    guard cleaned.hasPrefix("easynet:///r/") else {
        throw invalidAuthority("\(field) must be a URA")
    }
    return cleaned
}

private func requiredAuthorityPrincipalID(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredAuthorityString(value, field)
    try rejectAllZeroAuthorityField(cleaned, field)
    return cleaned
}

private func rejectAllZeroAuthorityField(_ value: String, _ field: String) throws {
    if containsAllZeroPrincipal(value) {
        throw invalidAuthority("\(field) must not be all-zero")
    }
}

private let allZeroPrincipalID = "00000000-0000-0000-0000-000000000000"

func containsAllZeroPrincipal(_ value: String) -> Bool {
    value.trimmingCharacters(in: .whitespacesAndNewlines)
        .lowercased()
        .contains(allZeroPrincipalID)
}

private struct AuthoritySubject {
    let kind: String
    let ownerUserID: String
    let sessionID: String
}

private func validateSessionAuthoritySubjectBinding(subjectURA: String, sessionOwnerUserID: String, sessionID: String) throws {
    guard let subject = try canonicalAuthoritySubject(subjectURA) else {
        throw invalidAuthority("session authority subject_ura must be a canonical user or session subject")
    }
    let owner = try requiredAuthorityPrincipalID(sessionOwnerUserID, "session_owner_user_id")
    guard subject.ownerUserID == owner else {
        throw invalidAuthority("session authority user subject must match session_owner_user_id")
    }
    if subject.kind == "session" {
        let expectedSessionID = try requiredAuthorityString(sessionID, "session_id")
        guard subject.sessionID == expectedSessionID else {
            throw invalidAuthority("session authority subject_ura owner/session must match session_owner_user_id and session_id")
        }
    }
}

private func canonicalAuthoritySubject(_ subjectURA: String) throws -> AuthoritySubject? {
    let raw = try requiredAuthorityURA(subjectURA, "subject_ura")
    let realmPrefix = "easynet:///r/"
    guard raw.hasPrefix(realmPrefix) else {
        return nil
    }
    let rest = String(raw.dropFirst(realmPrefix.count))
    guard let slash = rest.firstIndex(of: "/"), slash != rest.startIndex else {
        return nil
    }
    let path = String(rest[rest.index(after: slash)...])
    let userPrefix = "user/"
    if path.hasPrefix(userPrefix) {
        let ownerUserID = String(path.dropFirst(userPrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !ownerUserID.isEmpty, !ownerUserID.contains("/") else {
            return nil
        }
        return AuthoritySubject(kind: "user", ownerUserID: ownerUserID, sessionID: "")
    }
    let resourcePrefix = "resource/user."
    guard path.hasPrefix(resourcePrefix) else {
        return nil
    }
    let resource = String(path.dropFirst(resourcePrefix.count))
    let marker = "/session/"
    guard let markerRange = resource.range(of: marker), markerRange.lowerBound != resource.startIndex else {
        return nil
    }
    let ownerUserID = String(resource[..<markerRange.lowerBound]).trimmingCharacters(in: .whitespacesAndNewlines)
    let authoritySessionID = String(resource[markerRange.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
    guard !ownerUserID.isEmpty, !ownerUserID.contains("/"), !authoritySessionID.isEmpty, !authoritySessionID.contains("/") else {
        return nil
    }
    return AuthoritySubject(kind: "session", ownerUserID: ownerUserID, sessionID: authoritySessionID)
}

private func requiredAuthorityBase64(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredAuthorityString(value, field)
    guard Data(base64Encoded: cleaned) != nil else {
        throw invalidAuthority("\(field) must be base64")
    }
    return cleaned
}

private func requiredAuthorityInteger(_ value: JSONValue?, _ field: String) throws -> Int64 {
    guard case let .number(number) = value else {
        throw invalidAuthority("\(field) must be a non-negative integer")
    }
    let integer = Int64(number)
    guard number >= 0, Double(integer) == number else {
        throw invalidAuthority("\(field) must be a non-negative integer")
    }
    return integer
}

private func requiredAuthorityStringArray(_ object: [String: JSONValue], _ field: String) throws -> [String] {
    guard case let .array(values) = object[field], !values.isEmpty else {
        throw invalidAuthority("\(field) must be a non-empty string array")
    }
    return try values.map {
        guard case let .string(value) = $0 else {
            throw invalidAuthority("\(field) must be a non-empty string array")
        }
        return try requiredAuthorityString(value, field)
    }
}

private func requiredAuthorityScopes(_ scopes: [String]) throws -> [String] {
    guard !scopes.isEmpty else {
        throw invalidAuthority("authority scopes are required")
    }
    return try scopes.map { try requiredAuthorityString($0, "scope") }
}

private func requiredAuthorityObject(_ object: [String: JSONValue], _ field: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[field] {
        return value
    }
    throw invalidAuthority("\(field) must be an object")
}

private func invalidAuthority(_ message: String) -> SDKError {
    SDKError.validation(authorityProfile, message)
}

private func encodeAuthorityJSONObject(_ object: [String: JSONValue]) throws -> Data {
    try JSONSerialization.data(
        withJSONObject: object.mapValues(authorityJSONCompatible),
        options: [.sortedKeys]
    )
}

private func authorityJSONCompatible(_ value: JSONValue) -> Any {
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
        return values.map(authorityJSONCompatible)
    case let .object(object):
        return object.mapValues(authorityJSONCompatible)
    }
}
