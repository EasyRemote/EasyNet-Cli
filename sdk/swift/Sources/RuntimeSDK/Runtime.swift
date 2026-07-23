import Foundation

public enum InvocationTerminalState: String, Sendable {
    case completed = "Completed"
    case failed = "Failed"
    case cancelled = "Cancelled"
    case timedOut = "TimedOut"
    case backpressureTerminated = "BackpressureTerminated"
}

public struct InvocationResult: Sendable {
    public let ok: Bool
    public let terminalState: InvocationTerminalState
    public let outputJSON: String
    public let error: SDKError?
    public let terminalReceipt: [String: String]

    public init(
        ok: Bool,
        terminalState: InvocationTerminalState,
        outputJSON: String = "",
        error: SDKError? = nil,
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
        self.terminalReceipt = terminalReceipt
    }

    static func fromJSON(_ raw: Data) throws -> InvocationResult {
        let object = try runtimeJSONObject(raw, "invocation_result")
        if object.keys.contains("receipt") {
            throw SDKError.validation("invocation_result", "retired receipt alias is not accepted")
        }
        let terminalState = try InvocationTerminalState(
            rawValue: runtimeRequiredString(object, "terminal_state", "invocation_result")
        ).unwrap("terminal_state")
        return try InvocationResult(
            ok: try runtimeRequiredBool(object, "ok", "invocation_result"),
            terminalState: terminalState,
            outputJSON: runtimeOptionalJSONObjectString(object["output_json"]),
            error: nil,
            terminalReceipt: try runtimeRequiredTerminalReceipt(object)
        )
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

private func runtimeRequiredTerminalReceipt(_ object: [String: Any]) throws -> [String: String] {
    guard let value = object["terminal_receipt"] else {
        throw SDKError.validation("invocation_result", "terminal_receipt is required")
    }
    guard let map = value as? [String: Any] else {
        throw SDKError.validation("invocation_result", "terminal_receipt must be an object")
    }
    return map.compactMapValues { $0 as? String }
}

private func runtimeOptionalJSONObjectString(_ value: Any?) throws -> String {
    guard let value else {
        return ""
    }
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    return String(data: data, encoding: .utf8) ?? ""
}

private extension Optional {
    func unwrap(_ field: String) throws -> Wrapped {
        guard let value = self else {
            throw SDKError.validation("runtime", "\(field) is invalid")
        }
        return value
    }
}
