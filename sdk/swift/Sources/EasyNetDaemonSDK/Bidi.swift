public struct BidiFrame: Sendable, Equatable {
    public let sequence: Int
    public let kind: String
    public let payloadJSON: String
    public let terminal: Bool

    public init(sequence: Int, kind: String, payloadJSON: String = "", terminal: Bool = false) throws {
        guard sequence >= 0 else {
            throw SDKError.validation("bidi", "sequence must be non-negative")
        }
        guard !kind.isEmpty else {
            throw SDKError.validation("bidi", "kind is required")
        }
        self.sequence = sequence
        self.kind = kind
        self.payloadJSON = payloadJSON
        self.terminal = terminal
    }

    public static func data(_ sequence: Int, payloadJSON: String) throws -> BidiFrame {
        try BidiFrame(sequence: sequence, kind: "data", payloadJSON: payloadJSON)
    }

    public static func terminal(_ sequence: Int, kind: String) throws -> BidiFrame {
        try BidiFrame(sequence: sequence, kind: kind, terminal: true)
    }
}

public protocol BidiSource: AnyObject, Sendable {
    func send(_ frame: BidiFrame) async throws
    func next() async throws -> BidiFrame
    func closeSend() async throws -> BidiFrame
    func cancel(reason: String) async throws -> BidiFrame
    func close() async throws
}

public extension BidiSource {
    func closeSend() async throws -> BidiFrame {
        try BidiFrame.terminal(0, kind: "send_closed")
    }

    func cancel(reason: String) async throws -> BidiFrame {
        try BidiFrame.terminal(0, kind: "cancelled")
    }

    func close() async throws {}
}

public final class BidiSession: @unchecked Sendable {
    public static let maxRetainedFrames = 1024

    private let source: BidiSource
    private var closed = false
    private var sendClosed = false
    private var retained: [BidiFrame] = []
    private var terminal: BidiFrame?

    public init(source: BidiSource) {
        self.source = source
    }

    public func send(_ frame: BidiFrame) async throws {
        try requireOpen()
        if sendClosed {
            throw SDKError(
                code: .cancelled,
                stage: "bidi",
                message: "bidi send side is closed",
                details: ["state": "send_closed"]
            )
        }
        if terminal != nil {
            throw SDKError.closed("bidi")
        }
        try await source.send(frame)
    }

    public func next() async throws -> BidiFrame {
        try requireOpen()
        if let terminal {
            return terminal
        }
        let frame = try await source.next()
        try retain(frame)
        if frame.terminal {
            terminal = frame
        }
        return terminal ?? frame
    }

    public func closeSend() async throws -> BidiFrame {
        try requireOpen()
        if sendClosed {
            throw SDKError.closed("bidi_send")
        }
        let frame = try await source.closeSend()
        sendClosed = true
        return frame
    }

    public func cancel(reason: String) async throws -> BidiFrame {
        try requireOpen()
        let frame = try await source.cancel(reason: reason)
        terminal = frame
        try retain(frame)
        return frame
    }

    public func retainedFrames() -> [BidiFrame] {
        retained
    }

    public func terminalFrame() -> BidiFrame? {
        terminal
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await source.close()
    }

    private func retain(_ frame: BidiFrame) throws {
        if retained.count >= BidiSession.maxRetainedFrames, terminal == nil {
            let overflow = try BidiFrame.terminal(frame.sequence, kind: "backpressure_terminated")
            terminal = overflow
            retained.append(overflow)
            return
        }
        if terminal == nil || frame.terminal {
            retained.append(frame)
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("bidi")
        }
    }
}

public struct BidiFrameIterator: AsyncIteratorProtocol {
    private let session: BidiSession
    private var finished = false

    init(session: BidiSession) {
        self.session = session
    }

    public mutating func next() async throws -> BidiFrame? {
        if finished {
            return nil
        }
        let frame = try await session.next()
        if frame.terminal {
            finished = true
        }
        return frame
    }
}

extension BidiSession: AsyncSequence {
    public typealias Element = BidiFrame
    public typealias AsyncIterator = BidiFrameIterator

    public func makeAsyncIterator() -> BidiFrameIterator {
        BidiFrameIterator(session: self)
    }
}
