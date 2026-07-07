import Foundation

public let missionProfile = "mission"

public struct MissionCarrierBase: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try requiredMissionString(callerURA, "caller_ura")
        self.calleeURA = try requiredMissionString(calleeURA, "callee_ura")
        self.subjectURA = try requiredMissionString(subjectURA, "subject_ura")
        self.descriptorVersion = try requiredMissionString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try requiredMissionString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else { throw invalidMission("causal_context is required") }
        self.causalContext = causalContext
        self.metadata = metadata
    }

    func write(to object: inout [String: JSONValue]) {
        object["caller_ura"] = .string(callerURA)
        object["callee_ura"] = .string(calleeURA)
        object["subject_ura"] = .string(subjectURA)
        object["descriptor_version"] = .string(descriptorVersion)
        object["nonce_base64"] = .string(nonceBase64)
        object["causal_context"] = .object(causalContext)
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
    }
}

public struct MissionRunRequest: Sendable, Equatable {
    public let base: MissionCarrierBase
    public let source: String
    public let label: String

    public init(base: MissionCarrierBase, source: String, label: String = "") throws {
        self.base = base
        self.source = try requiredMissionString(source, "source")
        self.label = try optionalMissionString(label, "label")
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["source"] = .string(source)
        if !label.isEmpty { object["label"] = .string(label) }
        return try encodeJSONObject(object)
    }
}

public struct MissionRunFileRequest: Sendable, Equatable {
    public let base: MissionCarrierBase
    public let path: String
    public let label: String

    public init(base: MissionCarrierBase, path: String, label: String = "") throws {
        self.base = base
        self.path = try absoluteMissionPath(path)
        self.label = try optionalMissionString(label, "label")
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["path"] = .string(path)
        if !label.isEmpty { object["label"] = .string(label) }
        return try encodeJSONObject(object)
    }
}

public struct MissionTrackRequest: Sendable, Equatable {
    public let base: MissionCarrierBase
    public let missionID: String

    public init(base: MissionCarrierBase, missionID: String) throws {
        self.base = base
        self.missionID = try cleanMissionID(missionID)
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["mission_id"] = .string(missionID)
        return try encodeJSONObject(object)
    }
}

public struct MissionCancelRequest: Sendable, Equatable {
    public let base: MissionCarrierBase
    public let missionID: String

    public init(base: MissionCarrierBase, missionID: String) throws {
        self.base = base
        self.missionID = try cleanMissionID(missionID)
    }

    func jsonData() throws -> Data {
        try MissionTrackRequest(base: base, missionID: missionID).jsonData()
    }
}

public struct MissionEventsRequest: Sendable, Equatable {
    public let base: MissionCarrierBase
    public let missionID: String
    public let cursorSequence: Int
    public let limit: Int?

    public init(base: MissionCarrierBase, missionID: String, cursorSequence: Int = 0, limit: Int? = nil) throws {
        self.base = base
        self.missionID = try cleanMissionID(missionID)
        guard cursorSequence >= 0 else { throw invalidMission("cursor_sequence must be non-negative") }
        self.cursorSequence = cursorSequence
        if let limit, (limit < 0 || limit > 1000) {
            throw invalidMission("limit must be between 0 and 1000")
        }
        self.limit = limit
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["mission_id"] = .string(missionID)
        object["cursor_sequence"] = .number(Double(cursorSequence))
        if let limit { object["limit"] = .number(Double(limit)) }
        return try encodeJSONObject(object)
    }
}

public struct MissionStatus: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let missionID: String
    public let state: String
    public let terminal: Bool
    public let partialFailures: Int
    public let cancelled: Bool
    public let parentInvocationID: String?
    public let parentReceiptURA: String?
    public let parentInvocation: [String: JSONValue]
    public let childInvocations: [JSONValue]
    public let childReceipts: [JSONValue]
    public let outputRefs: [JSONValue]
    public let error: JSONValue?
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> MissionStatus {
        let object = try decodeMissionObject(raw, label: "mission status JSON")
        return try MissionStatus(
            profile: requiredMissionString(object, "profile"),
            kind: requiredMissionString(object, "kind"),
            missionID: requiredMissionString(object, "mission_id"),
            state: requiredMissionString(object, "state"),
            terminal: requiredMissionBool(object, "terminal"),
            partialFailures: requiredMissionInt(object, "partial_failures"),
            cancelled: requiredMissionBool(object, "cancelled"),
            parentInvocationID: optionalMissionJSONString(object["parent_invocation_id"], "parent_invocation_id"),
            parentReceiptURA: optionalMissionJSONString(object["parent_receipt_ura"], "parent_receipt_ura"),
            parentInvocation: optionalMissionObject(object["parent_invocation"], "parent_invocation") ?? [:],
            childInvocations: requiredMissionArray(object, "child_invocations"),
            childReceipts: requiredMissionArray(object, "child_receipts"),
            outputRefs: requiredMissionArray(object, "output_refs"),
            error: object["error"] == .null ? nil : object["error"],
            metadata: requiredMissionObject(object, "metadata")
        )
    }

    public init(
        profile: String,
        kind: String,
        missionID: String,
        state: String,
        terminal: Bool,
        partialFailures: Int,
        cancelled: Bool,
        parentInvocationID: String?,
        parentReceiptURA: String?,
        parentInvocation: [String: JSONValue],
        childInvocations: [JSONValue],
        childReceipts: [JSONValue],
        outputRefs: [JSONValue],
        error: JSONValue?,
        metadata: [String: JSONValue]
    ) throws {
        guard profile == missionProfile, kind == "mission_status" else { throw invalidMission("invalid mission status projection") }
        self.profile = profile
        self.kind = kind
        self.missionID = try cleanMissionID(missionID)
        self.state = try requiredMissionString(state, "state")
        self.terminal = terminal
        guard partialFailures >= 0 else { throw invalidMission("partial_failures must be non-negative") }
        self.partialFailures = partialFailures
        self.cancelled = cancelled
        self.parentInvocationID = parentInvocationID
        self.parentReceiptURA = parentReceiptURA
        self.parentInvocation = parentInvocation
        try validateMissionChildInvocationFacts(childInvocations)
        try validateMissionChildReceiptFacts(childReceipts)
        self.childInvocations = childInvocations
        self.childReceipts = childReceipts
        self.outputRefs = outputRefs
        self.error = error
        guard !metadata.isEmpty else { throw invalidMission("metadata must be an object") }
        self.metadata = metadata
    }
}

public typealias MissionRun = MissionStatus
public typealias MissionCancelResult = MissionStatus

public struct MissionEvent: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let missionID: String
    public let sequence: Int
    public let eventType: String
    public let occurredUnixMS: Int64
    public let terminal: Bool
    public let payload: [String: JSONValue]
    public let receipt: [String: JSONValue]
    public let metadata: [String: JSONValue]

    static func fromObject(_ object: [String: JSONValue]) throws -> MissionEvent {
        try MissionEvent(
            profile: requiredMissionString(object, "profile"),
            kind: requiredMissionString(object, "kind"),
            missionID: requiredMissionString(object, "mission_id"),
            sequence: requiredMissionInt(object, "sequence"),
            eventType: requiredMissionString(object, "event_type"),
            occurredUnixMS: Int64(requiredMissionInt(object, "occurred_unix_ms")),
            terminal: requiredMissionBool(object, "terminal"),
            payload: requiredMissionObject(object, "payload"),
            receipt: requiredMissionObject(object, "receipt"),
            metadata: requiredMissionObject(object, "metadata")
        )
    }

    public init(profile: String, kind: String, missionID: String, sequence: Int, eventType: String, occurredUnixMS: Int64, terminal: Bool, payload: [String: JSONValue], receipt: [String: JSONValue], metadata: [String: JSONValue]) throws {
        guard profile == missionProfile, kind == "mission_event" else { throw invalidMission("invalid mission event projection") }
        self.profile = profile
        self.kind = kind
        self.missionID = try cleanMissionID(missionID)
        guard sequence >= 0 else { throw invalidMission("sequence must be non-negative") }
        self.sequence = sequence
        self.eventType = try requiredMissionString(eventType, "event_type")
        guard occurredUnixMS >= 0 else { throw invalidMission("occurred_unix_ms must be non-negative") }
        self.occurredUnixMS = occurredUnixMS
        self.terminal = terminal
        self.payload = payload
        self.receipt = receipt
        self.metadata = metadata
    }
}

public struct MissionEventPage: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let missionID: String
    public let cursorSequence: Int
    public let nextCursorSequence: Int?
    public let hasMore: Bool
    public let droppedCount: Int
    public let events: [MissionEvent]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> MissionEventPage {
        let object = try decodeMissionObject(raw, label: "mission event page JSON")
        return try MissionEventPage(
            profile: requiredMissionString(object, "profile"),
            kind: requiredMissionString(object, "kind"),
            missionID: requiredMissionString(object, "mission_id"),
            cursorSequence: requiredMissionInt(object, "cursor_sequence"),
            nextCursorSequence: optionalMissionInt(object["next_cursor_sequence"], "next_cursor_sequence"),
            hasMore: requiredMissionBool(object, "has_more"),
            droppedCount: requiredMissionInt(object, "dropped_count"),
            events: requiredMissionArray(object, "events").map { item in
                guard case let .object(event) = item else { throw invalidMission("events items must be objects") }
                return try MissionEvent.fromObject(event)
            },
            metadata: requiredMissionObject(object, "metadata")
        )
    }

    public init(profile: String, kind: String, missionID: String, cursorSequence: Int, nextCursorSequence: Int?, hasMore: Bool, droppedCount: Int, events: [MissionEvent], metadata: [String: JSONValue]) throws {
        guard profile == missionProfile, kind == "mission_event_page" else { throw invalidMission("invalid mission event page projection") }
        self.profile = profile
        self.kind = kind
        self.missionID = try cleanMissionID(missionID)
        guard cursorSequence >= 0, droppedCount >= 0 else { throw invalidMission("mission event page counters must be non-negative") }
        self.cursorSequence = cursorSequence
        self.nextCursorSequence = nextCursorSequence
        self.hasMore = hasMore
        self.droppedCount = droppedCount
        self.events = events
        guard !metadata.isEmpty else { throw invalidMission("metadata must be an object") }
        self.metadata = metadata
    }
}

public final class MissionEventStream: AsyncSequence, @unchecked Sendable {
    public typealias Element = MissionEvent
    private let handle: StreamHandle

    public init(handle: StreamHandle) {
        self.handle = handle
    }

    public var state: String {
        handle.terminalEvent() == nil ? "Open" : "Terminal"
    }

    public func receive() async throws -> MissionEvent {
        let frame = try await handle.next()
        let object = try decodeMissionObject(Data(frame.payloadJSON.utf8), label: "mission stream event JSON")
        return try MissionEvent.fromObject(object)
    }

    public func cancel(reason: String = "") async throws {
        _ = try await handle.cancel(reason: reason)
    }

    public func close() async throws {
        try await handle.close()
    }

    public func makeAsyncIterator() -> MissionEventIterator {
        MissionEventIterator(stream: self)
    }
}

public struct MissionEventIterator: AsyncIteratorProtocol {
    private let stream: MissionEventStream
    private var finished = false

    init(stream: MissionEventStream) {
        self.stream = stream
    }

    public mutating func next() async throws -> MissionEvent? {
        if finished { return nil }
        let event = try await stream.receive()
        if event.terminal { finished = true }
        return event
    }
}

public protocol MissionTransport: AnyObject, Sendable {
    func buildRunEALInvocation(_ requestJSON: Data) async throws -> Data
    func buildRunFileInvocation(_ requestJSON: Data) async throws -> Data
    func buildTrackInvocation(_ requestJSON: Data) async throws -> Data
    func buildCancelInvocation(_ requestJSON: Data) async throws -> Data
    func buildEventsInvocation(_ requestJSON: Data) async throws -> Data
    func runEAL(_ requestJSON: Data) async throws -> Data
    func runFile(_ requestJSON: Data) async throws -> Data
    func track(_ requestJSON: Data) async throws -> Data
    func cancel(_ requestJSON: Data) async throws -> Data
    func events(_ requestJSON: Data) async throws -> Data
    func openEventStream(_ requestJSON: Data) async throws -> StreamSource
    func close() async throws
}

public extension MissionTransport {
    func buildRunEALInvocation(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission run invocation transport is not available") }
    func buildRunFileInvocation(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission run-file invocation transport is not available") }
    func buildTrackInvocation(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission track invocation transport is not available") }
    func buildCancelInvocation(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission cancel invocation transport is not available") }
    func buildEventsInvocation(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission events invocation transport is not available") }
    func runEAL(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission run transport is not available") }
    func runFile(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission run-file transport is not available") }
    func track(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission track transport is not available") }
    func cancel(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission cancel transport is not available") }
    func events(_ requestJSON: Data) async throws -> Data { throw missionUnsupported("mission events transport is not available") }
    func openEventStream(_ requestJSON: Data) async throws -> StreamSource { throw missionUnsupported("mission event stream transport is not available") }
    func close() async throws {}
}

public final class MissionClient: @unchecked Sendable {
    private let transport: MissionTransport
    private var closed = false

    public init(transport: MissionTransport) {
        self.transport = transport
    }

    public func buildRunEALInvocation(_ request: MissionRunRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildRunEALInvocation(request.jsonData()) }
    }

    public func buildRunFileInvocation(_ request: MissionRunFileRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildRunFileInvocation(request.jsonData()) }
    }

    public func buildTrackInvocation(_ request: MissionTrackRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildTrackInvocation(request.jsonData()) }
    }

    public func buildCancelInvocation(_ request: MissionCancelRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildCancelInvocation(request.jsonData()) }
    }

    public func buildEventsInvocation(_ request: MissionEventsRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildEventsInvocation(request.jsonData()) }
    }

    public func runEAL(_ request: MissionRunRequest) async throws -> MissionRun {
        try await MissionStatus.fromJSON(raw { try await transport.runEAL(request.jsonData()) })
    }

    public func runFile(_ request: MissionRunFileRequest) async throws -> MissionRun {
        try await MissionStatus.fromJSON(raw { try await transport.runFile(request.jsonData()) })
    }

    public func track(_ request: MissionTrackRequest) async throws -> MissionStatus {
        try await MissionStatus.fromJSON(raw { try await transport.track(request.jsonData()) })
    }

    public func cancel(_ request: MissionCancelRequest) async throws -> MissionCancelResult {
        try await MissionStatus.fromJSON(raw { try await transport.cancel(request.jsonData()) })
    }

    public func events(_ request: MissionEventsRequest) async throws -> MissionEventPage {
        try await MissionEventPage.fromJSON(raw { try await transport.events(request.jsonData()) })
    }

    public func openEventStream(_ request: MissionEventsRequest) async throws -> MissionEventStream {
        try requireOpen()
        do {
            return try await MissionEventStream(handle: StreamHandle(source: transport.openEventStream(request.jsonData())))
        } catch let error as SDKError {
            throw error
        } catch {
            throw missionTransport("mission event stream failed")
        }
    }

    public func projectStatus(_ raw: Data) throws -> MissionStatus {
        try MissionStatus.fromJSON(raw)
    }

    public func projectEvents(_ raw: Data) throws -> MissionEventPage {
        try MissionEventPage.fromJSON(raw)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodeMissionObject(try await raw(call), label: "mission invocation JSON")
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw missionTransport("mission transport failed")
        }
    }

    private func requireOpen() throws {
        if closed { throw SDKError.closed("mission") }
    }
}

private func decodeMissionObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else { throw invalidMission("\(label) must be an object") }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)", details: ["profile": missionProfile])
    }
}

private func requiredMissionString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidMission("\(field) is required")
    }
    return value
}

private func optionalMissionString(_ value: String, _ field: String) throws -> String {
    if value.isEmpty { return "" }
    return try requiredMissionString(value, field)
}

private func requiredMissionString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty { return value }
    throw invalidMission("\(name) must be a non-empty string")
}

private func optionalMissionJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidMission("\(name) must be a string or null")
    }
}

private func requiredMissionBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] { return value }
    throw invalidMission("\(name) must be a boolean")
}

private func requiredMissionInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if let value = try optionalMissionInt(object[name], name) { return value }
    throw invalidMission("\(name) must be a non-negative integer")
}

private func optionalMissionInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .number(number):
        guard number >= 0, number.rounded() == number, number <= Double(Int.max) else {
            throw invalidMission("\(name) must be a non-negative integer or null")
        }
        return Int(number)
    default:
        throw invalidMission("\(name) must be a non-negative integer or null")
    }
}

private func requiredMissionObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    guard case let .object(value) = object[name] else { throw invalidMission("\(name) must be an object") }
    return value
}

private func optionalMissionObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidMission("\(name) must be an object or null")
    }
}

private func requiredMissionArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    guard case let .array(value) = object[name] else { throw invalidMission("\(name) must be a list") }
    return value
}

private func validateMissionChildInvocationFacts(_ values: [JSONValue]) throws {
    for value in values {
        guard case let .object(child) = value else {
            throw invalidMission("child_invocations items must be objects")
        }
        let receipt = try optionalMissionObject(child["receipt"], "receipt") ?? [:]
        if receipt.isEmpty {
            continue
        }
        for field in [
            "step_id",
            "request_id",
            "trace_id",
            "ability",
            "invocation_ura",
            "caller_ura",
            "callee_ura",
            "subject_ura",
        ] {
            _ = try requiredMissionString(child, field)
        }
    }
}

private func validateMissionChildReceiptFacts(_ values: [JSONValue]) throws {
    for value in values {
        guard case let .object(receipt) = value else {
            throw invalidMission("child_receipts items must be objects")
        }
        _ = try requiredMissionString(receipt, "receipt_ura")
        _ = try requiredMissionString(receipt, "receipt_hash")
    }
}

private func cleanMissionID(_ value: String) throws -> String {
    let missionID = try requiredMissionString(value, "mission_id")
    if missionID.contains("/") || missionID.contains("://") || missionID.contains("\\") || missionID == "." || missionID == ".." {
        throw invalidMission("mission_id must be an opaque daemon mission id")
    }
    return missionID
}

private func absoluteMissionPath(_ value: String) throws -> String {
    let path = try requiredMissionString(value, "path")
    guard path.hasPrefix("/") else { throw invalidMission("path must be absolute") }
    return path
}

private func invalidMission(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: missionProfile, message: message, details: ["profile": missionProfile])
}

private func missionUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": missionProfile])
}

private func missionTransport(_ message: String) -> SDKError {
    SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: message, details: ["profile": missionProfile])
}
