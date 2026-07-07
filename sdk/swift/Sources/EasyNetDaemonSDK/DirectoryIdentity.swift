import Foundation

public let defaultDirectoryPageSize = 50
public let maxDirectoryPageSize = 500
public let directoryIdentityProfile = "directory_identity"

public struct DirectoryQueryBase: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let limit: Int
    public let cursor: String
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        limit: Int = 0,
        cursor: String = "",
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try cleanDirectoryString(callerURA, "caller_ura")
        self.calleeURA = try cleanDirectoryString(calleeURA, "callee_ura")
        self.subjectURA = try cleanDirectoryString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanDirectoryString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanDirectoryString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidDirectory("causal_context is required")
        }
        guard limit >= 0, limit <= maxDirectoryPageSize else {
            throw invalidDirectory("limit must be between 1 and \(maxDirectoryPageSize)")
        }
        self.causalContext = causalContext
        self.limit = limit
        self.cursor = try optionalDirectoryString(cursor, "cursor")
        self.metadata = metadata
    }

    public func withDefaultLimit() throws -> DirectoryQueryBase {
        if limit == 0 {
            return try DirectoryQueryBase(
                callerURA: callerURA,
                calleeURA: calleeURA,
                subjectURA: subjectURA,
                descriptorVersion: descriptorVersion,
                nonceBase64: nonceBase64,
                causalContext: causalContext,
                limit: defaultDirectoryPageSize,
                cursor: cursor,
                metadata: metadata
            )
        }
        return self
    }

    func jsonObject(requireLimit: Bool) throws -> [String: JSONValue] {
        let value = requireLimit ? try withDefaultLimit() : self
        if requireLimit, value.limit < 1 {
            throw invalidDirectory("limit must be between 1 and \(maxDirectoryPageSize)")
        }
        var object: [String: JSONValue] = [
            "caller_ura": .string(value.callerURA),
            "callee_ura": .string(value.calleeURA),
            "subject_ura": .string(value.subjectURA),
            "descriptor_version": .string(value.descriptorVersion),
            "nonce_base64": .string(value.nonceBase64),
            "causal_context": .object(value.causalContext),
        ]
        if value.limit > 0 {
            object["limit"] = .number(Double(value.limit))
        }
        if !value.cursor.isEmpty {
            object["cursor"] = .string(value.cursor)
        }
        if !value.metadata.isEmpty {
            object["metadata"] = .object(value.metadata)
        }
        return object
    }

    func jsonData(requireLimit: Bool) throws -> Data {
        try encodeJSONObject(jsonObject(requireLimit: requireLimit))
    }
}

public struct ResolveQuery: Sendable, Equatable {
    public let base: DirectoryQueryBase
    public let queryName: String
    public let abilityName: String
    public let queryType: String
    public let realmHint: String
    public let peerHubURLs: [String]

    public init(
        base: DirectoryQueryBase,
        queryName: String = "",
        abilityName: String = "",
        queryType: String = "",
        realmHint: String = "",
        peerHubURLs: [String] = []
    ) throws {
        self.base = base
        self.queryName = try optionalDirectoryString(queryName, "query_name")
        self.abilityName = try optionalDirectoryString(abilityName, "ability_name")
        self.queryType = try optionalDirectoryString(queryType, "qtype")
        self.realmHint = try optionalDirectoryString(realmHint, "realm_hint")
        self.peerHubURLs = peerHubURLs
        guard !self.queryName.isEmpty || !self.realmHint.isEmpty else {
            throw invalidDirectory("query_name or realm_hint is required")
        }
    }

    func jsonData() throws -> Data {
        var object = try base.jsonObject(requireLimit: false)
        if !queryName.isEmpty { object["query_name"] = .string(queryName) }
        if !abilityName.isEmpty { object["ability_name"] = .string(abilityName) }
        if !queryType.isEmpty { object["qtype"] = .string(queryType) }
        if !realmHint.isEmpty { object["realm_hint"] = .string(realmHint) }
        if !peerHubURLs.isEmpty { object["peer_hub_urls"] = .array(peerHubURLs.map(JSONValue.string)) }
        return try encodeJSONObject(object)
    }
}

public struct AbilityQuery: Sendable, Equatable {
    public let base: DirectoryQueryBase
    public let scope: String
    public let ownerURA: String
    public let abilityURA: String

    public init(base: DirectoryQueryBase, scope: String = "", ownerURA: String = "", abilityURA: String = "") throws {
        self.base = base
        self.scope = try optionalDirectoryString(scope, "scope")
        self.ownerURA = try optionalDirectoryString(ownerURA, "owner_ura")
        self.abilityURA = try optionalDirectoryString(abilityURA, "ability_ura")
    }

    func jsonData() throws -> Data {
        var object = try base.withDefaultLimit().jsonObject(requireLimit: true)
        if !scope.isEmpty { object["scope"] = .string(scope) }
        if !ownerURA.isEmpty { object["owner_ura"] = .string(ownerURA) }
        if !abilityURA.isEmpty { object["ability_ura"] = .string(abilityURA) }
        return try encodeJSONObject(object)
    }
}

public struct DirectorySubscriptionCursor: Sendable, Equatable {
    public let stream: String
    public let sequence: Int
    public let token: String

    public init(stream: String = "directory", sequence: Int, token: String = "") throws {
        self.stream = try cleanDirectoryString(stream, "stream")
        guard self.stream == "directory" else {
            throw invalidDirectory("directory subscription cursor stream mismatch")
        }
        guard sequence >= 0 else {
            throw invalidDirectory("directory subscription cursor sequence must be non-negative")
        }
        let resolvedToken = token.isEmpty ? "\(self.stream):\(sequence)" : try cleanDirectoryString(token, "token")
        guard resolvedToken == "\(self.stream):\(sequence)" else {
            throw invalidDirectory("directory subscription cursor token mismatch")
        }
        self.sequence = sequence
        self.token = resolvedToken
    }

    public func resumeToken() -> String {
        token
    }

    func jsonObject() -> [String: JSONValue] {
        [
            "stream": .string(stream),
            "sequence": .number(Double(sequence)),
            "token": .string(token),
        ]
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> DirectorySubscriptionCursor {
        try DirectorySubscriptionCursor(
            stream: requiredDirectoryString(object, "stream"),
            sequence: requiredDirectoryInt(object, "sequence"),
            token: requiredDirectoryString(object, "token")
        )
    }
}

public struct DirectorySubscriptionRequest: Sendable, Equatable {
    public let base: DirectoryQueryBase
    public let stream: String
    public let realm: String
    public let ownerURA: String
    public let deviceURA: String
    public let agentURA: String
    public let abilityURA: String
    public let itemKind: String
    public let resumeCursor: DirectorySubscriptionCursor?
    public let heartbeatIntervalMS: Int?
    public let metadata: [String: JSONValue]

    public init(
        base: DirectoryQueryBase,
        stream: String = "directory",
        realm: String = "",
        ownerURA: String = "",
        deviceURA: String = "",
        agentURA: String = "",
        abilityURA: String = "",
        itemKind: String = "",
        resumeCursor: DirectorySubscriptionCursor? = nil,
        heartbeatIntervalMS: Int? = nil,
        metadata: [String: JSONValue] = [:]
    ) throws {
        guard base.limit == 0, base.cursor.isEmpty else {
            throw invalidDirectory("directory subscription uses resume_cursor, not pagination cursor")
        }
        self.base = base
        self.stream = try cleanDirectoryString(stream, "stream")
        guard self.stream == "directory" else {
            throw invalidDirectory("directory subscription stream mismatch")
        }
        self.realm = try optionalDirectoryString(realm, "realm")
        self.ownerURA = try optionalDirectoryString(ownerURA, "owner_ura")
        self.deviceURA = try optionalDirectoryString(deviceURA, "device_ura")
        self.agentURA = try optionalDirectoryString(agentURA, "agent_ura")
        self.abilityURA = try optionalDirectoryString(abilityURA, "ability_ura")
        self.itemKind = try optionalDirectoryString(itemKind, "item_kind")
        self.resumeCursor = resumeCursor
        if let heartbeatIntervalMS, heartbeatIntervalMS < 0 {
            throw invalidDirectory("heartbeat_interval_ms must be non-negative")
        }
        self.heartbeatIntervalMS = heartbeatIntervalMS
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object = try base.jsonObject(requireLimit: false)
        object["stream"] = .string(stream)
        if !realm.isEmpty { object["realm"] = .string(realm) }
        if !ownerURA.isEmpty { object["owner_ura"] = .string(ownerURA) }
        if !deviceURA.isEmpty { object["device_ura"] = .string(deviceURA) }
        if !agentURA.isEmpty { object["agent_ura"] = .string(agentURA) }
        if !abilityURA.isEmpty { object["ability_ura"] = .string(abilityURA) }
        if !itemKind.isEmpty { object["item_kind"] = .string(itemKind) }
        if let resumeCursor { object["resume_cursor"] = .object(resumeCursor.jsonObject()) }
        if let heartbeatIntervalMS {
            object["heartbeat_interval_ms"] = .number(Double(heartbeatIntervalMS))
        }
        let mergedMetadata = metadata.isEmpty ? base.metadata : metadata
        if !mergedMetadata.isEmpty { object["metadata"] = .object(mergedMetadata) }
        return try encodeJSONObject(object)
    }
}

public struct DirectorySubscription: Sendable, Equatable {
    public static let maxBufferedEvents = 1024

    public let profile: String
    public let kind: String
    public let stream: String
    public let state: String
    public let cursor: DirectorySubscriptionCursor
    public let resumeToken: String
    public let dropCount: Int
    public let events: [Event]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> DirectorySubscription {
        let object = try decodeDirectoryObject(raw, label: "directory subscription JSON")
        let events = try requiredDirectoryArray(object, "events").map { value -> Event in
            guard case let .object(eventObject) = value else {
                throw invalidDirectory("directory subscription event must be an object")
            }
            return try Event.fromObject(eventObject)
        }
        let subscription = try DirectorySubscription(
            profile: requiredDirectoryString(object, "profile"),
            kind: requiredDirectoryString(object, "kind"),
            stream: requiredDirectoryString(object, "stream"),
            state: requiredDirectoryString(object, "state"),
            cursor: DirectorySubscriptionCursor.fromObject(requiredDirectoryObject(object, "cursor")),
            resumeToken: requiredDirectoryString(object, "resume_token"),
            dropCount: requiredDirectoryInt(object, "drop_count"),
            events: events,
            metadata: requiredDirectoryObject(object, "metadata")
        )
        try subscription.validate()
        return subscription
    }

    private func validate() throws {
        guard profile == directoryIdentityProfile, kind == "directory_subscription", stream == "directory" else {
            throw invalidDirectory("directory subscription projection mismatch")
        }
        guard ["Opening", "CatchingUp", "Live", "Resuming", "Closed", "Failed"].contains(state) else {
            throw invalidDirectory("directory subscription state is unsupported")
        }
        guard resumeToken == cursor.resumeToken() else {
            throw invalidDirectory("directory subscription resume token mismatch")
        }
        guard dropCount >= 0, events.count <= DirectorySubscription.maxBufferedEvents else {
            throw invalidDirectory("directory subscription buffered events exceeds bounds")
        }
    }

    public struct Event: Sendable, Equatable {
        public let profile: String
        public let stream: String
        public let kind: String
        public let eventID: String
        public let phase: String
        public let itemKind: String?
        public let item: [String: JSONValue]?
        public let cursor: DirectorySubscriptionCursor
        public let resumeToken: String
        public let terminal: Bool
        public let metadata: [String: JSONValue]

        static func fromObject(_ object: [String: JSONValue]) throws -> Event {
            let event = try Event(
                profile: requiredDirectoryString(object, "profile"),
                stream: requiredDirectoryString(object, "stream"),
                kind: requiredDirectoryString(object, "kind"),
                eventID: requiredDirectoryString(object, "event_id"),
                phase: requiredDirectoryString(object, "phase"),
                itemKind: optionalDirectoryJSONString(object["item_kind"], "item_kind"),
                item: optionalDirectoryObject(object["item"], "item"),
                cursor: DirectorySubscriptionCursor.fromObject(requiredDirectoryObject(object, "cursor")),
                resumeToken: requiredDirectoryString(object, "resume_token"),
                terminal: requiredDirectoryBool(object, "terminal"),
                metadata: requiredDirectoryObject(object, "metadata")
            )
            guard event.profile == directoryIdentityProfile, event.stream == "directory" else {
                throw invalidDirectory("directory subscription event projection mismatch")
            }
            guard event.resumeToken == event.cursor.resumeToken() else {
                throw invalidDirectory("directory subscription event resume token mismatch")
            }
            return event
        }
    }
}

public struct DirectoryPage: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let itemKind: String
    public let items: [JSONValue]
    public let nextCursor: String?
    public let limit: Int
    public let source: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data, expectedKind: String, expectedItemKind: String) throws -> DirectoryPage {
        let object = try decodeDirectoryObject(raw, label: "directory page JSON")
        let page = try DirectoryPage(
            profile: requiredDirectoryString(object, "profile"),
            kind: requiredDirectoryString(object, "kind"),
            itemKind: requiredDirectoryString(object, "item_kind"),
            items: requiredDirectoryArray(object, "items"),
            nextCursor: optionalDirectoryJSONString(object["next_cursor"], "next_cursor"),
            limit: requiredDirectoryInt(object, "limit"),
            source: requiredDirectoryString(object, "source"),
            metadata: requiredDirectoryObject(object, "metadata")
        )
        guard page.profile == directoryIdentityProfile,
              page.kind == expectedKind,
              page.itemKind == expectedItemKind,
              page.source == "read_model"
        else {
            throw invalidDirectory("directory page projection mismatch")
        }
        guard page.limit >= 1, page.limit <= maxDirectoryPageSize else {
            throw invalidDirectory("directory page limit exceeds bounds")
        }
        return page
    }
}

public struct ResolvedRef: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let answerKind: String
    public let queryName: String?
    public let canonicalName: String?
    public let ownerURA: String?
    public let abilityURA: String?
    public let routeURA: String?
    public let nextHop: [String: JSONValue]?
    public let selectedRoute: [String: JSONValue]?
    public let routeCandidates: [JSONValue]
    public let records: [JSONValue]
    public let negative: [String: JSONValue]?
    public let releaseProfile: String?
    public let authority: [String: JSONValue]?
    public let cachePolicy: [String: JSONValue]?
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> ResolvedRef {
        let object = try decodeDirectoryObject(raw, label: "resolved ref JSON")
        let projection = try ResolvedRef(
            profile: requiredDirectoryString(object, "profile"),
            kind: requiredDirectoryString(object, "kind"),
            answerKind: requiredDirectoryString(object, "answer_kind"),
            queryName: optionalDirectoryJSONString(object["query_name"], "query_name"),
            canonicalName: optionalDirectoryJSONString(object["canonical_name"], "canonical_name"),
            ownerURA: optionalDirectoryJSONString(object["owner_ura"], "owner_ura"),
            abilityURA: optionalDirectoryJSONString(object["ability_ura"], "ability_ura"),
            routeURA: optionalDirectoryJSONString(object["route_ura"], "route_ura"),
            nextHop: optionalDirectoryObject(object["next_hop"], "next_hop"),
            selectedRoute: optionalDirectoryObject(object["selected_route"], "selected_route"),
            routeCandidates: requiredDirectoryArray(object, "route_candidates"),
            records: requiredDirectoryArray(object, "records"),
            negative: optionalDirectoryObject(object["negative"], "negative"),
            releaseProfile: optionalDirectoryJSONString(object["release_profile"], "release_profile"),
            authority: optionalDirectoryObject(object["authority"], "authority"),
            cachePolicy: optionalDirectoryObject(object["cache_policy"], "cache_policy"),
            metadata: requiredDirectoryObject(object, "metadata")
        )
        guard projection.profile == directoryIdentityProfile, projection.kind == "resolved_ref" else {
            throw invalidDirectory("resolved ref projection mismatch")
        }
        return projection
    }
}

public protocol DirectoryTransport: AnyObject, Sendable {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data
    func buildListDevicesInvocation(_ requestJSON: Data) async throws -> Data
    func buildListAgentsInvocation(_ requestJSON: Data) async throws -> Data
    func buildListAbilitiesInvocation(_ requestJSON: Data) async throws -> Data
    func buildResolveInvocation(_ requestJSON: Data) async throws -> Data
    func listDevices(_ requestJSON: Data) async throws -> Data
    func listAgents(_ requestJSON: Data) async throws -> Data
    func listAbilities(_ requestJSON: Data) async throws -> Data
    func resolve(_ requestJSON: Data) async throws -> Data
    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource
    func projectSubscription(_ subscriptionJSON: Data) async throws -> Data
    func close() async throws
}

public extension DirectoryTransport {
    func buildDirectorySubscriptionInvocation(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory subscription invocation transport is not available") }
    func buildListDevicesInvocation(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-devices invocation transport is not available") }
    func buildListAgentsInvocation(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-agents invocation transport is not available") }
    func buildListAbilitiesInvocation(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-abilities invocation transport is not available") }
    func buildResolveInvocation(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory resolve invocation transport is not available") }
    func listDevices(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-devices transport is not available") }
    func listAgents(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-agents transport is not available") }
    func listAbilities(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory list-abilities transport is not available") }
    func resolve(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("directory resolve transport is not available") }
    func subscribeDirectory(_ requestJSON: Data) async throws -> StreamSource { throw directoryUnsupported("directory subscribe transport is not available") }
    func projectSubscription(_ subscriptionJSON: Data) async throws -> Data { throw directoryUnsupported("directory project subscription transport is not available") }
    func close() async throws {}
}

public final class DirectoryClient: @unchecked Sendable {
    private let transport: DirectoryTransport
    private var closed = false

    public init(transport: DirectoryTransport) {
        self.transport = transport
    }

    public func buildListDevicesInvocation(_ query: DirectoryQueryBase) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildListDevicesInvocation(query.jsonData(requireLimit: true)) }
    }

    public func buildListAgentsInvocation(_ query: DirectoryQueryBase) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildListAgentsInvocation(query.jsonData(requireLimit: true)) }
    }

    public func buildListAbilitiesInvocation(_ query: AbilityQuery) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildListAbilitiesInvocation(query.jsonData()) }
    }

    public func buildResolveInvocation(_ query: ResolveQuery) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildResolveInvocation(query.jsonData()) }
    }

    public func buildDirectorySubscriptionInvocation(_ request: DirectorySubscriptionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildDirectorySubscriptionInvocation(request.jsonData()) }
    }

    public func subscribeDirectory(_ request: DirectorySubscriptionRequest) async throws -> StreamHandle {
        try requireOpen()
        do {
            return StreamHandle(source: try await transport.subscribeDirectory(request.jsonData()))
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "directory subscribe transport failed",
                details: ["profile": directoryIdentityProfile]
            )
        }
    }

    public func projectSubscription(_ subscriptionJSON: Data) async throws -> DirectorySubscription {
        let data = try await raw { try await transport.projectSubscription(subscriptionJSON) }
        return try DirectorySubscription.fromJSON(data)
    }

    public func listDevices(_ query: DirectoryQueryBase) async throws -> DirectoryPage {
        let data = try await raw { try await transport.listDevices(query.jsonData(requireLimit: true)) }
        return try DirectoryPage.fromJSON(data, expectedKind: "device_page", expectedItemKind: "device")
    }

    public func listAgents(_ query: DirectoryQueryBase) async throws -> DirectoryPage {
        let data = try await raw { try await transport.listAgents(query.jsonData(requireLimit: true)) }
        return try DirectoryPage.fromJSON(data, expectedKind: "agent_page", expectedItemKind: "agent")
    }

    public func listAbilities(_ query: AbilityQuery) async throws -> DirectoryPage {
        let data = try await raw { try await transport.listAbilities(query.jsonData()) }
        return try DirectoryPage.fromJSON(data, expectedKind: "ability_page", expectedItemKind: "ability")
    }

    public func resolve(_ query: ResolveQuery) async throws -> ResolvedRef {
        let data = try await raw { try await transport.resolve(query.jsonData()) }
        return try ResolvedRef.fromJSON(data)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        let data = try await raw(call)
        return try decodeDirectoryObject(data, label: "directory invocation JSON")
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "directory transport failed",
                details: ["profile": directoryIdentityProfile]
            )
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("directory")
        }
    }
}

public struct DescriptorRefRequest: Sendable, Equatable {
    public let descriptorRef: String
    public let metadata: [String: JSONValue]

    public init(descriptorRef: String, metadata: [String: JSONValue] = [:]) throws {
        self.descriptorRef = try cleanDirectoryString(descriptorRef, "descriptor_ref")
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = ["descriptor_ref": .string(descriptorRef)]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public struct DescriptorRefBuildRequest: Sendable, Equatable {
    public let abilityURA: String
    public let descriptorVersion: String
    public let metadata: [String: JSONValue]

    public init(abilityURA: String, descriptorVersion: String, metadata: [String: JSONValue] = [:]) throws {
        self.abilityURA = try cleanDirectoryString(abilityURA, "ability_ura")
        self.descriptorVersion = try cleanDirectoryString(descriptorVersion, "descriptor_version")
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [
            "ability_ura": .string(abilityURA),
            "descriptor_version": .string(descriptorVersion),
        ]
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public struct IdentityProjection: Sendable, Equatable {
    public let kind: String
    public let valid: Bool
    public let ura: String
    public let realm: String
    public let displayID: String
    public let descriptorRef: String
    public let abilityURA: String
    public let descriptorVersion: String
    public let profile: String
    public let components: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> IdentityProjection {
        let object = try decodeDirectoryObject(raw, label: "identity projection JSON")
        return try IdentityProjection(
            kind: requiredDirectoryString(object, "kind"),
            valid: requiredDirectoryBool(object, "valid"),
            ura: optionalDirectoryJSONString(object["ura"], "ura") ?? "",
            realm: optionalDirectoryJSONString(object["realm"], "realm") ?? "",
            displayID: optionalDirectoryJSONString(object["display_id"], "display_id") ?? "",
            descriptorRef: optionalDirectoryJSONString(object["descriptor_ref"], "descriptor_ref") ?? "",
            abilityURA: optionalDirectoryJSONString(object["ability_ura"], "ability_ura") ?? "",
            descriptorVersion: optionalDirectoryJSONString(object["descriptor_version"], "descriptor_version") ?? "",
            profile: requiredDirectoryString(object, "profile"),
            components: requiredDirectoryObject(object, "components"),
            metadata: requiredDirectoryObject(object, "metadata")
        )
    }
}

public protocol IdentityTransport: AnyObject, Sendable {
    func projectDescriptorRef(_ requestJSON: Data) async throws -> Data
    func buildDescriptorRef(_ requestJSON: Data) async throws -> Data
    func ownerAbilityURA(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension IdentityTransport {
    func projectDescriptorRef(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("identity descriptor projection transport is not available") }
    func buildDescriptorRef(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("identity descriptor build transport is not available") }
    func ownerAbilityURA(_ requestJSON: Data) async throws -> Data { throw directoryUnsupported("identity owner ability transport is not available") }
    func close() async throws {}
}

public final class IdentityClient: @unchecked Sendable {
    private let transport: IdentityTransport
    private var closed = false

    public init(transport: IdentityTransport) {
        self.transport = transport
    }

    public func projectDescriptorRef(_ request: DescriptorRefRequest) async throws -> IdentityProjection {
        let data = try await raw { try await transport.projectDescriptorRef(request.jsonData()) }
        return try IdentityProjection.fromJSON(data)
    }

    public func buildDescriptorRef(_ request: DescriptorRefBuildRequest) async throws -> IdentityProjection {
        let data = try await raw { try await transport.buildDescriptorRef(request.jsonData()) }
        return try IdentityProjection.fromJSON(data)
    }

    public func canonicalAbilityDescriptorRef(_ value: String, descriptorVersion: String = "") async throws -> String {
        _ = try cleanDirectoryString(value, descriptorVersion.isEmpty ? "descriptor_ref" : "ability_ura")
        if !descriptorVersion.isEmpty {
            let projection = try await buildDescriptorRef(
                DescriptorRefBuildRequest(abilityURA: value, descriptorVersion: descriptorVersion)
            )
            return try requiredProjectionString(projection.descriptorRef, "descriptor_ref")
        }
        let projection = try await projectDescriptorRef(DescriptorRefRequest(descriptorRef: value))
        return try requiredProjectionString(projection.descriptorRef, "descriptor_ref")
    }

    public func abilityURAFromDescriptorRef(_ descriptorRef: String) async throws -> String {
        let projection = try await projectDescriptorRef(DescriptorRefRequest(descriptorRef: descriptorRef))
        return try requiredProjectionString(projection.abilityURA, "ability_ura")
    }

    public func ownerAbilityURA(ownerURA: String, abilityName: String) async throws -> String {
        let object: [String: JSONValue] = [
            "owner_ura": .string(try cleanDirectoryString(ownerURA, "owner_ura")),
            "ability_name": .string(try cleanDirectoryString(abilityName, "ability_name")),
        ]
        let data = try await raw { try await transport.ownerAbilityURA(encodeJSONObject(object)) }
        let projection = try decodeDirectoryObject(data, label: "owner ability projection JSON")
        if let ability = try optionalDirectoryJSONString(projection["ability_ura"], "ability_ura"), !ability.isEmpty {
            return ability
        }
        if let ura = try optionalDirectoryJSONString(projection["ura"], "ura"), !ura.isEmpty {
            return ura
        }
        throw invalidDirectory("ability_ura is required")
    }

    public func ownerAbilityDescriptorRef(ownerURA: String, abilityName: String, descriptorVersion: String) async throws -> String {
        let ability = try await ownerAbilityURA(ownerURA: ownerURA, abilityName: abilityName)
        return try await canonicalAbilityDescriptorRef(ability, descriptorVersion: descriptorVersion)
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
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "identity transport failed",
                details: ["profile": directoryIdentityProfile]
            )
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("identity")
        }
    }
}

func encodeJSONObject(_ object: [String: JSONValue]) throws -> Data {
    let compatible = object.mapValues(jsonCompatible)
    return try JSONSerialization.data(withJSONObject: compatible, options: [.sortedKeys])
}

func jsonCompatible(_ value: JSONValue) -> Any {
    switch value {
    case .null:
        return NSNull()
    case let .bool(value):
        return value
    case let .number(value):
        return value
    case let .string(value):
        return value
    case let .array(values):
        return values.map(jsonCompatible)
    case let .object(object):
        return object.mapValues(jsonCompatible)
    }
}

func decodeDirectoryObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidDirectory("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

func requiredDirectoryString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty {
        return value
    }
    throw invalidDirectory("\(name) must be a non-empty string")
}

func requiredDirectoryBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidDirectory("\(name) must be a boolean")
}

func requiredDirectoryInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if case let .number(value) = object[name],
       value >= 0,
       value.rounded() == value,
       value <= Double(Int.max)
    {
        return Int(value)
    }
    throw invalidDirectory("\(name) must be a non-negative integer")
}

func optionalDirectoryJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidDirectory("\(name) must be a string or null")
    }
}

func requiredDirectoryArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    if case let .array(value) = object[name] {
        return value
    }
    throw invalidDirectory("\(name) must be a list")
}

func requiredDirectoryObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] {
        return value
    }
    throw invalidDirectory("\(name) must be an object")
}

func optionalDirectoryObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidDirectory("\(name) must be an object or null")
    }
}

func cleanDirectoryString(_ value: String, _ field: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidDirectory("\(field) is required")
    }
    return value
}

func optionalDirectoryString(_ value: String, _ field: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidDirectory("\(field) must not contain surrounding whitespace")
    }
    return value
}

func requiredProjectionString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty else {
        throw invalidDirectory("\(field) is required")
    }
    return value
}

func invalidDirectory(_ message: String) -> SDKError {
    SDKError(
        code: .invalidArgument,
        stage: "directory_identity",
        message: message,
        details: ["profile": directoryIdentityProfile]
    )
}

func directoryUnsupported(_ message: String) -> SDKError {
    SDKError(
        code: .notImplemented,
        stage: "transport",
        message: message,
        details: ["profile": directoryIdentityProfile]
    )
}
