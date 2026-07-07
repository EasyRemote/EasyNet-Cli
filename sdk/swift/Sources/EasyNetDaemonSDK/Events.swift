import Foundation

public let eventsProfile = "events"
public let minEventHeartbeatIntervalMS = 1000
public let maxEventHeartbeatIntervalMS = 300000
public let defaultEventPageSize = 50
public let maxEventPageSize = 500

public struct EventsCarrierBase: Sendable, Equatable {
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
        self.callerURA = try cleanEventString(callerURA, "caller_ura")
        self.calleeURA = try cleanEventString(calleeURA, "callee_ura")
        self.subjectURA = try cleanEventString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanEventString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanEventString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidEvents("causal_context is required")
        }
        self.causalContext = causalContext
        self.metadata = metadata
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "caller_ura": .string(callerURA),
            "callee_ura": .string(calleeURA),
            "subject_ura": .string(subjectURA),
            "descriptor_version": .string(descriptorVersion),
            "nonce_base64": .string(nonceBase64),
            "causal_context": .object(causalContext),
        ]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return object
    }
}

public struct EventCursor: Sendable, Equatable {
    public let stream: String
    public let sequence: Int
    public let token: String

    public init(stream: String, sequence: Int, token: String = "") throws {
        self.stream = try cleanEventStream(stream)
        guard sequence >= 0 else {
            throw invalidEvents("sequence must be a non-negative integer")
        }
        let resolvedToken = token.isEmpty ? "\(self.stream):\(sequence)" : try cleanEventString(token, "token")
        guard resolvedToken == "\(self.stream):\(sequence)" else {
            throw invalidEvents("event cursor token must match stream sequence")
        }
        self.sequence = sequence
        self.token = resolvedToken
    }

    public func resumeToken() -> String {
        token
    }

    func jsonObject(includeToken: Bool) -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "stream": .string(stream),
            "sequence": .number(Double(sequence)),
        ]
        if includeToken { object["token"] = .string(token) }
        return object
    }

    static func fromObject(_ object: [String: JSONValue], requireToken: Bool) throws -> EventCursor {
        try EventCursor(
            stream: requiredEventString(object, "stream"),
            sequence: requiredEventInt(object, "sequence"),
            token: requireToken ? requiredEventString(object, "token") : optionalEventString(object["token"], "token") ?? ""
        )
    }
}

public struct EventFilter: Sendable, Equatable {
    public let realm: String
    public let ownerURA: String
    public let deviceURA: String
    public let agentURA: String
    public let sessionID: String
    public let invocationID: String

    public init(
        realm: String = "",
        ownerURA: String = "",
        deviceURA: String = "",
        agentURA: String = "",
        sessionID: String = "",
        invocationID: String = ""
    ) throws {
        self.realm = try cleanNoWhitespaceEventString(realm, "realm")
        self.ownerURA = try optionalEventInput(ownerURA, "owner_ura")
        self.deviceURA = try optionalEventInput(deviceURA, "device_ura")
        self.agentURA = try optionalEventInput(agentURA, "agent_ura")
        self.sessionID = try cleanNoWhitespaceEventString(sessionID, "session_id")
        self.invocationID = try cleanNoWhitespaceEventString(invocationID, "invocation_id")
    }

    func normalized(topLevel: [String: String]) throws -> EventFilter {
        try EventFilter(
            realm: chooseEventField("realm", realm, topLevel["realm"] ?? ""),
            ownerURA: chooseEventField("owner_ura", ownerURA, topLevel["owner_ura"] ?? ""),
            deviceURA: chooseEventField("device_ura", deviceURA, topLevel["device_ura"] ?? ""),
            agentURA: chooseEventField("agent_ura", agentURA, topLevel["agent_ura"] ?? ""),
            sessionID: chooseEventField("session_id", sessionID, topLevel["session_id"] ?? ""),
            invocationID: chooseEventField("invocation_id", invocationID, topLevel["invocation_id"] ?? "")
        )
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [:]
        if !realm.isEmpty { object["realm"] = .string(realm) }
        if !ownerURA.isEmpty { object["owner_ura"] = .string(ownerURA) }
        if !deviceURA.isEmpty { object["device_ura"] = .string(deviceURA) }
        if !agentURA.isEmpty { object["agent_ura"] = .string(agentURA) }
        if !sessionID.isEmpty { object["session_id"] = .string(sessionID) }
        if !invocationID.isEmpty { object["invocation_id"] = .string(invocationID) }
        return object
    }
}

public struct EventsSubscriptionRequest: Sendable, Equatable {
    public let base: EventsCarrierBase
    public let stream: String
    public let filter: EventFilter?
    public let realm: String
    public let ownerURA: String
    public let deviceURA: String
    public let agentURA: String
    public let sessionID: String
    public let sessionURA: String
    public let invocationID: String
    public let resumeCursor: EventCursor?
    public let heartbeatIntervalMS: Int?

    public init(
        base: EventsCarrierBase,
        stream: String = "",
        filter: EventFilter? = nil,
        realm: String = "",
        ownerURA: String = "",
        deviceURA: String = "",
        agentURA: String = "",
        sessionID: String = "",
        sessionURA: String = "",
        invocationID: String = "",
        resumeCursor: EventCursor? = nil,
        heartbeatIntervalMS: Int? = nil
    ) throws {
        self.base = base
        self.stream = try optionalEventInput(stream, "stream")
        self.filter = filter
        self.realm = try cleanNoWhitespaceEventString(realm, "realm")
        self.ownerURA = try optionalEventInput(ownerURA, "owner_ura")
        self.deviceURA = try optionalEventInput(deviceURA, "device_ura")
        self.agentURA = try optionalEventInput(agentURA, "agent_ura")
        self.sessionID = try cleanNoWhitespaceEventString(sessionID, "session_id")
        self.sessionURA = try optionalEventInput(sessionURA, "session_ura")
        self.invocationID = try cleanNoWhitespaceEventString(invocationID, "invocation_id")
        self.resumeCursor = resumeCursor
        if let heartbeatIntervalMS,
           heartbeatIntervalMS < minEventHeartbeatIntervalMS || heartbeatIntervalMS > maxEventHeartbeatIntervalMS {
            throw invalidEvents("heartbeat_interval_ms exceeds bounds")
        }
        self.heartbeatIntervalMS = heartbeatIntervalMS
    }

    func jsonData(expectedStream: String) throws -> Data {
        let expected = try cleanEventStream(expectedStream)
        let resolvedStream = stream.isEmpty ? expected : try cleanEventStream(stream)
        guard resolvedStream == expected else {
            throw invalidEvents("event subscription stream mismatch")
        }
        let normalized = try (filter ?? EventFilter()).normalized(topLevel: [
            "realm": realm,
            "owner_ura": ownerURA,
            "device_ura": deviceURA,
            "agent_ura": agentURA,
            "session_id": sessionID,
            "invocation_id": invocationID,
        ])
        if let resumeCursor, resumeCursor.stream != expected {
            throw invalidEvents("resume cursor stream mismatch")
        }
        if expected == "session" {
            guard sessionURA.isEmpty else {
                throw invalidEvents("session_ura cannot be converted into daemon session_id")
            }
            guard !normalized.sessionID.isEmpty else {
                throw invalidEvents("session_id is required")
            }
        }
        if expected == "invocation", normalized.invocationID.isEmpty {
            throw invalidEvents("invocation_id is required")
        }
        var object = base.jsonObject()
        object["stream"] = .string(resolvedStream)
        let filterObject = normalized.jsonObject()
        if filter != nil, !filterObject.isEmpty { object["filter"] = .object(filterObject) }
        if !normalized.realm.isEmpty { object["realm"] = .string(normalized.realm) }
        if !normalized.ownerURA.isEmpty { object["owner_ura"] = .string(normalized.ownerURA) }
        if !normalized.deviceURA.isEmpty { object["device_ura"] = .string(normalized.deviceURA) }
        if !normalized.agentURA.isEmpty { object["agent_ura"] = .string(normalized.agentURA) }
        if !normalized.sessionID.isEmpty { object["session_id"] = .string(normalized.sessionID) }
        if !sessionURA.isEmpty { object["session_ura"] = .string(sessionURA) }
        if !normalized.invocationID.isEmpty { object["invocation_id"] = .string(normalized.invocationID) }
        if let resumeCursor { object["resume_cursor"] = .object(resumeCursor.jsonObject(includeToken: false)) }
        if let heartbeatIntervalMS { object["heartbeat_interval_ms"] = .number(Double(heartbeatIntervalMS)) }
        return try encodeJSONObject(object)
    }
}

public struct EventsDeviceEventListRequest: Sendable, Equatable {
    public let base: EventsCarrierBase
    public let filter: EventFilter?
    public let deviceURA: String
    public let limit: Int
    public let cursor: String

    public init(base: EventsCarrierBase, filter: EventFilter? = nil, deviceURA: String = "", limit: Int = 0, cursor: String = "") throws {
        self.base = base
        self.filter = filter
        self.deviceURA = try optionalEventInput(deviceURA, "device_ura")
        self.limit = limit
        self.cursor = try optionalEventInput(cursor, "cursor")
    }

    func jsonData() throws -> Data {
        let normalized = try (filter ?? EventFilter()).normalized(topLevel: ["device_ura": deviceURA])
        let resolvedLimit = limit == 0 ? defaultEventPageSize : limit
        guard resolvedLimit >= 1, resolvedLimit <= maxEventPageSize else {
            throw invalidEvents("limit exceeds bounds")
        }
        var object = base.jsonObject()
        let filterObject = normalized.jsonObject()
        if filter != nil, !filterObject.isEmpty { object["filter"] = .object(filterObject) }
        if !normalized.deviceURA.isEmpty { object["device_ura"] = .string(normalized.deviceURA) }
        object["limit"] = .number(Double(resolvedLimit))
        if !cursor.isEmpty { object["cursor"] = .string(cursor) }
        return try encodeJSONObject(object)
    }
}

public struct EventProjectionInput: Sendable, Equatable {
    public let cursor: EventCursor
    public let event: [String: JSONValue]
    public let eventID: String
    public let resumeToken: String
    public let tenantRef: JSONValue?

    public init(cursor: EventCursor, event: [String: JSONValue], eventID: String = "", resumeToken: String = "", tenantRef: JSONValue? = nil) throws {
        self.cursor = cursor
        guard !event.isEmpty else { throw invalidEvents("event payload is required") }
        self.event = event
        self.eventID = try optionalEventInput(eventID, "event_id")
        self.resumeToken = try optionalEventInput(resumeToken, "resume_token")
        self.tenantRef = tenantRef
    }

    func jsonData(expectedStream: String = "") throws -> Data {
        if !expectedStream.isEmpty, cursor.stream != expectedStream {
            throw invalidEvents("event cursor stream mismatch")
        }
        var object: [String: JSONValue] = [
            "cursor": .object(cursor.jsonObject(includeToken: false)),
            "event": .object(event),
        ]
        if !eventID.isEmpty { object["event_id"] = .string(eventID) }
        if !resumeToken.isEmpty { object["resume_token"] = .string(resumeToken) }
        if let tenantRef { object["tenant_ref"] = tenantRef }
        return try encodeJSONObject(object)
    }
}

public struct EventDropReportInput: Sendable, Equatable {
    public let cursor: EventCursor
    public let occurredUnixMS: Int64
    public let droppedCount: Int
    public let reconnectAfterMS: Int?
    public let reason: String
    public let eventID: String
    public let resumeToken: String
    public let tenantRef: JSONValue?

    public init(cursor: EventCursor, occurredUnixMS: Int64, droppedCount: Int, reconnectAfterMS: Int? = nil, reason: String = "", eventID: String = "", resumeToken: String = "", tenantRef: JSONValue? = nil) throws {
        guard cursor.stream == "directory" else { throw invalidEvents("event cursor stream mismatch") }
        guard occurredUnixMS >= 0 else { throw invalidEvents("occurred_unix_ms must be non-negative") }
        guard droppedCount > 0 else { throw invalidEvents("dropped_count must be greater than zero") }
        if let reconnectAfterMS, reconnectAfterMS < 0 { throw invalidEvents("reconnect_after_ms must be non-negative") }
        self.cursor = cursor
        self.occurredUnixMS = occurredUnixMS
        self.droppedCount = droppedCount
        self.reconnectAfterMS = reconnectAfterMS
        self.reason = try optionalEventInput(reason, "reason")
        self.eventID = try optionalEventInput(eventID, "event_id")
        self.resumeToken = try optionalEventInput(resumeToken, "resume_token")
        self.tenantRef = tenantRef
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "cursor": .object(cursor.jsonObject(includeToken: false)),
            "occurred_unix_ms": .number(Double(occurredUnixMS)),
            "dropped_count": .number(Double(droppedCount)),
        ]
        try copyOptionalEventProjectionFields(self, into: &object)
        return try encodeJSONObject(object)
    }
}

public struct EventTerminalInput: Sendable, Equatable {
    public let cursor: EventCursor
    public let occurredUnixMS: Int64
    public let reconnectAfterMS: Int?
    public let reason: String
    public let eventID: String
    public let resumeToken: String
    public let tenantRef: JSONValue?

    public init(cursor: EventCursor, occurredUnixMS: Int64, reconnectAfterMS: Int? = nil, reason: String = "", eventID: String = "", resumeToken: String = "", tenantRef: JSONValue? = nil) throws {
        guard cursor.stream == "directory" else { throw invalidEvents("event cursor stream mismatch") }
        guard occurredUnixMS >= 0 else { throw invalidEvents("occurred_unix_ms must be non-negative") }
        if let reconnectAfterMS, reconnectAfterMS < 0 { throw invalidEvents("reconnect_after_ms must be non-negative") }
        self.cursor = cursor
        self.occurredUnixMS = occurredUnixMS
        self.reconnectAfterMS = reconnectAfterMS
        self.reason = try optionalEventInput(reason, "reason")
        self.eventID = try optionalEventInput(eventID, "event_id")
        self.resumeToken = try optionalEventInput(resumeToken, "resume_token")
        self.tenantRef = tenantRef
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "cursor": .object(cursor.jsonObject(includeToken: false)),
            "occurred_unix_ms": .number(Double(occurredUnixMS)),
        ]
        try copyOptionalEventProjectionFields(self, into: &object)
        return try encodeJSONObject(object)
    }
}

public struct EventFrame: Sendable, Equatable {
    public let profile: String
    public let stream: String
    public let kind: String
    public let eventID: String
    public let cursor: EventCursor
    public let resumeToken: String
    public let occurredUnixMS: Int64
    public let occurredAt: String
    public let subjectRef: JSONValue?
    public let tenantRef: JSONValue?
    public let payload: JSONValue?
    public let droppedCount: Int
    public let reconnectAfterMS: Int?
    public let terminal: Bool
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> EventFrame {
        try fromObject(decodeEventObject(raw, label: "event frame JSON"))
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> EventFrame {
        let frame = try EventFrame(
            profile: requiredEventString(object, "profile"),
            stream: requiredEventString(object, "stream"),
            kind: requiredEventString(object, "kind"),
            eventID: requiredEventString(object, "event_id"),
            cursor: EventCursor.fromObject(requiredEventObject(object, "cursor"), requireToken: true),
            resumeToken: requiredEventString(object, "resume_token"),
            occurredUnixMS: Int64(requiredEventInt(object, "occurred_unix_ms")),
            occurredAt: requiredEventString(object, "occurred_at"),
            subjectRef: object["subject_ref"],
            tenantRef: object["tenant_ref"],
            payload: object["payload"],
            droppedCount: requiredEventInt(object, "dropped_count"),
            reconnectAfterMS: optionalEventInt(object["reconnect_after_ms"], "reconnect_after_ms"),
            terminal: requiredEventBool(object, "terminal"),
            metadata: requiredEventObject(object, "metadata")
        )
        try frame.validate()
        return frame
    }

    private func validate() throws {
        guard profile == eventsProfile, stream == cursor.stream else {
            throw invalidEvents("invalid event frame projection")
        }
        if kind.contains("drop_report"), droppedCount == 0 {
            throw invalidEvents("dropped_count must be greater than zero")
        }
        if kind.contains("terminal"), !terminal {
            throw invalidEvents("terminal event frame must be terminal")
        }
    }
}

public struct DeviceEventPage: Sendable, Equatable {
    public let profile: String
    public let stream: String
    public let itemKind: String
    public let items: [EventFrame]
    public let nextCursor: String?
    public let hasMore: Bool
    public let limit: Int
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> DeviceEventPage {
        let object = try decodeEventObject(raw, label: "device event page JSON")
        let items = try requiredEventArray(object, "items").map { value -> EventFrame in
            guard case let .object(eventObject) = value else {
                throw invalidEvents("device event page item must be an object")
            }
            let frame = try EventFrame.fromObject(eventObject)
            guard frame.stream == "device" else {
                throw invalidEvents("device event page item stream mismatch")
            }
            return frame
        }
        let page = try DeviceEventPage(
            profile: requiredEventString(object, "profile"),
            stream: requiredEventString(object, "stream"),
            itemKind: requiredEventString(object, "item_kind"),
            items: items,
            nextCursor: optionalEventString(object["next_cursor"], "next_cursor"),
            hasMore: requiredEventBool(object, "has_more"),
            limit: requiredEventInt(object, "limit"),
            metadata: requiredEventObject(object, "metadata")
        )
        guard page.profile == eventsProfile, page.stream == "device", page.limit >= 1, page.limit <= maxEventPageSize else {
            throw invalidEvents("invalid device event page projection")
        }
        return page
    }
}

public final class EventStream: @unchecked Sendable {
    public let stream: String
    public let handle: StreamHandle
    public private(set) var state: String
    public let streamID: String
    public let resumeToken: String
    public let metadata: [String: JSONValue]

    public init(stream: String, source: StreamSource, state: String = "Live", streamID: String = "", resumeToken: String = "", metadata: [String: JSONValue] = ["profile": .string(eventsProfile)]) throws {
        self.stream = try cleanEventStream(stream)
        self.handle = StreamHandle(source: source)
        self.state = try cleanEventString(state, "state")
        self.streamID = streamID
        self.resumeToken = resumeToken
        self.metadata = metadata
    }

    public func receive() async throws -> EventFrame {
        let event = try await handle.next()
        let frame = try EventFrame.fromJSON(Data(event.payloadJSON.utf8))
        if frame.terminal {
            state = "Terminal"
        }
        return frame
    }

    public func cancel(reason: String) async throws {
        _ = try await handle.cancel(reason: reason)
        state = "Cancelled"
    }

    public func close() async throws {
        try await handle.close()
        state = "Closed"
    }
}

public protocol EventTransport: AnyObject, Sendable {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data
    func buildDeviceSubscriptionInvocation(_ requestJSON: Data) async throws -> Data
    func buildSessionSubscriptionInvocation(_ requestJSON: Data) async throws -> Data
    func buildInvocationSubscriptionInvocation(_ requestJSON: Data) async throws -> Data
    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource
    func subscribeDevices(_ requestJSON: Data) async throws -> StreamSource
    func subscribeSessions(_ requestJSON: Data) async throws -> StreamSource
    func subscribeInvocations(_ requestJSON: Data) async throws -> StreamSource
    func listDeviceEvents(_ requestJSON: Data) async throws -> Data
    func projectDirectoryEvent(_ eventJSON: Data) async throws -> Data
    func projectLiveEvent(_ eventJSON: Data) async throws -> Data
    func projectDropReport(_ dropJSON: Data) async throws -> Data
    func projectTerminal(_ terminalJSON: Data) async throws -> Data
    func close() async throws
}

public extension EventTransport {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data { throw eventsUnsupported("events directory subscription invocation transport is not available") }
    func buildDeviceSubscriptionInvocation(_ requestJSON: Data) async throws -> Data { throw eventsUnsupported("events device subscription invocation transport is not available") }
    func buildSessionSubscriptionInvocation(_ requestJSON: Data) async throws -> Data { throw eventsUnsupported("events session subscription invocation transport is not available") }
    func buildInvocationSubscriptionInvocation(_ requestJSON: Data) async throws -> Data { throw eventsUnsupported("events invocation subscription invocation transport is not available") }
    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource { throw eventsUnsupported("events subscribe directory transport is not available") }
    func subscribeDevices(_ requestJSON: Data) async throws -> StreamSource { throw eventsUnsupported("events subscribe devices transport is not available") }
    func subscribeSessions(_ requestJSON: Data) async throws -> StreamSource { throw eventsUnsupported("events subscribe sessions transport is not available") }
    func subscribeInvocations(_ requestJSON: Data) async throws -> StreamSource { throw eventsUnsupported("events subscribe invocations transport is not available") }
    func listDeviceEvents(_ requestJSON: Data) async throws -> Data { throw eventsUnsupported("events list device events transport is not available") }
    func projectDirectoryEvent(_ eventJSON: Data) async throws -> Data { throw eventsUnsupported("events project directory event transport is not available") }
    func projectLiveEvent(_ eventJSON: Data) async throws -> Data { throw eventsUnsupported("events project live event transport is not available") }
    func projectDropReport(_ dropJSON: Data) async throws -> Data { throw eventsUnsupported("events project drop report transport is not available") }
    func projectTerminal(_ terminalJSON: Data) async throws -> Data { throw eventsUnsupported("events project terminal transport is not available") }
    func close() async throws {}
}

public final class EventClient: @unchecked Sendable {
    private let transport: EventTransport
    private var closed = false

    public init(transport: EventTransport) {
        self.transport = transport
    }

    public func buildDirectorySubscriptionInvocation(_ request: EventsSubscriptionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildDirectorySubscriptionInvocation(request.jsonData(expectedStream: "directory")) }
    }

    public func buildDeviceSubscriptionInvocation(_ request: EventsSubscriptionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildDeviceSubscriptionInvocation(request.jsonData(expectedStream: "device")) }
    }

    public func buildSessionSubscriptionInvocation(_ request: EventsSubscriptionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildSessionSubscriptionInvocation(request.jsonData(expectedStream: "session")) }
    }

    public func buildInvocationSubscriptionInvocation(_ request: EventsSubscriptionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildInvocationSubscriptionInvocation(request.jsonData(expectedStream: "invocation")) }
    }

    public func subscribeDirectory(_ request: EventsSubscriptionRequest) async throws -> EventStream {
        try await subscribe(request, stream: "directory") { try await transport.subscribeDirectory($0) }
    }

    public func subscribeDevices(_ request: EventsSubscriptionRequest) async throws -> EventStream {
        try await subscribe(request, stream: "device") { try await transport.subscribeDevices($0) }
    }

    public func subscribeSessions(_ request: EventsSubscriptionRequest) async throws -> EventStream {
        try await subscribe(request, stream: "session") { try await transport.subscribeSessions($0) }
    }

    public func subscribeInvocations(_ request: EventsSubscriptionRequest) async throws -> EventStream {
        try await subscribe(request, stream: "invocation") { try await transport.subscribeInvocations($0) }
    }

    public func listDeviceEvents(_ request: EventsDeviceEventListRequest) async throws -> DeviceEventPage {
        let data = try await raw { try await transport.listDeviceEvents(request.jsonData()) }
        return try DeviceEventPage.fromJSON(data)
    }

    public func projectDirectoryEvent(_ input: EventProjectionInput) async throws -> EventFrame {
        try await frame(input.jsonData(expectedStream: "directory"), transport.projectDirectoryEvent)
    }

    public func projectLiveEvent(_ input: EventProjectionInput) async throws -> EventFrame {
        try await frame(input.jsonData(), transport.projectLiveEvent)
    }

    public func projectDropReport(_ input: EventDropReportInput) async throws -> EventFrame {
        try await frame(input.jsonData(), transport.projectDropReport)
    }

    public func projectTerminal(_ input: EventTerminalInput) async throws -> EventFrame {
        try await frame(input.jsonData(), transport.projectTerminal)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func subscribe(_ request: EventsSubscriptionRequest, stream: String, _ call: (Data) async throws -> StreamSource) async throws -> EventStream {
        try requireOpen()
        do {
            return try await EventStream(stream: stream, source: call(request.jsonData(expectedStream: stream)))
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: "events subscribe transport failed", details: ["profile": eventsProfile])
        }
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodeEventObject(try await raw(call), label: "events invocation JSON")
    }

    private func frame(_ requestJSON: Data, _ call: (Data) async throws -> Data) async throws -> EventFrame {
        try EventFrame.fromJSON(try await raw { try await call(requestJSON) })
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: "events transport failed", details: ["profile": eventsProfile])
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("events")
        }
    }
}

private func copyOptionalEventProjectionFields(_ input: EventDropReportInput, into object: inout [String: JSONValue]) throws {
    if let reconnectAfterMS = input.reconnectAfterMS { object["reconnect_after_ms"] = .number(Double(reconnectAfterMS)) }
    if !input.reason.isEmpty { object["reason"] = .string(input.reason) }
    if !input.eventID.isEmpty { object["event_id"] = .string(input.eventID) }
    if !input.resumeToken.isEmpty { object["resume_token"] = .string(input.resumeToken) }
    if let tenantRef = input.tenantRef { object["tenant_ref"] = tenantRef }
}

private func copyOptionalEventProjectionFields(_ input: EventTerminalInput, into object: inout [String: JSONValue]) throws {
    if let reconnectAfterMS = input.reconnectAfterMS { object["reconnect_after_ms"] = .number(Double(reconnectAfterMS)) }
    if !input.reason.isEmpty { object["reason"] = .string(input.reason) }
    if !input.eventID.isEmpty { object["event_id"] = .string(input.eventID) }
    if !input.resumeToken.isEmpty { object["resume_token"] = .string(input.resumeToken) }
    if let tenantRef = input.tenantRef { object["tenant_ref"] = tenantRef }
}

private func chooseEventField(_ name: String, _ filterValue: String, _ topLevel: String) throws -> String {
    let filtered = try optionalEventInput(filterValue, "filter.\(name)")
    let top = try optionalEventInput(topLevel, name)
    if !filtered.isEmpty, !top.isEmpty, filtered != top {
        throw invalidEvents("\(name) conflicts with filter field")
    }
    return filtered.isEmpty ? top : filtered
}

private func cleanEventStream(_ value: String) throws -> String {
    let stream = try cleanEventString(value, "stream")
    guard ["directory", "device", "session", "invocation"].contains(stream) else {
        throw invalidEvents("unsupported event stream")
    }
    return stream
}

private func cleanEventString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidEvents("\(field) is required")
    }
    return value
}

private func optionalEventInput(_ value: String, _ field: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidEvents("\(field) must not contain surrounding whitespace")
    }
    return value
}

private func cleanNoWhitespaceEventString(_ value: String, _ field: String) throws -> String {
    let cleaned = try optionalEventInput(value, field)
    guard !cleaned.contains(where: { $0.isWhitespace }) else {
        throw invalidEvents("\(field) must not contain whitespace")
    }
    return cleaned
}

private func decodeEventObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidEvents("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

private func requiredEventString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty { return value }
    throw invalidEvents("\(name) is required")
}

private func optionalEventString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .string(string): return string
    default: throw invalidEvents("\(name) must be a string or null")
    }
}

private func requiredEventInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if case let .number(value) = object[name], value >= 0, value.rounded() == value, value <= Double(Int.max) {
        return Int(value)
    }
    throw invalidEvents("\(name) must be a non-negative integer")
}

private func optionalEventInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else { return nil }
    switch value {
    case .null: return nil
    case let .number(number) where number >= 0 && number.rounded() == number && number <= Double(Int.max):
        return Int(number)
    default:
        throw invalidEvents("\(name) must be a non-negative integer or null")
    }
}

private func requiredEventBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] { return value }
    throw invalidEvents("\(name) must be a boolean")
}

private func requiredEventObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] { return value }
    throw invalidEvents("\(name) must be an object")
}

private func requiredEventArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    if case let .array(value) = object[name] { return value }
    throw invalidEvents("\(name) must be a list")
}

private func invalidEvents(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: eventsProfile, message: message, details: ["profile": eventsProfile])
}

private func eventsUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": eventsProfile])
}
