import Foundation

public let receiptProfile = "receipt"
public let receiptFetchAbility = "invocation.history.get"

public struct ReceiptFetchRequest: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let descriptorRef: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let invocationURA: String
    public let requestID: String
    public let traceID: String
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        descriptorRef: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        invocationURA: String = "",
        requestID: String = "",
        traceID: String = "",
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try cleanReceiptString(callerURA, "caller_ura")
        self.calleeURA = try cleanReceiptString(calleeURA, "callee_ura")
        self.descriptorRef = try cleanReceiptString(descriptorRef, "descriptor_ref")
        self.subjectURA = try cleanReceiptString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanReceiptString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanReceiptString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidReceipt("causal_context is required")
        }
        self.causalContext = causalContext
        self.invocationURA = try optionalReceiptString(invocationURA, "invocation_ura")
        self.requestID = try optionalReceiptString(requestID, "request_id")
        self.traceID = try optionalReceiptString(traceID, "trace_id")
        self.metadata = metadata
        let selectors = [self.invocationURA, self.requestID, self.traceID].filter { !$0.isEmpty }.count
        guard selectors == 1 else {
            throw invalidReceipt("exactly one receipt fetch selector is required")
        }
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "caller_ura": .string(callerURA),
            "callee_ura": .string(calleeURA),
            "descriptor_ref": .string(descriptorRef),
            "subject_ura": .string(subjectURA),
            "descriptor_version": .string(descriptorVersion),
            "nonce_base64": .string(nonceBase64),
            "causal_context": .object(causalContext),
        ]
        if !invocationURA.isEmpty { object["invocation_ura"] = .string(invocationURA) }
        if !requestID.isEmpty { object["request_id"] = .string(requestID) }
        if !traceID.isEmpty { object["trace_id"] = .string(traceID) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return object
    }

    func jsonData() throws -> Data {
        try encodeJSONObject(jsonObject())
    }
}

public struct ReceiptSummary: Sendable, Equatable {
    public let receiptURA: String
    public let invocationID: String
    public let state: String
    public let verified: Bool
    public let output: JSONValue
    public let error: JSONValue
    public let causalRef: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> ReceiptSummary {
        let object = try decodeReceiptObject(raw, label: "receipt summary JSON")
        guard let output = object["output"] else {
            throw invalidReceipt("output is required")
        }
        return try ReceiptSummary(
            receiptURA: optionalReceiptJSONString(object["receipt_ura"], "receipt_ura") ?? "",
            invocationID: optionalReceiptJSONString(object["invocation_id"], "invocation_id") ?? "",
            state: requiredReceiptString(object, "state"),
            verified: requiredReceiptBool(object, "verified"),
            output: output,
            error: object["error"] ?? .null,
            causalRef: optionalReceiptJSONString(object["causal_ref"], "causal_ref") ?? "",
            metadata: optionalReceiptObject(object["metadata"], "metadata") ?? [:]
        )
    }

    public func summaryVerification() throws -> ReceiptVerification {
        try ReceiptVerification(
            verified: false,
            method: "summary-only",
            receiptURA: receiptURA,
            invocationID: invocationID,
            reason: "summary projection is not cryptographic evidence",
            metadata: ["profile": .string(receiptProfile)]
        )
    }
}

public struct ReceiptVerification: Sendable, Equatable {
    public let verified: Bool
    public let method: String
    public let receiptURA: String
    public let invocationID: String
    public let reason: String
    public let metadata: [String: JSONValue]

    public init(
        verified: Bool,
        method: String,
        receiptURA: String = "",
        invocationID: String = "",
        reason: String = "",
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.verified = verified
        self.method = try cleanReceiptString(method, "method")
        self.receiptURA = receiptURA
        self.invocationID = invocationID
        self.reason = reason
        self.metadata = metadata
        if verified && self.method == "summary-only" {
            throw invalidReceipt("summary-only projection cannot be verified")
        }
    }

    public var isCryptographic: Bool {
        guard verified else { return false }
        let normalized = method.trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "_", with: "-")
        let assurance = optionalPlainString(metadata["assurance"])
        return normalized.hasPrefix("axon-")
            || normalized == "full-receipt"
            || normalized == "full-receipt-verification"
            || normalized == "cryptographic"
            || assurance == "cryptographic"
            || assurance == "axon-cryptographic"
    }

    @discardableResult
    public func requireCryptographic() throws -> ReceiptVerification {
        guard isCryptographic else {
            throw invalidReceipt("receipt verification is not Axon-backed cryptographic evidence")
        }
        return self
    }

    public static func fromJSON(_ raw: Data) throws -> ReceiptVerification {
        let object = try decodeReceiptObject(raw, label: "receipt verification JSON")
        return try ReceiptVerification(
            verified: requiredReceiptBool(object, "verified"),
            method: requiredReceiptString(object, "method"),
            receiptURA: optionalReceiptJSONString(object["receipt_ura"], "receipt_ura") ?? "",
            invocationID: optionalReceiptJSONString(object["invocation_id"], "invocation_id") ?? "",
            reason: optionalReceiptJSONString(object["reason"], "reason") ?? "",
            metadata: optionalReceiptObject(object["metadata"], "metadata") ?? [:]
        )
    }
}

public struct ReceiptRef: Sendable, Equatable {
    public let receiptURA: String
    public let receiptHashHex: String
    public let invocationID: String
    public let prevReceiptHashHex: String
    public let index: Int
    public let metadata: [String: JSONValue]

    public init(
        receiptURA: String,
        receiptHashHex: String,
        invocationID: String = "",
        prevReceiptHashHex: String = "",
        index: Int = -1,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.receiptURA = try cleanReceiptString(receiptURA, "receipt_ura")
        self.receiptHashHex = try normalizeReceiptHash(receiptHashHex, "receipt_hash_hex")
        self.invocationID = invocationID
        self.prevReceiptHashHex = prevReceiptHashHex.isEmpty
            ? ""
            : try normalizeReceiptHash(prevReceiptHashHex, "prev_receipt_hash_hex")
        guard index >= -1 else {
            throw invalidReceipt("index must be non-negative")
        }
        self.index = index
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> ReceiptRef {
        let object = try decodeReceiptObject(raw, label: "receipt ref JSON")
        return try ReceiptRef(
            receiptURA: requiredReceiptString(object, "receipt_ura"),
            receiptHashHex: requiredReceiptString(object, "receipt_hash_hex"),
            invocationID: optionalReceiptJSONString(object["invocation_id"], "invocation_id") ?? "",
            prevReceiptHashHex: optionalReceiptJSONString(object["prev_receipt_hash_hex"], "prev_receipt_hash_hex") ?? "",
            index: optionalReceiptInt(object["index"], "index") ?? -1,
            metadata: optionalReceiptObject(object["metadata"], "metadata") ?? [:]
        )
    }

    public static func fromSummary(_ summary: ReceiptSummary) throws -> ReceiptRef {
        guard !summary.receiptURA.isEmpty else {
            throw invalidReceipt("receipt_ura is required")
        }
        throw invalidReceipt("receipt_hash_hex is required")
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "receipt_ura": .string(receiptURA),
            "receipt_hash_hex": .string(receiptHashHex),
        ]
        if !invocationID.isEmpty { object["invocation_id"] = .string(invocationID) }
        if !prevReceiptHashHex.isEmpty { object["prev_receipt_hash_hex"] = .string(prevReceiptHashHex) }
        if index >= 0 { object["index"] = .number(Double(index)) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "receipt_ura": .string(receiptURA),
            "receipt_hash_hex": .string(receiptHashHex),
        ]
        if !invocationID.isEmpty { object["invocation_id"] = .string(invocationID) }
        if !prevReceiptHashHex.isEmpty { object["prev_receipt_hash_hex"] = .string(prevReceiptHashHex) }
        if index >= 0 { object["index"] = .number(Double(index)) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return object
    }
}

public struct ReceiptChain: Sendable, Equatable {
    public let receipts: [ReceiptRef]
    public let metadata: [String: JSONValue]

    public init(receipts: [ReceiptRef], metadata: [String: JSONValue] = [:]) throws {
        guard !receipts.isEmpty else {
            throw invalidReceipt("receipt chain requires at least one receipt")
        }
        self.receipts = receipts
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "receipts": .array(receipts.map { .object($0.jsonObject()) }),
            "metadata": .object(metadata),
        ])
    }
}

public struct ReceiptChainVerification: Sendable, Equatable {
    public let verified: Bool
    public let method: String
    public let rootReceiptURA: String
    public let terminalReceiptURA: String
    public let items: [JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> ReceiptChainVerification {
        let object = try decodeReceiptObject(raw, label: "receipt chain verification JSON")
        guard case let .array(items) = object["items"] else {
            throw invalidReceipt("items must be an array")
        }
        return try ReceiptChainVerification(
            verified: requiredReceiptBool(object, "verified"),
            method: requiredReceiptString(object, "method"),
            rootReceiptURA: optionalReceiptJSONString(object["root_receipt_ura"], "root_receipt_ura") ?? "",
            terminalReceiptURA: optionalReceiptJSONString(
                object["terminal_receipt_ura"], "terminal_receipt_ura"
            ) ?? "",
            items: items,
            metadata: optionalReceiptObject(object["metadata"], "metadata") ?? [:]
        )
    }
}

public protocol ReceiptTransport: AnyObject, Sendable {
    func fetch(_ requestJSON: Data) async throws -> Data
    func project(_ receiptJSON: Data) async throws -> Data
    func verify(_ receiptJSON: Data) async throws -> Data
    func verifyChain(_ requestJSON: Data) async throws -> Data
    func causalRef(_ receiptJSON: Data) async throws -> Data
    func close() async throws
}

public extension ReceiptTransport {
    func fetch(_ requestJSON: Data) async throws -> Data { throw receiptUnsupported("receipt fetch transport is not available") }
    func project(_ receiptJSON: Data) async throws -> Data { throw receiptUnsupported("receipt projection transport is not available") }
    func verify(_ receiptJSON: Data) async throws -> Data { throw receiptUnsupported("receipt verification transport is not available") }
    func verifyChain(_ requestJSON: Data) async throws -> Data {
        throw receiptUnsupported("receipt chain verification transport is not available")
    }
    func causalRef(_ receiptJSON: Data) async throws -> Data { throw receiptUnsupported("receipt causal-ref transport is not available") }
    func close() async throws {}
}

public final class ReceiptClient: @unchecked Sendable {
    private let transport: ReceiptTransport
    private var closed = false

    public init(transport: ReceiptTransport) {
        self.transport = transport
    }

    public func fetch(_ request: ReceiptFetchRequest) async throws -> ReceiptSummary {
        let data = try await raw { try await transport.fetch(request.jsonData()) }
        return try ReceiptSummary.fromJSON(data)
    }

    public func buildFetchInvocation(_ request: ReceiptFetchRequest) throws -> [String: JSONValue] {
        var metadata = request.metadata
        metadata["profile"] = .string(receiptProfile)
        metadata["system_ability"] = .string(receiptFetchAbility)
        metadata["carrier_owner"] = .string("daemon_sdk")
        return [
            "caller_ura": .string(request.callerURA),
            "callee_ura": .string(request.calleeURA),
            "descriptor_ref": .string(request.descriptorRef),
            "subject_ura": .string(request.subjectURA),
            "nonce_base64": .string(request.nonceBase64),
            "causal_context": .object(request.causalContext),
            "args": .object(["key": .object(selector(request))]),
            "content_type": .string("application/json"),
            "metadata": .object(metadata),
        ]
    }

    public func project(_ receiptJSON: Data) async throws -> ReceiptSummary {
        let data = try await raw { try await transport.project(receiptJSON) }
        return try ReceiptSummary.fromJSON(data)
    }

    public func verify(_ receiptJSON: Data) async throws -> ReceiptVerification {
        let data = try await raw { try await transport.verify(receiptJSON) }
        return try ReceiptVerification.fromJSON(data)
    }

    public func verifySummary(_ summary: ReceiptSummary) throws -> ReceiptVerification {
        try summary.summaryVerification()
    }

    public func verifyChain(_ chain: ReceiptChain) async throws -> ReceiptChainVerification {
        let data = try await raw { try await transport.verifyChain(chain.jsonData()) }
        return try ReceiptChainVerification.fromJSON(data)
    }

    public func causalRef(_ ref: ReceiptRef) async throws -> [String: JSONValue] {
        let data = try await raw { try await transport.causalRef(ref.jsonData()) }
        return try decodeReceiptObject(data, label: "receipt causal ref JSON")
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "receipt transport failed",
                details: ["profile": receiptProfile]
            )
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("receipt")
        }
    }
}

private func selector(_ request: ReceiptFetchRequest) -> [String: JSONValue] {
    if !request.invocationURA.isEmpty {
        return ["invocation_ura": .string(request.invocationURA)]
    }
    if !request.requestID.isEmpty {
        return ["request_id": .string(request.requestID)]
    }
    return ["trace_id": .string(request.traceID)]
}

private func decodeReceiptObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidReceipt("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

private func requiredReceiptString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty {
        return value
    }
    throw invalidReceipt("\(name) must be a non-empty string")
}

private func requiredReceiptBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidReceipt("\(name) must be a boolean")
}

private func optionalReceiptJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidReceipt("\(name) must be a string or null")
    }
}

private func optionalReceiptObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidReceipt("\(name) must be an object or null")
    }
}

private func optionalReceiptInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .number(number):
        guard number >= 0, number.rounded() == number, number <= Double(Int.max) else {
            throw invalidReceipt("\(name) must be a non-negative integer")
        }
        return Int(number)
    default:
        throw invalidReceipt("\(name) must be a non-negative integer or null")
    }
}

private func optionalPlainString(_ value: JSONValue?) -> String {
    guard case let .string(string) = value else {
        return ""
    }
    return string
}

private func cleanReceiptString(_ value: String, _ field: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidReceipt("\(field) is required")
    }
    return value
}

private func optionalReceiptString(_ value: String, _ field: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidReceipt("\(field) must not contain surrounding whitespace")
    }
    return value
}

private func normalizeReceiptHash(_ value: String, _ field: String) throws -> String {
    let hash = try cleanReceiptString(value, field)
    guard hash.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else {
        throw invalidReceipt("\(field) must be 64 lowercase hex characters")
    }
    return hash
}

private func invalidReceipt(_ message: String) -> SDKError {
    SDKError(
        code: .invalidArgument,
        stage: receiptProfile,
        message: message,
        details: ["profile": receiptProfile]
    )
}

private func receiptUnsupported(_ message: String) -> SDKError {
    SDKError(
        code: .notImplemented,
        stage: "transport",
        message: message,
        details: ["profile": receiptProfile]
    )
}
