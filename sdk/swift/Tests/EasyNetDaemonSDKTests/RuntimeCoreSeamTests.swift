import XCTest
@testable import EasyNetDaemonSDK

final class RuntimeCoreSeamTests: XCTestCase {
    func testFeatureDiscoveryAndTypedErrors() async throws {
        let transport = MemoryDiscoveryTransport()
        let client = Client(transport: transport)
        let features = try await client.requireABI(4)
        XCTAssertEqual(features.profiles["runtime_core"], "seam")
        await expectSDKError(.versionIncompatible) {
            _ = try await client.requireABI(5)
        }
        try await client.close()
        await expectSDKError(.invalidHandle) {
            _ = try await client.featureDiscovery()
        }
        XCTAssertEqual(SDKError.validation("x", "bad").errorClass, .validation)
    }

    func testCompleteInvocationDraftAndRuntimeDispatch() async throws {
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
        XCTAssertTrue(draft.inspectTuple().descriptorRef.hasSuffix("@1.0.0"))
        let result = try await runtime.invoke(draft)
        XCTAssertEqual(result.terminalState, .completed)
        expectSyncSDKError(.invalidArgument) {
            _ = try InvocationBuilder().withCallerURA("x").build()
        }
        try await runtime.close()
        expectSyncSDKError(.invalidHandle) {
            _ = try runtime.newInvocation()
        }
    }

    func testStreamHistoryIsBounded() async throws {
        let handle = StreamHandle(source: QueueStreamSource(count: StreamHandle.maxRetainedEvents + 2))
        for _ in 0..<(StreamHandle.maxRetainedEvents + 2) {
            _ = try await handle.next()
        }
        XCTAssertEqual(handle.terminalEvent()?.error?.code, .transport)
        XCTAssertEqual(handle.retainedEvents().count, StreamHandle.maxRetainedEvents + 1)
        try await handle.close()
    }

    func testStreamHandleIsAsyncSequence() async throws {
        let handle = StreamHandle(source: QueueStreamSource(count: 2))
        var sequences: [Int] = []
        var terminalCount = 0
        for try await event in handle {
            sequences.append(event.sequence)
            if event.terminal {
                terminalCount += 1
            }
        }
        XCTAssertEqual(sequences, [0, 1, 9999])
        XCTAssertEqual(terminalCount, 1)
        XCTAssertEqual(handle.terminalEvent()?.state, "Completed")
        try await handle.close()
    }

    func testBidiHistoryIsBounded() async throws {
        let session = BidiSession(source: QueueBidiSource(count: BidiSession.maxRetainedFrames + 2))
        try await session.send(.data(0, payloadJSON: "{\"hello\":true}"))
        for _ in 0..<(BidiSession.maxRetainedFrames + 2) {
            _ = try await session.next()
        }
        XCTAssertEqual(session.terminalFrame()?.kind, "backpressure_terminated")
        XCTAssertEqual(session.retainedFrames().count, BidiSession.maxRetainedFrames + 1)
        try await session.close()
    }

    func testBidiSessionIsAsyncSequence() async throws {
        let session = BidiSession(source: QueueBidiSource(count: 2))
        var sequences: [Int] = []
        var terminalCount = 0
        for try await frame in session {
            sequences.append(frame.sequence)
            if frame.terminal {
                terminalCount += 1
            }
        }
        XCTAssertEqual(sequences, [0, 1, 9999])
        XCTAssertEqual(terminalCount, 1)
        XCTAssertEqual(session.terminalFrame()?.kind, "completed")
        try await session.close()
    }

    private func expectSyncSDKError(_ code: SDKErrorCode, _ action: () throws -> Void) {
        do {
            try action()
        } catch let error as SDKError {
            XCTAssertEqual(error.code, code)
            return
        } catch {
            XCTFail("expected SDKError, got \(error)")
            return
        }
        XCTFail("expected SDKError \(code)")
    }

    private func expectSDKError(_ code: SDKErrorCode, _ action: () async throws -> Void) async {
        do {
            try await action()
        } catch let error as SDKError {
            XCTAssertEqual(error.code, code)
            return
        } catch {
            XCTFail("expected SDKError, got \(error)")
            return
        }
        XCTFail("expected SDKError \(code)")
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
