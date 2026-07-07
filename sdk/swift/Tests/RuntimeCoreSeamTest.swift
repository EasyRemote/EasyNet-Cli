import Foundation

@main
struct RuntimeCoreSeamTest {
    static func main() async throws {
        try await featureDiscoveryAndTypedErrors()
        try await completeInvocationDraftAndRuntimeDispatch()
        try await streamHistoryIsBounded()
        try await bidiHistoryIsBounded()
    }

    static func featureDiscoveryAndTypedErrors() async throws {
        let transport = MemoryDiscoveryTransport()
        let client = Client(transport: transport)
        let features = try await client.requireABI(4)
        check(features.profiles["runtime_core"] == "seam", "feature profile")
        try await client.close()
        await expectSDKError(.invalidHandle) {
            _ = try await client.featureDiscovery()
        }
        check(SDKError.validation("x", "bad").errorClass == .validation, "error class")
    }

    static func completeInvocationDraftAndRuntimeDispatch() async throws {
        let runtime = RuntimeClient(transport: MemoryRuntimeTransport())
        let draft = try runtime.newInvocation()
            .withCallerURA("easynet:///r/example/agent/alice")
            .withCalleeURA("easynet:///r/example/agent/bob")
            .withDescriptorRef("easynet:///r/example/ability/bob.echo@1.0.0")
            .withSubjectURA("easynet:///r/example/resource/message")
            .withNonce("n-1")
            .withCausalContext("root")
            .withArgsJSON("{\"text\":\"hi\"}")
            .build()
        check(draft.inspectTuple().descriptorRef.hasSuffix("@1.0.0"), "descriptor preserved")
        let result = try await runtime.invoke(draft)
        check(result.terminalState == .completed, "invoke result")
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationBuilder().withCallerURA("x").build()
        }
        try await runtime.close()
        expectSyncSDKError(.invalidHandle) {
            _ = try runtime.newInvocation()
        }
    }

    static func streamHistoryIsBounded() async throws {
        let handle = StreamHandle(source: QueueStreamSource(count: StreamHandle.maxRetainedEvents + 2))
        for _ in 0..<(StreamHandle.maxRetainedEvents + 2) {
            _ = try await handle.next()
        }
        check(handle.terminalEvent()?.error?.code == .transport, "stream typed overflow")
        check(handle.retainedEvents().count == StreamHandle.maxRetainedEvents + 1, "stream bound")
        try await handle.close()
    }

    static func bidiHistoryIsBounded() async throws {
        let session = BidiSession(source: QueueBidiSource(count: BidiSession.maxRetainedFrames + 2))
        try await session.send(.data(0, payloadJSON: "{\"hello\":true}"))
        for _ in 0..<(BidiSession.maxRetainedFrames + 2) {
            _ = try await session.next()
        }
        check(session.terminalFrame()?.kind == "backpressure_terminated", "bidi overflow")
        check(session.retainedFrames().count == BidiSession.maxRetainedFrames + 1, "bidi bound")
        try await session.close()
    }

    static func expectSyncSDKError(_ code: SDKErrorCode, _ action: () throws -> Void) {
        do {
            try action()
        } catch let error as SDKError {
            check(error.code == code, "expected \(code), got \(error.code)")
            return
        } catch {
            fatalError("expected SDKError, got \(error)")
        }
        fatalError("expected SDKError \(code)")
    }

    static func expectSDKError(_ code: SDKErrorCode, _ action: () async throws -> Void) async {
        do {
            try await action()
        } catch let error as SDKError {
            check(error.code == code, "expected \(code), got \(error.code)")
            return
        } catch {
            fatalError("expected SDKError, got \(error)")
        }
        fatalError("expected SDKError \(code)")
    }

    static func check(_ condition: Bool, _ message: String) {
        if !condition {
            fatalError(message)
        }
    }
}

final class MemoryDiscoveryTransport: DiscoveryTransport, @unchecked Sendable {
    var closed = false

    func featureDiscovery() async throws -> FeatureSet {
        if closed {
            throw SDKError.closed("discovery")
        }
        return try FeatureSet(abiVersion: 4, sdkVersion: "0.0.0-seam", profiles: ["runtime_core": "seam"])
    }

    func close() async throws {
        closed = true
    }
}

final class MemoryRuntimeTransport: RuntimeTransport, @unchecked Sendable {
    func invoke(_ draft: InvocationDraft) async throws -> InvocationResult {
        try InvocationResult(ok: true, terminalState: .completed, outputJSON: "{\"ok\":true}")
    }

    func openStream(_ draft: InvocationDraft) async throws -> StreamSource {
        QueueStreamSource(count: 1)
    }

    func openBidi(_ draft: InvocationDraft, frame0: BidiFrame) async throws -> BidiSource {
        QueueBidiSource(count: 1)
    }
}

final class QueueStreamSource: StreamSource, @unchecked Sendable {
    private var events: [StreamEvent] = []

    init(count: Int) {
        for index in 0..<count {
            events.append(try! .data(index, payloadJSON: "{\"n\":\(index)}"))
        }
    }

    func next() async throws -> StreamEvent {
        if events.isEmpty {
            return try .terminal(9999, state: "Completed")
        }
        return events.removeFirst()
    }
}

final class QueueBidiSource: BidiSource, @unchecked Sendable {
    private var frames: [BidiFrame] = []

    init(count: Int) {
        for index in 0..<count {
            frames.append(try! .data(index, payloadJSON: "{\"n\":\(index)}"))
        }
    }

    func send(_ frame: BidiFrame) async throws {}

    func next() async throws -> BidiFrame {
        if frames.isEmpty {
            return try .terminal(9999, kind: "completed")
        }
        return frames.removeFirst()
    }
}
