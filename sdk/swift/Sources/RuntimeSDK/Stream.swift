public struct StreamEvent: Sendable, Equatable {
    public let sequence: Int
    public let kind: String
    public let state: String
    public let payloadJSON: String
    public let terminal: Bool
    public let transportTerminal: Bool
    public let error: SDKError?

    public init(
        sequence: Int,
        kind: String,
        state: String,
        payloadJSON: String = "",
        terminal: Bool = false,
        transportTerminal: Bool = false,
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
        self.transportTerminal = transportTerminal
        self.error = error
    }

    public static func data(_ sequence: Int, payloadJSON: String) throws -> StreamEvent {
        try StreamEvent(sequence: sequence, kind: "data", state: "Open", payloadJSON: payloadJSON)
    }

    public static func terminal(_ sequence: Int, state: String) throws -> StreamEvent {
        try StreamEvent(sequence: sequence, kind: "terminal", state: state, terminal: true)
    }

    public static func transportTerminal(_ sequence: Int, kind: String, state: String) throws -> StreamEvent {
        try StreamEvent(sequence: sequence, kind: kind, state: state, transportTerminal: true)
    }

    public static func backpressure(_ sequence: Int) throws -> StreamEvent {
        try StreamEvent(
            sequence: sequence,
            kind: "error",
            state: "Failed",
            transportTerminal: true,
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
            && lhs.transportTerminal == rhs.transportTerminal
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
        try StreamEvent.transportTerminal(0, kind: "cancel_requested", state: "CancelRequested")
    }

    func close() async throws {}
}

public final class StreamHandle: @unchecked Sendable {
    public static let maxRetainedEvents = 1024

    private let source: StreamSource
    private var closed = false
    private var retained: [StreamEvent] = []
    private var terminal: StreamEvent?
    private var transportTerminal: StreamEvent?

    public init(source: StreamSource) {
        self.source = source
    }

    public func next() async throws -> StreamEvent {
        try requireOpen()
        if let terminal {
            return terminal
        }
        if let transportTerminal {
            return transportTerminal
        }
        let event = try await source.next()
        try retain(event)
        if event.terminal {
            terminal = event
        } else if event.transportTerminal {
            transportTerminal = event
        }
        return terminal ?? event
    }

    public func cancel(reason: String) async throws -> StreamEvent {
        try requireOpen()
        let event = try await source.cancel(reason: reason)
        try retain(event)
        if event.terminal {
            terminal = event
        } else if event.transportTerminal {
            transportTerminal = event
        }
        return event
    }

    public func retainedEvents() -> [StreamEvent] {
        retained
    }

    public func terminalEvent() -> StreamEvent? {
        terminal
    }

    public func transportTerminalEvent() -> StreamEvent? {
        transportTerminal
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await source.close()
    }

    private func retain(_ event: StreamEvent) throws {
        if retained.count >= StreamHandle.maxRetainedEvents, terminal == nil, transportTerminal == nil {
            let overflow = try StreamEvent.backpressure(event.sequence)
            transportTerminal = overflow
            retained.append(overflow)
            return
        }
        if (terminal == nil && transportTerminal == nil) || event.terminal || event.transportTerminal {
            retained.append(event)
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("stream")
        }
    }
}

public struct StreamEventIterator: AsyncIteratorProtocol {
    private let handle: StreamHandle
    private var finished = false

    init(handle: StreamHandle) {
        self.handle = handle
    }

    public mutating func next() async throws -> StreamEvent? {
        if finished {
            return nil
        }
        let event = try await handle.next()
        if event.terminal {
            finished = true
        }
        return event
    }
}

extension StreamHandle: AsyncSequence {
    public typealias Element = StreamEvent
    public typealias AsyncIterator = StreamEventIterator

    public func makeAsyncIterator() -> StreamEventIterator {
        StreamEventIterator(handle: self)
    }
}
