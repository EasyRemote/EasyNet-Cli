import CryptoKit
import Foundation

public let hostBindingProfile = "host_binding"
public let hostStreamFrameSchema = "host-stream-frame.schema.json"
public let hostStreamHashAlgorithm = "sha256(prev_hash || seq_be || canonical_json(value))"
public let hostStreamEmptyOutputHash = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

public struct HostStreamBindingRequest: Sendable, Equatable {
    public let bindingID: String
    public let descriptorRef: String
    public let endpoint: String
    public let frameSchema: String
    public let cleanup: [String: JSONValue]
    public let timeoutMS: Int?
    public let readiness: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public init(
        bindingID: String,
        descriptorRef: String,
        endpoint: String,
        frameSchema: String = hostStreamFrameSchema,
        cleanup: [String: JSONValue] = [:],
        timeoutMS: Int? = nil,
        readiness: [String: JSONValue] = [:],
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.bindingID = try requiredHostString(bindingID, "binding_id")
        self.descriptorRef = try requiredHostString(descriptorRef, "descriptor_ref")
        self.endpoint = try cleanHostEndpoint(endpoint)
        self.frameSchema = try cleanHostFrameSchema(frameSchema)
        if let timeoutMS, timeoutMS < 0 {
            throw invalidHostBinding("timeout_ms must be non-negative or null")
        }
        self.cleanup = cleanup
        self.timeoutMS = timeoutMS
        self.readiness = readiness
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "binding_id": .string(bindingID),
            "descriptor_ref": .string(descriptorRef),
            "endpoint": .string(endpoint),
            "frame_schema": .string(frameSchema),
        ]
        if !cleanup.isEmpty { object["cleanup"] = .object(cleanup) }
        if let timeoutMS { object["timeout_ms"] = .number(Double(timeoutMS)) }
        if !readiness.isEmpty { object["readiness"] = .object(readiness) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public struct HostStreamBinding: Sendable, Equatable {
    public let bindingID: String
    public let descriptorRef: String
    public let endpoint: String
    public let frameSchema: String
    public let cleanup: [String: JSONValue]
    public let timeoutMS: Int?
    public let readiness: [String: JSONValue]
    public let lifecycle: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> HostStreamBinding {
        let object = try decodeHostObject(raw, label: "host stream binding JSON")
        let binding = try HostStreamBinding(
            bindingID: requiredHostString(object, "binding_id"),
            descriptorRef: requiredHostString(object, "descriptor_ref"),
            endpoint: requiredHostString(object, "endpoint"),
            frameSchema: requiredHostString(object, "frame_schema"),
            cleanup: requiredHostObject(object, "cleanup"),
            timeoutMS: optionalHostInt(object["timeout_ms"], "timeout_ms"),
            readiness: requiredHostObject(object, "readiness"),
            lifecycle: requiredHostObject(object, "lifecycle"),
            metadata: requiredHostObject(object, "metadata")
        )
        guard !binding.cleanup.isEmpty, !binding.readiness.isEmpty, !binding.lifecycle.isEmpty, !binding.metadata.isEmpty else {
            throw invalidHostBinding("invalid host stream binding projection")
        }
        return binding
    }

    public init(
        bindingID: String,
        descriptorRef: String,
        endpoint: String,
        frameSchema: String,
        cleanup: [String: JSONValue],
        timeoutMS: Int?,
        readiness: [String: JSONValue],
        lifecycle: [String: JSONValue],
        metadata: [String: JSONValue]
    ) throws {
        self.bindingID = try requiredHostString(bindingID, "binding_id")
        self.descriptorRef = try requiredHostString(descriptorRef, "descriptor_ref")
        self.endpoint = try cleanHostEndpoint(endpoint)
        self.frameSchema = try cleanHostFrameSchema(frameSchema)
        if let timeoutMS, timeoutMS < 0 {
            throw invalidHostBinding("timeout_ms must be non-negative or null")
        }
        self.cleanup = cleanup
        self.timeoutMS = timeoutMS
        self.readiness = readiness
        self.lifecycle = lifecycle
        self.metadata = metadata
    }
}

public struct HostStreamEnvelope: Sendable, Equatable {
    public let function: String
    public let args: JSONValue
    public let callID: String
    public let caller: String

    public init(function: String, args: JSONValue, callID: String, caller: String) throws {
        self.function = try requiredHostString(function, "fn")
        self.args = args
        self.callID = try requiredHostString(callID, "call_id")
        self.caller = try requiredHostString(caller, "caller")
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "request": .object([
                "fn": .string(function),
                "args": args,
                "call_id": .string(callID),
                "caller": .string(caller),
            ])
        ])
    }
}

public struct HostStreamRequest: Sendable, Equatable {
    public let function: String
    public let args: JSONValue
    public let callID: String
    public let caller: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> HostStreamRequest {
        let object = try decodeHostObject(raw, label: "host stream request JSON")
        return try HostStreamRequest(
            function: requiredHostString(object, "function"),
            args: object["args"] ?? .null,
            callID: requiredHostString(object, "call_id"),
            caller: requiredHostString(object, "caller"),
            metadata: requiredHostObject(object, "metadata")
        )
    }

    public init(function: String, args: JSONValue, callID: String, caller: String, metadata: [String: JSONValue]) throws {
        self.function = try requiredHostString(function, "function")
        self.args = args
        self.callID = try requiredHostString(callID, "call_id")
        self.caller = try requiredHostString(caller, "caller")
        guard !metadata.isEmpty else {
            throw invalidHostBinding("metadata must be an object")
        }
        self.metadata = metadata
    }
}

public struct HostStreamTerminalSummary: Sendable, Equatable {
    public let outputHash: String
    public let frames: Int
    public let metadata: [String: JSONValue]

    public init(outputHash: String, frames: Int, metadata: [String: JSONValue] = [:]) throws {
        self.outputHash = try cleanHostOutputHash(outputHash, "output_hash")
        guard frames >= 0 else { throw invalidHostBinding("frames must be non-negative") }
        self.frames = frames
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> HostStreamTerminalSummary {
        try fromObject(decodeHostObject(raw, label: "host stream terminal summary JSON"))
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> HostStreamTerminalSummary {
        try HostStreamTerminalSummary(
            outputHash: requiredHostString(object, "output_hash"),
            frames: requiredHostInt(object, "frames"),
            metadata: optionalHostObject(object["metadata"], "metadata") ?? [:]
        )
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "output_hash": .string(outputHash),
            "frames": .number(Double(frames)),
        ]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return object
    }
}

public struct HostStreamFrame: Sendable, Equatable {
    public let frameType: String
    public let seq: Int?
    public let value: JSONValue
    public let error: [String: JSONValue]
    public let terminal: HostStreamTerminalSummary?
    public let outputHash: String?

    public static func fromJSON(_ raw: Data) throws -> HostStreamFrame {
        let object = try decodeHostObject(raw, label: "host stream frame JSON")
        let terminal = try optionalHostObject(object["terminal"], "terminal").map(HostStreamTerminalSummary.fromObject)
        return try HostStreamFrame(
            frameType: requiredHostString(object, "frame_type"),
            seq: optionalHostInt(object["seq"], "seq"),
            value: object["value"] ?? .null,
            error: optionalHostObject(object["error"], "error") ?? [:],
            terminal: terminal,
            outputHash: optionalHostString(object["output_hash"], "output_hash")
        )
    }

    public init(
        frameType: String,
        seq: Int?,
        value: JSONValue = .null,
        error: [String: JSONValue] = [:],
        terminal: HostStreamTerminalSummary? = nil,
        outputHash: String? = nil
    ) throws {
        self.frameType = try requiredHostString(frameType, "frame_type")
        self.seq = seq
        self.value = value
        self.error = error
        self.terminal = terminal
        self.outputHash = outputHash
        switch self.frameType {
        case "item":
            guard let seq, seq >= 0, error.isEmpty, terminal == nil, outputHash == nil else {
                throw invalidHostBinding("invalid item host stream frame")
            }
        case "error":
            guard seq == nil, value == .null, !error.isEmpty, terminal == nil, outputHash == nil else {
                throw invalidHostBinding("invalid error host stream frame")
            }
        case "terminal":
            guard let seq, seq >= 0, value == .null, error.isEmpty, let terminal, outputHash == terminal.outputHash else {
                throw invalidHostBinding("invalid terminal host stream frame")
            }
        default:
            throw invalidHostBinding("unknown host stream frame type")
        }
    }
}

public struct HostStreamHashState: Sendable, Equatable {
    public let algorithm: String
    public let outputHash: String
    public let frames: Int
    public let lastSeq: Int?
    public let canonicalJSON: String

    public init(algorithm: String, outputHash: String, frames: Int, lastSeq: Int?, canonicalJSON: String = "") throws {
        self.algorithm = try requiredHostString(algorithm, "algorithm")
        self.outputHash = try cleanHostOutputHash(outputHash, "output_hash")
        guard frames >= 0 else { throw invalidHostBinding("frames must be non-negative") }
        self.frames = frames
        self.lastSeq = lastSeq
        self.canonicalJSON = try optionalHostCleanString(canonicalJSON, "canonical_json")
        try validateHostHashState(self.algorithm, self.outputHash, self.frames, self.lastSeq)
    }

    public static func initial() throws -> HostStreamHashState {
        try HostStreamHashState(
            algorithm: hostStreamHashAlgorithm,
            outputHash: hostStreamEmptyOutputHash,
            frames: 0,
            lastSeq: nil
        )
    }

    public static func fromJSON(_ raw: Data) throws -> HostStreamHashState {
        let object = try decodeHostObject(raw, label: "host stream hash state JSON")
        return try HostStreamHashState(
            algorithm: requiredHostString(object, "algorithm"),
            outputHash: requiredHostString(object, "output_hash"),
            frames: requiredHostInt(object, "frames"),
            lastSeq: optionalHostInt(object["last_seq"], "last_seq"),
            canonicalJSON: optionalHostString(object["canonical_json"], "canonical_json") ?? ""
        )
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "algorithm": .string(algorithm),
            "output_hash": .string(outputHash),
            "frames": .number(Double(frames)),
            "last_seq": lastSeq == nil ? .null : .number(Double(lastSeq!)),
        ]
        if !canonicalJSON.isEmpty { object["canonical_json"] = .string(canonicalJSON) }
        return object
    }

    public func fold(seq: Int, value: JSONValue) throws -> HostStreamHashState {
        try foldHostStreamHash(state: self, seq: seq, value: value)
    }
}

public enum HostStreamLifecycleState: String, Sendable {
    case declared
    case checking
    case ready
    case notReady = "not_ready"
    case cleaning
    case cleaned
    case failed
    case closed
}

public struct HostStreamReadiness: Sendable, Equatable {
    public let state: String
    public let checked: Bool
    public let endpointReady: Bool?
    public let metadata: [String: JSONValue]

    public init(state: String, checked: Bool, endpointReady: Bool?, metadata: [String: JSONValue] = [:]) throws {
        self.state = try requiredHostString(state, "state")
        self.checked = checked
        self.endpointReady = endpointReady
        self.metadata = metadata
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> HostStreamReadiness {
        var metadata = object
        metadata.removeValue(forKey: "state")
        metadata.removeValue(forKey: "checked")
        metadata.removeValue(forKey: "endpoint_ready")
        return try HostStreamReadiness(
            state: requiredHostString(object, "state"),
            checked: requiredHostBool(object, "checked"),
            endpointReady: optionalHostBool(object["endpoint_ready"], "endpoint_ready"),
            metadata: metadata
        )
    }
}

public struct HostStreamCleanup: Sendable, Equatable {
    public let mode: String
    public let metadata: [String: JSONValue]

    public init(mode: String, metadata: [String: JSONValue] = [:]) throws {
        self.mode = try optionalHostCleanString(mode, "mode")
        self.metadata = metadata
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> HostStreamCleanup {
        var metadata = object
        metadata.removeValue(forKey: "mode")
        return try HostStreamCleanup(
            mode: optionalHostString(object["mode"], "mode") ?? "",
            metadata: metadata
        )
    }
}

public protocol HostStreamLifecycleProvider: AnyObject, Sendable {
    func checkReadiness(_ binding: HostStreamBinding) async throws -> HostStreamReadiness
    func cleanup(_ binding: HostStreamBinding) async throws -> HostStreamCleanup
}

public final class HostStreamLifecycleController: @unchecked Sendable {
    public let binding: HostStreamBinding
    private let provider: HostStreamLifecycleProvider
    public private(set) var state: HostStreamLifecycleState = .declared
    public private(set) var readiness: HostStreamReadiness?
    public private(set) var cleanupResult: HostStreamCleanup?

    public init(binding: HostStreamBinding, provider: HostStreamLifecycleProvider) {
        self.binding = binding
        self.provider = provider
        self.readiness = try? HostStreamReadiness.fromObject(binding.readiness)
    }

    public func checkReadiness() async throws -> HostStreamReadiness {
        if [.cleaning, .cleaned, .closed].contains(state) {
            throw invalidHostBinding("host stream lifecycle is not readable")
        }
        state = .checking
        do {
            let result = try await provider.checkReadiness(binding)
            readiness = result
            state = result.endpointReady == true ? .ready : .notReady
            return result
        } catch let error as SDKError {
            state = .failed
            throw error
        } catch {
            state = .failed
            throw hostTransport("host binding readiness provider failed", error)
        }
    }

    public func cleanup() async throws -> HostStreamCleanup {
        if state == .cleaned || state == .closed {
            if let cleanupResult {
                return cleanupResult
            }
            return try HostStreamCleanup.fromObject(binding.cleanup)
        }
        if state == .cleaning {
            throw invalidHostBinding("host stream lifecycle cleanup is already running")
        }
        if state == .checking {
            throw invalidHostBinding("host stream lifecycle readiness check is running")
        }
        state = .cleaning
        do {
            let result = try await provider.cleanup(binding)
            cleanupResult = result
            state = .cleaned
            return result
        } catch let error as SDKError {
            state = .failed
            throw error
        } catch {
            state = .failed
            throw hostTransport("host binding cleanup provider failed", error)
        }
    }

    public func close() async throws {
        guard state != .closed else { return }
        _ = try await cleanup()
        state = .closed
    }
}

public protocol HostBindingTransport: AnyObject, Sendable {
    func buildHostStreamBinding(_ requestJSON: Data) async throws -> Data
    func decodeRequest(_ envelopeJSON: Data) async throws -> Data
    func encodeItem(_ requestJSON: Data) async throws -> Data
    func encodeError(_ requestJSON: Data) async throws -> Data
    func encodeTerminal(_ requestJSON: Data) async throws -> Data
    func foldOutputHash(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension HostBindingTransport {
    func buildHostStreamBinding(_ requestJSON: Data) async throws -> Data { throw hostUnsupported("host binding build transport is not available") }
    func decodeRequest(_ envelopeJSON: Data) async throws -> Data { throw hostUnsupported("host binding decode transport is not available") }
    func encodeItem(_ requestJSON: Data) async throws -> Data { throw hostUnsupported("host binding encode item transport is not available") }
    func encodeError(_ requestJSON: Data) async throws -> Data { throw hostUnsupported("host binding encode error transport is not available") }
    func encodeTerminal(_ requestJSON: Data) async throws -> Data { throw hostUnsupported("host binding encode terminal transport is not available") }
    func foldOutputHash(_ requestJSON: Data) async throws -> Data { throw hostUnsupported("host binding hash transport is not available") }
    func close() async throws {}
}

public final class HostBindingClient: @unchecked Sendable {
    private let transport: HostBindingTransport
    private let lifecycleProvider: HostStreamLifecycleProvider?
    private var closed = false

    public init(transport: HostBindingTransport, lifecycleProvider: HostStreamLifecycleProvider? = nil) {
        self.transport = transport
        self.lifecycleProvider = lifecycleProvider
    }

    public func buildHostStreamBinding(_ request: HostStreamBindingRequest) async throws -> HostStreamBinding {
        try await HostStreamBinding.fromJSON(raw { try await transport.buildHostStreamBinding(request.jsonData()) })
    }

    public func decodeRequest(_ envelope: HostStreamEnvelope) async throws -> HostStreamRequest {
        try await HostStreamRequest.fromJSON(raw { try await transport.decodeRequest(envelope.jsonData()) })
    }

    public func encodeItem(seq: Int, value: JSONValue) async throws -> HostStreamFrame {
        guard seq >= 0 else { throw invalidHostBinding("seq must be non-negative") }
        return try await HostStreamFrame.fromJSON(raw {
            try await transport.encodeItem(encodeJSONObject(["seq": .number(Double(seq)), "value": value]))
        })
    }

    public func encodeError(_ error: SDKError) async throws -> HostStreamFrame {
        try await HostStreamFrame.fromJSON(raw {
            try await transport.encodeError(encodeJSONObject(["error": .object(errorObject(error))]))
        })
    }

    public func encodeTerminal(_ summary: HostStreamTerminalSummary) async throws -> HostStreamFrame {
        try await HostStreamFrame.fromJSON(raw {
            try await transport.encodeTerminal(encodeJSONObject(["summary": .object(summary.jsonObject())]))
        })
    }

    public func foldOutputHash(state: HostStreamHashState, seq: Int, value: JSONValue) async throws -> HostStreamHashState {
        guard seq >= 0 else { throw invalidHostBinding("seq must be non-negative") }
        try validateHostHashFold(state, seq)
        return try await HostStreamHashState.fromJSON(raw {
            try await transport.foldOutputHash(encodeJSONObject([
                "state": .object(state.jsonObject()),
                "seq": .number(Double(seq)),
                "value": value,
            ]))
        })
    }

    public func foldOutputHashLocal(state: HostStreamHashState, seq: Int, value: JSONValue) throws -> HostStreamHashState {
        try foldHostStreamHash(state: state, seq: seq, value: value)
    }

    public func openLifecycle(_ binding: HostStreamBinding, provider: HostStreamLifecycleProvider? = nil) throws -> HostStreamLifecycleController {
        try requireOpen()
        guard let resolved = provider ?? lifecycleProvider else {
            throw invalidHostBinding("host stream lifecycle provider is required")
        }
        return HostStreamLifecycleController(binding: binding, provider: resolved)
    }

    public func checkReadiness(_ binding: HostStreamBinding, provider: HostStreamLifecycleProvider? = nil) async throws -> HostStreamReadiness {
        try await openLifecycle(binding, provider: provider).checkReadiness()
    }

    public func cleanup(_ binding: HostStreamBinding, provider: HostStreamLifecycleProvider? = nil) async throws -> HostStreamCleanup {
        try await openLifecycle(binding, provider: provider).cleanup()
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
            throw hostTransport("host binding transport failed", error)
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("host binding")
        }
    }

    private func errorObject(_ error: SDKError) -> [String: JSONValue] {
        var details: [String: JSONValue] = [:]
        for (key, value) in error.details {
            details[key] = .string(value)
        }
        return [
            "code": .string(error.code.rawValue),
            "stage": .string(error.stage),
            "message": .string(error.message),
            "retry": .string(error.retryHint.rawValue),
            "source": error.source.isEmpty ? .null : .string(error.source),
            "invocation_id": error.invocationID.isEmpty ? .null : .string(error.invocationID),
            "receipt_ura": error.receiptURA.isEmpty ? .null : .string(error.receiptURA),
            "details": .object(details),
        ]
    }
}

private func foldHostStreamHash(state: HostStreamHashState, seq: Int, value: JSONValue) throws -> HostStreamHashState {
    guard seq >= 0 else { throw invalidHostBinding("seq must be non-negative") }
    try validateHostHashFold(state, seq)
    let canonicalJSON = String(decoding: try encodeJSONValue(value), as: UTF8.self)
    var payload = try Data(hostOutputHash: state.outputHash)
    var bigEndian = UInt64(seq).bigEndian
    withUnsafeBytes(of: &bigEndian) { payload.append(contentsOf: $0) }
    payload.append(Data(canonicalJSON.utf8))
    let digest = SHA256.hash(data: payload)
    return try HostStreamHashState(
        algorithm: hostStreamHashAlgorithm,
        outputHash: "sha256:" + digest.map { String(format: "%02x", $0) }.joined(),
        frames: state.frames + 1,
        lastSeq: seq,
        canonicalJSON: canonicalJSON
    )
}

private func encodeJSONValue(_ value: JSONValue) throws -> Data {
    try JSONSerialization.data(withJSONObject: jsonCompatible(value), options: [.sortedKeys])
}

private func decodeHostObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidHostBinding("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

private func requiredHostString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty { return value }
    throw invalidHostBinding("\(name) must be a non-empty string")
}

private func requiredHostString(_ value: String, _ name: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidHostBinding("\(name) is required")
    }
    return value
}

private func optionalHostCleanString(_ value: String, _ name: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidHostBinding("\(name) must not contain surrounding whitespace")
    }
    return value
}

private func optionalHostString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .string(string): return string
    default: throw invalidHostBinding("\(name) must be a string or null")
    }
}

private func requiredHostBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] { return value }
    throw invalidHostBinding("\(name) must be a boolean")
}

private func optionalHostBool(_ value: JSONValue?, _ name: String) throws -> Bool? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .bool(bool): return bool
    default: throw invalidHostBinding("\(name) must be a boolean or null")
    }
}

private func requiredHostInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if let value = try optionalHostInt(object[name], name) { return value }
    throw invalidHostBinding("\(name) must be a non-negative integer")
}

private func optionalHostInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .number(number) where number >= 0 && number.rounded() == number && number <= Double(Int.max):
        return Int(number)
    default:
        throw invalidHostBinding("\(name) must be a non-negative integer or null")
    }
}

private func requiredHostObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] { return value }
    throw invalidHostBinding("\(name) must be an object")
}

private func optionalHostObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .object(object): return object
    default: throw invalidHostBinding("\(name) must be an object or null")
    }
}

private func cleanHostEndpoint(_ value: String) throws -> String {
    let endpoint = try requiredHostString(value, "endpoint")
    guard endpoint.hasPrefix("/") || endpoint.hasPrefix("unix:///") else {
        throw invalidHostBinding("host stream endpoint must be absolute")
    }
    return endpoint
}

private func cleanHostFrameSchema(_ value: String) throws -> String {
    let frameSchema = try requiredHostString(value, "frame_schema")
    guard frameSchema == hostStreamFrameSchema else {
        throw invalidHostBinding("frame_schema must be host-stream-frame.schema.json")
    }
    return frameSchema
}

private func cleanHostOutputHash(_ value: String, _ name: String) throws -> String {
    let hash = try requiredHostString(value, name)
    let pattern = #"^sha256:[0-9a-f]{64}$"#
    if hash.range(of: pattern, options: .regularExpression) == nil {
        throw invalidHostBinding("\(name) must use sha256:<64 lowercase hex> form")
    }
    return hash
}

private func validateHostHashState(_ algorithm: String, _ outputHash: String, _ frames: Int, _ lastSeq: Int?) throws {
    guard algorithm == hostStreamHashAlgorithm else { throw invalidHostBinding("invalid host stream hash algorithm") }
    _ = try cleanHostOutputHash(outputHash, "output_hash")
    guard frames >= 0 else { throw invalidHostBinding("frames must be non-negative") }
    if frames == 0 {
        if lastSeq != nil {
            throw invalidHostBinding("host stream hash state cannot have last_seq when frames is zero")
        }
        return
    }
    guard lastSeq == frames - 1 else {
        throw invalidHostBinding("host stream hash state last_seq must match frames")
    }
}

private func validateHostHashFold(_ state: HostStreamHashState, _ seq: Int) throws {
    try validateHostHashState(state.algorithm, state.outputHash, state.frames, state.lastSeq)
    guard seq == state.frames else {
        throw invalidHostBinding("host stream hash sequence gap")
    }
}

private func invalidHostBinding(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: hostBindingProfile, message: message, details: ["profile": hostBindingProfile])
}

private func hostUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": hostBindingProfile])
}

private func hostTransport(_ message: String, _ error: Error) -> SDKError {
    if let sdkError = error as? SDKError { return sdkError }
    return SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: message, details: ["profile": hostBindingProfile])
}

private extension Data {
    init(hostOutputHash value: String) throws {
        let hash = try cleanHostOutputHash(value, "output_hash")
        let hex = String(hash.dropFirst("sha256:".count))
        var bytes: [UInt8] = []
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else {
                throw invalidHostBinding("output_hash must use sha256:<64 lowercase hex> form")
            }
            bytes.append(byte)
            index = next
        }
        self.init(bytes)
    }
}
