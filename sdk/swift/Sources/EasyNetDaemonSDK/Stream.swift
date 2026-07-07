public struct StreamEvent: Sendable, Equatable {
    public let sequence: Int
    public let kind: String
    public let state: String
    public let payloadJSON: String
    public let terminal: Bool
    public let error: SDKError?

    public init(
        sequence: Int,
        kind: String,
        state: String,
        payloadJSON: String = "",
        terminal: Bool = false,
        error: SDKError? = nil
    ) throws {
        guard sequence >= 0 else {
            throw SDKError.validation("stream", "sequence must be non-negative")
        }
        guard !kind.isEmpty else {
            throw SDKError.validation("stream", "kind is required")
        }
        guard !state.isEmpty else {
            throw SDKError.validation("stream", "state is required")
        }
        self.sequence = sequence
        self.kind = kind
        self.state = state
        self.payloadJSON = payloadJSON
        self.terminal = terminal
        self.error = error
    }

    public static func data(_ sequence: Int, payloadJSON: String) throws -> StreamEvent {
        try StreamEvent(sequence: sequence, kind: "data", state: "Open", payloadJSON: payloadJSON)
    }

    public static func terminal(_ sequence: Int, state: String) throws -> StreamEvent {
        try StreamEvent(sequence: sequence, kind: "terminal", state: state, terminal: true)
    }

    public static func backpressure(_ sequence: Int) throws -> StreamEvent {
        try StreamEvent(
            sequence: sequence,
            kind: "terminal",
            state: "BackpressureTerminated",
            terminal: true,
            error: SDKError(
                code: .transport,
                stage: "stream",
                retryHint: .safe,
                retryable: true,
                message: "stream retained history exceeded bounded capacity",
                details: ["terminal_state": "backpressure"]
            )
        )
    }
}

extension StreamEvent {
    public static func == (lhs: StreamEvent, rhs: StreamEvent) -> Bool {
        lhs.sequence == rhs.sequence
            && lhs.kind == rhs.kind
            && lhs.state == rhs.state
            && lhs.payloadJSON == rhs.payloadJSON
            && lhs.terminal == rhs.terminal
            && lhs.error?.code == rhs.error?.code
    }
}

public protocol StreamSource: AnyObject, Sendable {
    func next() async throws -> StreamEvent
    func cancel(reason: String) async throws -> StreamEvent
    func close() async throws
}

public extension StreamSource {
    func cancel(reason: String) async throws -> StreamEvent {
        try StreamEvent.terminal(0, state: "Cancelled")
    }

    func close() async throws {}
}

public final class StreamHandle: @unchecked Sendable {
    public static let maxRetainedEvents = 1024

    private let source: StreamSource
    private var closed = false
    private var retained: [StreamEvent] = []
    private var terminal: StreamEvent?

    public init(source: StreamSource) {
        self.source = source
    }

    public func next() async throws -> StreamEvent {
        try requireOpen()
        if let terminal {
            return terminal
        }
        let event = try await source.next()
        try retain(event)
        if event.terminal {
            terminal = event
        }
        return terminal ?? event
    }

    public func cancel(reason: String) async throws -> StreamEvent {
        try requireOpen()
        let event = try await source.cancel(reason: reason)
        terminal = event
        try retain(event)
        return event
    }

    public func retainedEvents() -> [StreamEvent] {
        retained
    }

    public func terminalEvent() -> StreamEvent? {
        terminal
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await source.close()
    }

    private func retain(_ event: StreamEvent) throws {
        if retained.count >= StreamHandle.maxRetainedEvents, terminal == nil {
            let overflow = try StreamEvent.backpressure(event.sequence)
            terminal = overflow
            retained.append(overflow)
            return
        }
        if terminal == nil || event.terminal {
            retained.append(event)
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("stream")
        }
    }
}
