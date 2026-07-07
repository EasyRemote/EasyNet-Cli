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
    public let receipt: [String: String]

    public init(
        ok: Bool,
        terminalState: InvocationTerminalState,
        outputJSON: String = "",
        error: SDKError? = nil,
        receipt: [String: String] = [:]
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
        self.receipt = receipt
    }
}

public protocol RuntimeTransport: AnyObject, Sendable {
    func invoke(_ draft: InvocationDraft) async throws -> InvocationResult
    func openStream(_ draft: InvocationDraft) async throws -> StreamSource
    func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) async throws -> BidiSource
    func close() async throws
}

public extension RuntimeTransport {
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

    public func openStream(_ draft: InvocationDraft) async throws -> StreamHandle {
        try requireOpen()
        return StreamHandle(source: try await transport.openStream(draft))
    }

    public func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) async throws -> BidiSession {
        try requireOpen()
        return BidiSession(source: try await transport.openBidi(draft, frame0: frame0))
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
}
