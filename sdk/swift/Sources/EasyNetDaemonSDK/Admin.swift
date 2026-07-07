import Foundation

public let adminGatewayProfile = "admin_gateway"

public struct AdminCarrierBase: Sendable, Equatable {
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
        self.callerURA = try requiredAdminString(callerURA, "caller_ura")
        self.calleeURA = try requiredAdminString(calleeURA, "callee_ura")
        self.subjectURA = try requiredAdminString(subjectURA, "subject_ura")
        self.descriptorVersion = try requiredAdminString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try requiredAdminString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else { throw invalidAdmin("causal_context is required") }
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

public struct AdminAgentListRequest: Sendable, Equatable {
    public let base: AdminCarrierBase

    public init(base: AdminCarrierBase) {
        self.base = base
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        return try encodeJSONObject(object)
    }
}

public struct AdminAgentStartRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let name: String
    public let agentType: String
    public let entry: [String: JSONValue]
    public let model: String
    public let label: String

    public init(
        base: AdminCarrierBase,
        name: String,
        agentType: String = "",
        entry: [String: JSONValue] = [:],
        model: String = "",
        label: String = ""
    ) throws {
        self.base = base
        self.name = try adminAgentName(name)
        self.agentType = try optionalAdminString(agentType, "agent_type")
        self.entry = entry
        guard !self.agentType.isEmpty || !entry.isEmpty else {
            throw invalidAdmin("either agent_type or entry is required")
        }
        self.model = try optionalAdminString(model, "model")
        self.label = try optionalAdminString(label, "label")
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["name"] = .string(name)
        if !agentType.isEmpty { object["agent_type"] = .string(agentType) }
        if !entry.isEmpty { object["entry"] = .object(entry) }
        if !model.isEmpty { object["model"] = .string(model) }
        if !label.isEmpty { object["label"] = .string(label) }
        return try encodeJSONObject(object)
    }
}

public struct AdminAgentStopRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let name: String
    public let agentURA: String

    public init(base: AdminCarrierBase, name: String = "", agentURA: String = "") throws {
        self.base = base
        self.name = name.isEmpty ? "" : try adminAgentName(name)
        self.agentURA = agentURA.isEmpty ? "" : try adminAgentURA(agentURA)
        guard !self.name.isEmpty || !self.agentURA.isEmpty else {
            throw invalidAdmin("either name or agent_ura is required")
        }
        if !self.name.isEmpty, !self.agentURA.isEmpty, !self.agentURA.hasSuffix(".\(self.name)") {
            throw invalidAdmin("agent_ura must name the same hosted agent as name")
        }
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        if !name.isEmpty { object["name"] = .string(name) }
        if !agentURA.isEmpty { object["agent_ura"] = .string(agentURA) }
        return try encodeJSONObject(object)
    }
}

public struct AdminAgentRefreshRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let name: String

    public init(base: AdminCarrierBase, name: String = "") throws {
        self.base = base
        self.name = name.isEmpty ? "" : try adminAgentName(name)
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        if !name.isEmpty { object["name"] = .string(name) }
        return try encodeJSONObject(object)
    }
}

public struct AdminSessionListRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let includeTerminated: Bool?

    public init(base: AdminCarrierBase, includeTerminated: Bool? = nil) {
        self.base = base
        self.includeTerminated = includeTerminated
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        if let includeTerminated { object["include_terminated"] = .bool(includeTerminated) }
        return try encodeJSONObject(object)
    }
}

public struct PairingPreflightRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let hubURA: String
    public let deviceURA: String
    public let requestedScopes: [String]

    public init(base: AdminCarrierBase, hubURA: String, deviceURA: String, requestedScopes: [String] = []) throws {
        self.base = base
        self.hubURA = try adminHubURA(hubURA)
        self.deviceURA = try adminDeviceURA(deviceURA)
        self.requestedScopes = try requestedScopes.map { try adminIdentifier($0, "scope") }
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["hub_ura"] = .string(hubURA)
        object["device_ura"] = .string(deviceURA)
        if !requestedScopes.isEmpty { object["requested_scopes"] = .array(requestedScopes.map(JSONValue.string)) }
        return try encodeJSONObject(object)
    }
}

public struct CreatePairingRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let hubURA: String
    public let deviceURA: String
    public let expiresUnixMS: Int64
    public let scopes: [String]

    public init(base: AdminCarrierBase, hubURA: String, deviceURA: String, expiresUnixMS: Int64, scopes: [String] = []) throws {
        self.base = base
        self.hubURA = try adminHubURA(hubURA)
        self.deviceURA = try adminDeviceURA(deviceURA)
        guard expiresUnixMS > 0 else { throw invalidAdmin("expires_unix_ms is required") }
        self.expiresUnixMS = expiresUnixMS
        self.scopes = try scopes.map { try adminIdentifier($0, "scope") }
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["hub_ura"] = .string(hubURA)
        object["device_ura"] = .string(deviceURA)
        object["expires_unix_ms"] = .number(Double(expiresUnixMS))
        if !scopes.isEmpty { object["scopes"] = .array(scopes.map(JSONValue.string)) }
        return try encodeJSONObject(object)
    }
}

public struct ValidatePairingRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let token: String
    public let deviceURA: String

    public init(base: AdminCarrierBase, token: String, deviceURA: String) throws {
        self.base = base
        self.token = try adminIdentifier(token, "token")
        self.deviceURA = try adminDeviceURA(deviceURA)
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["token"] = .string(token)
        object["device_ura"] = .string(deviceURA)
        return try encodeJSONObject(object)
    }
}

public struct CreateDeviceSessionRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let deviceURA: String
    public let hubURA: String
    public let sessionKind: String
    public let expiresUnixMS: Int64

    public init(base: AdminCarrierBase, deviceURA: String, hubURA: String, sessionKind: String, expiresUnixMS: Int64 = 0) throws {
        self.base = base
        self.deviceURA = try adminDeviceURA(deviceURA)
        self.hubURA = try adminHubURA(hubURA)
        self.sessionKind = try adminIdentifier(sessionKind, "session_kind")
        guard expiresUnixMS >= 0 else { throw invalidAdmin("expires_unix_ms must be non-negative") }
        self.expiresUnixMS = expiresUnixMS
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["device_ura"] = .string(deviceURA)
        object["hub_ura"] = .string(hubURA)
        object["session_kind"] = .string(sessionKind)
        if expiresUnixMS > 0 { object["expires_unix_ms"] = .number(Double(expiresUnixMS)) }
        return try encodeJSONObject(object)
    }
}

public struct DeleteDeviceSessionRequest: Sendable, Equatable {
    public let base: AdminCarrierBase
    public let sessionID: String
    public let reason: String

    public init(base: AdminCarrierBase, sessionID: String, reason: String = "") throws {
        self.base = base
        self.sessionID = try adminIdentifier(sessionID, "session_id")
        self.reason = reason.isEmpty ? "" : try adminReason(reason)
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        base.write(to: &object)
        object["session_id"] = .string(sessionID)
        if !reason.isEmpty { object["reason"] = .string(reason) }
        return try encodeJSONObject(object)
    }
}

public struct AdminGatewayStatusRequest: Sendable, Equatable {
    public let requirePublicListener: Bool?
    public let metadata: [String: JSONValue]

    public init(requirePublicListener: Bool? = nil, metadata: [String: JSONValue] = [:]) {
        self.requirePublicListener = requirePublicListener
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object: [String: JSONValue] = [:]
        if let requirePublicListener { object["require_public_listener"] = .bool(requirePublicListener) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }
}

public struct GatewayStatus: Sendable, Equatable {
    public let profile: String
    public let gatewayID: String
    public let ready: Bool
    public let state: String
    public let processLive: Bool
    public let controlReady: Bool
    public let runtimeReady: Bool
    public let directoryReady: Bool
    public let trustReady: Bool
    public let publicListenerReady: Bool
    public let listeners: [GatewayListener]
    public let identity: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> GatewayStatus {
        let object = try decodeAdminObject(raw, label: "gateway status JSON")
        return try GatewayStatus(
            profile: requiredAdminString(object, "profile"),
            gatewayID: requiredAdminString(object, "gateway_id"),
            ready: requiredAdminBool(object, "ready"),
            state: requiredAdminString(object, "state"),
            processLive: requiredAdminBool(object, "process_live"),
            controlReady: requiredAdminBool(object, "control_ready"),
            runtimeReady: requiredAdminBool(object, "runtime_ready"),
            directoryReady: requiredAdminBool(object, "directory_ready"),
            trustReady: requiredAdminBool(object, "trust_ready"),
            publicListenerReady: requiredAdminBool(object, "public_listener_ready"),
            listeners: try requiredAdminArray(object, "listeners").map { try GatewayListener.fromJSONValue($0) },
            identity: requiredAdminObject(object, "identity"),
            metadata: requiredAdminObject(object, "metadata")
        )
    }

    public init(profile: String, gatewayID: String, ready: Bool, state: String, processLive: Bool, controlReady: Bool, runtimeReady: Bool, directoryReady: Bool, trustReady: Bool, publicListenerReady: Bool, listeners: [GatewayListener], identity: [String: JSONValue], metadata: [String: JSONValue]) throws {
        guard profile == adminGatewayProfile else { throw invalidAdmin("invalid gateway status projection") }
        self.profile = profile
        self.gatewayID = try requiredAdminString(gatewayID, "gateway_id")
        self.ready = ready
        self.state = try requiredAdminString(state, "state")
        self.processLive = processLive
        self.controlReady = controlReady
        self.runtimeReady = runtimeReady
        self.directoryReady = directoryReady
        self.trustReady = trustReady
        self.publicListenerReady = publicListenerReady
        self.listeners = listeners
        self.identity = identity
        guard !metadata.isEmpty else { throw invalidAdmin("metadata must be an object") }
        self.metadata = metadata
    }
}

public struct GatewayListener: Sendable, Equatable {
    public let kind: String
    public let endpoint: String
    public let ready: Bool
    public let isPublic: Bool

    public init(kind: String, endpoint: String, ready: Bool, isPublic: Bool) throws {
        self.kind = try requiredAdminString(kind, "kind")
        self.endpoint = try requiredAdminString(endpoint, "endpoint")
        self.ready = ready
        self.isPublic = isPublic
    }

    static func fromJSONValue(_ value: JSONValue) throws -> GatewayListener {
        guard case let .object(object) = value else { throw invalidAdmin("listeners items must be objects") }
        return try GatewayListener(
            kind: requiredAdminString(object, "kind"),
            endpoint: requiredAdminString(object, "endpoint"),
            ready: requiredAdminBool(object, "ready"),
            isPublic: requiredAdminBool(object, "public")
        )
    }
}

public struct AdminAgentRecord: Sendable, Equatable {
    public let name: String
    public let agentURA: String?
    public let ownerURA: String?
    public let deviceURA: String?
    public let state: String
    public let runtime: String
    public let model: String?
    public let label: String?
    public let abilities: [JSONValue]
    public let metadata: [String: JSONValue]

    public init(name: String, agentURA: String?, ownerURA: String?, deviceURA: String?, state: String, runtime: String, model: String?, label: String?, abilities: [JSONValue], metadata: [String: JSONValue]) throws {
        self.name = try adminAgentName(name)
        self.agentURA = agentURA
        self.ownerURA = ownerURA
        self.deviceURA = deviceURA
        self.state = try requiredAdminString(state, "state")
        self.runtime = try requiredAdminString(runtime, "runtime")
        self.model = model
        self.label = label
        self.abilities = abilities
        guard !metadata.isEmpty else { throw invalidAdmin("agent metadata must be an object") }
        self.metadata = metadata
    }

    static func fromJSONValue(_ value: JSONValue) throws -> AdminAgentRecord {
        guard case let .object(object) = value else { throw invalidAdmin("agent items must be objects") }
        return try AdminAgentRecord(
            name: requiredAdminString(object, "name"),
            agentURA: optionalAdminJSONString(object["agent_ura"], "agent_ura"),
            ownerURA: optionalAdminJSONString(object["owner_ura"], "owner_ura"),
            deviceURA: optionalAdminJSONString(object["device_ura"], "device_ura"),
            state: requiredAdminString(object, "state"),
            runtime: requiredAdminString(object, "runtime"),
            model: optionalAdminJSONString(object["model"], "model"),
            label: optionalAdminJSONString(object["label"], "label"),
            abilities: requiredAdminArray(object, "abilities"),
            metadata: requiredAdminObject(object, "metadata")
        )
    }
}

public struct AdminAgentPage: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let state: String
    public let items: [AdminAgentRecord]
    public let nextCursor: JSONValue?
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> AdminAgentPage {
        let object = try decodeAdminObject(raw, label: "admin agent page JSON")
        return try AdminAgentPage(
            profile: requiredAdminString(object, "profile"),
            kind: requiredAdminString(object, "kind"),
            state: requiredAdminString(object, "state"),
            items: try requiredAdminArray(object, "items").map { try AdminAgentRecord.fromJSONValue($0) },
            nextCursor: object["next_cursor"] == .null ? nil : object["next_cursor"],
            metadata: requiredAdminObject(object, "metadata")
        )
    }

    public init(profile: String, kind: String, state: String, items: [AdminAgentRecord], nextCursor: JSONValue?, metadata: [String: JSONValue]) throws {
        guard profile == adminGatewayProfile, kind == "agent_records" else { throw invalidAdmin("invalid admin agent page projection") }
        self.profile = profile
        self.kind = kind
        self.state = try requiredAdminString(state, "state")
        self.items = items
        self.nextCursor = nextCursor
        self.metadata = metadata
    }
}

public struct AdminGatewayResult: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let operation: String
    public let state: String
    public let agentURA: String?
    public let deviceURA: String?
    public let ack: Bool?
    public let runtimeNotReady: Bool
    public let runtimeCatalogNotReady: Bool
    public let nextCursor: JSONValue?
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> AdminGatewayResult {
        let object = try decodeAdminObject(raw, label: "admin result JSON")
        return try AdminGatewayResult(
            profile: requiredAdminString(object, "profile"),
            kind: requiredAdminString(object, "kind"),
            operation: optionalAdminJSONString(object["operation"], "operation") ?? "",
            state: requiredAdminString(object, "state"),
            agentURA: optionalAdminJSONString(object["agent_ura"], "agent_ura"),
            deviceURA: optionalAdminJSONString(object["device_ura"], "device_ura"),
            ack: optionalAdminBool(object["ack"], "ack"),
            runtimeNotReady: optionalAdminBool(object["runtime_not_ready"], "runtime_not_ready") ?? false,
            runtimeCatalogNotReady: optionalAdminBool(object["runtime_catalog_not_ready"], "runtime_catalog_not_ready") ?? false,
            nextCursor: object["next_cursor"] == .null ? nil : object["next_cursor"],
            metadata: requiredAdminObject(object, "metadata")
        )
    }

    public init(profile: String, kind: String, operation: String = "", state: String, agentURA: String?, deviceURA: String?, ack: Bool?, runtimeNotReady: Bool = false, runtimeCatalogNotReady: Bool = false, nextCursor: JSONValue? = nil, metadata: [String: JSONValue]) throws {
        guard profile == adminGatewayProfile else { throw invalidAdmin("invalid admin result projection") }
        self.profile = profile
        self.kind = try requiredAdminString(kind, "kind")
        self.operation = operation
        self.state = try requiredAdminString(state, "state")
        self.agentURA = agentURA
        self.deviceURA = deviceURA
        self.ack = ack
        self.runtimeNotReady = runtimeNotReady
        self.runtimeCatalogNotReady = runtimeCatalogNotReady
        self.nextCursor = nextCursor
        guard !metadata.isEmpty else { throw invalidAdmin("metadata must be an object") }
        self.metadata = metadata
    }
}

public struct PairingPreflight: Sendable, Equatable {
    public let pairingRequired: Bool
    public let trustReady: Bool
    public let scopes: [String]

    public static func fromJSON(_ raw: Data) throws -> PairingPreflight {
        let object = try decodeAdminObject(raw, label: "pairing preflight JSON")
        try validateAdminKind(object, "pairing_preflight")
        return try PairingPreflight(
            pairingRequired: requiredAdminBool(object, "pairing_required"),
            trustReady: requiredAdminBool(object, "trust_ready"),
            scopes: requiredAdminStringArray(object, "scopes")
        )
    }
}

public struct PairingToken: Sendable, Equatable {
    public let tokenID: String
    public let token: String
    public let scopes: [String]

    public static func fromJSON(_ raw: Data) throws -> PairingToken {
        let object = try decodeAdminObject(raw, label: "pairing token JSON")
        try validateAdminKind(object, "pairing_token")
        return try PairingToken(
            tokenID: requiredAdminString(object, "token_id"),
            token: requiredAdminString(object, "token"),
            scopes: requiredAdminStringArray(object, "scopes")
        )
    }
}

public struct DeviceCredential: Sendable, Equatable {
    public let credentialID: String
    public let deviceURA: String
    public let hubURA: String

    public static func fromJSON(_ raw: Data) throws -> DeviceCredential {
        let object = try decodeAdminObject(raw, label: "device credential JSON")
        try validateAdminKind(object, "device_credential")
        return try DeviceCredential(
            credentialID: requiredAdminString(object, "credential_id"),
            deviceURA: adminDeviceURA(requiredAdminString(object, "device_ura")),
            hubURA: adminHubURA(requiredAdminString(object, "hub_ura"))
        )
    }
}

public struct DeviceSession: Sendable, Equatable {
    public let sessionID: String
    public let sessionKind: String

    public static func fromJSON(_ raw: Data) throws -> DeviceSession {
        let object = try decodeAdminObject(raw, label: "device session JSON")
        return try fromObject(object)
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> DeviceSession {
        try validateAdminKind(object, "device_session")
        return try DeviceSession(
            sessionID: adminIdentifier(requiredAdminString(object, "session_id"), "session_id"),
            sessionKind: adminIdentifier(requiredAdminString(object, "session_kind"), "session_kind")
        )
    }
}

public struct DeviceSessionPage: Sendable, Equatable {
    public let items: [DeviceSession]

    public static func fromJSON(_ raw: Data) throws -> DeviceSessionPage {
        let object = try decodeAdminObject(raw, label: "device session page JSON")
        try validateAdminKind(object, "device_sessions")
        let items = try requiredAdminArray(object, "items").map { value -> DeviceSession in
            guard case let .object(item) = value else { throw invalidAdmin("device session items must be objects") }
            return try DeviceSession.fromObject(item)
        }
        return DeviceSessionPage(items: items)
    }
}

public protocol AdminTransport: AnyObject, Sendable {
    func buildAgentListInvocation(_ requestJSON: Data) async throws -> Data
    func buildAgentStartInvocation(_ requestJSON: Data) async throws -> Data
    func buildAgentStopInvocation(_ requestJSON: Data) async throws -> Data
    func buildAgentRefreshInvocation(_ requestJSON: Data) async throws -> Data
    func buildSessionListInvocation(_ requestJSON: Data) async throws -> Data
    func gatewayStatus(_ requestJSON: Data) async throws -> Data
    func listAgents(_ requestJSON: Data) async throws -> Data
    func agentStart(_ requestJSON: Data) async throws -> Data
    func agentStop(_ requestJSON: Data) async throws -> Data
    func agentRefresh(_ requestJSON: Data) async throws -> Data
    func pairingPreflight(_ requestJSON: Data) async throws -> Data
    func createPairing(_ requestJSON: Data) async throws -> Data
    func validatePairing(_ requestJSON: Data) async throws -> Data
    func createDeviceSession(_ requestJSON: Data) async throws -> Data
    func listDeviceSessions(_ requestJSON: Data) async throws -> Data
    func deleteDeviceSession(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension AdminTransport {
    func buildAgentListInvocation(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-list invocation transport is not available") }
    func buildAgentStartInvocation(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-start invocation transport is not available") }
    func buildAgentStopInvocation(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-stop invocation transport is not available") }
    func buildAgentRefreshInvocation(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-refresh invocation transport is not available") }
    func buildSessionListInvocation(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin session-list invocation transport is not available") }
    func gatewayStatus(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin gateway-status transport is not available") }
    func listAgents(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin list-agents transport is not available") }
    func agentStart(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-start transport is not available") }
    func agentStop(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-stop transport is not available") }
    func agentRefresh(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin agent-refresh transport is not available") }
    func pairingPreflight(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin pairing-preflight transport is not available") }
    func createPairing(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin create-pairing transport is not available") }
    func validatePairing(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin validate-pairing transport is not available") }
    func createDeviceSession(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin create-device-session transport is not available") }
    func listDeviceSessions(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin list-device-sessions transport is not available") }
    func deleteDeviceSession(_ requestJSON: Data) async throws -> Data { throw adminUnsupported("admin delete-device-session transport is not available") }
    func close() async throws {}
}

public final class AdminClient: @unchecked Sendable {
    private let transport: AdminTransport
    private var closed = false

    public init(transport: AdminTransport) {
        self.transport = transport
    }

    public func buildAgentListInvocation(_ request: AdminAgentListRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildAgentListInvocation(request.jsonData()) }
    }

    public func buildAgentStartInvocation(_ request: AdminAgentStartRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildAgentStartInvocation(request.jsonData()) }
    }

    public func buildAgentStopInvocation(_ request: AdminAgentStopRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildAgentStopInvocation(request.jsonData()) }
    }

    public func buildAgentRefreshInvocation(_ request: AdminAgentRefreshRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildAgentRefreshInvocation(request.jsonData()) }
    }

    public func buildSessionListInvocation(_ request: AdminSessionListRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildSessionListInvocation(request.jsonData()) }
    }

    public func gatewayStatus(_ request: AdminGatewayStatusRequest = AdminGatewayStatusRequest()) async throws -> GatewayStatus {
        try await GatewayStatus.fromJSON(raw { try await transport.gatewayStatus(request.jsonData()) })
    }

    public func listAgents(_ request: AdminAgentListRequest) async throws -> AdminAgentPage {
        try await AdminAgentPage.fromJSON(raw { try await transport.listAgents(request.jsonData()) })
    }

    public func agentStart(_ request: AdminAgentStartRequest) async throws -> AdminGatewayResult {
        try await AdminGatewayResult.fromJSON(raw { try await transport.agentStart(request.jsonData()) })
    }

    public func agentStop(_ request: AdminAgentStopRequest) async throws -> AdminGatewayResult {
        try await AdminGatewayResult.fromJSON(raw { try await transport.agentStop(request.jsonData()) })
    }

    public func agentRefresh(_ request: AdminAgentRefreshRequest) async throws -> AdminGatewayResult {
        try await AdminGatewayResult.fromJSON(raw { try await transport.agentRefresh(request.jsonData()) })
    }

    public func pairingPreflight(_ request: PairingPreflightRequest) async throws -> PairingPreflight {
        try await PairingPreflight.fromJSON(raw { try await transport.pairingPreflight(request.jsonData()) })
    }

    public func createPairing(_ request: CreatePairingRequest) async throws -> PairingToken {
        try await PairingToken.fromJSON(raw { try await transport.createPairing(request.jsonData()) })
    }

    public func validatePairing(_ request: ValidatePairingRequest) async throws -> DeviceCredential {
        try await DeviceCredential.fromJSON(raw { try await transport.validatePairing(request.jsonData()) })
    }

    public func createDeviceSession(_ request: CreateDeviceSessionRequest) async throws -> DeviceSession {
        try await DeviceSession.fromJSON(raw { try await transport.createDeviceSession(request.jsonData()) })
    }

    public func listDeviceSessions(_ request: AdminSessionListRequest) async throws -> DeviceSessionPage {
        try await DeviceSessionPage.fromJSON(raw { try await transport.listDeviceSessions(request.jsonData()) })
    }

    public func deleteDeviceSession(_ request: DeleteDeviceSessionRequest) async throws -> AdminGatewayResult {
        try await AdminGatewayResult.fromJSON(raw { try await transport.deleteDeviceSession(request.jsonData()) })
    }

    public func projectGatewayStatus(_ raw: Data) throws -> GatewayStatus {
        try GatewayStatus.fromJSON(raw)
    }

    public func projectAgentRecords(_ raw: Data) throws -> AdminAgentPage {
        try AdminAgentPage.fromJSON(raw)
    }

    public func projectAgentLifecycleResult(_ raw: Data) throws -> AdminGatewayResult {
        try AdminGatewayResult.fromJSON(raw)
    }

    public func projectPairingPreflight(_ raw: Data) throws -> PairingPreflight {
        try PairingPreflight.fromJSON(raw)
    }

    public func projectPairingToken(_ raw: Data) throws -> PairingToken {
        try PairingToken.fromJSON(raw)
    }

    public func projectDeviceCredential(_ raw: Data) throws -> DeviceCredential {
        try DeviceCredential.fromJSON(raw)
    }

    public func projectDeviceSession(_ raw: Data) throws -> DeviceSession {
        try DeviceSession.fromJSON(raw)
    }

    public func projectDeviceSessionPage(_ raw: Data) throws -> DeviceSessionPage {
        try DeviceSessionPage.fromJSON(raw)
    }

    public func projectDeviceAdminResult(_ raw: Data) throws -> AdminGatewayResult {
        try AdminGatewayResult.fromJSON(raw)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodeAdminObject(try await raw(call), label: "admin invocation JSON")
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw adminTransport("admin transport failed")
        }
    }

    private func requireOpen() throws {
        if closed { throw SDKError.closed(adminGatewayProfile) }
    }
}

private func decodeAdminObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else { throw invalidAdmin("\(label) must be an object") }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)", details: ["profile": adminGatewayProfile])
    }
}

private func validateAdminKind(_ object: [String: JSONValue], _ kind: String) throws {
    guard try requiredAdminString(object, "profile") == adminGatewayProfile,
          try requiredAdminString(object, "kind") == kind
    else {
        throw invalidAdmin("invalid admin projection kind")
    }
}

private func requiredAdminString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidAdmin("\(field) is required")
    }
    return value
}

private func optionalAdminString(_ value: String, _ field: String) throws -> String {
    if value.isEmpty { return "" }
    return try requiredAdminString(value, field)
}

private func requiredAdminString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty { return value }
    throw invalidAdmin("\(name) must be a non-empty string")
}

private func optionalAdminJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidAdmin("\(name) must be a string or null")
    }
}

private func requiredAdminBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] { return value }
    throw invalidAdmin("\(name) must be a boolean")
}

private func optionalAdminBool(_ value: JSONValue?, _ name: String) throws -> Bool? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .bool(bool):
        return bool
    default:
        throw invalidAdmin("\(name) must be a boolean or null")
    }
}

private func requiredAdminObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    guard case let .object(value) = object[name] else { throw invalidAdmin("\(name) must be an object") }
    return value
}

private func requiredAdminArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    guard case let .array(value) = object[name] else { throw invalidAdmin("\(name) must be a list") }
    return value
}

private func requiredAdminStringArray(_ object: [String: JSONValue], _ name: String) throws -> [String] {
    try requiredAdminArray(object, name).map { value in
        guard case let .string(string) = value else { throw invalidAdmin("\(name) entries must be strings") }
        return string
    }
}

private func adminIdentifier(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredAdminString(value, field)
    guard !cleaned.contains("/"), !cleaned.contains("\\"), cleaned.rangeOfCharacter(from: .whitespacesAndNewlines) == nil else {
        throw invalidAdmin("\(field) must be an opaque daemon identifier")
    }
    return cleaned
}

private func adminAgentName(_ value: String) throws -> String {
    let name = try adminIdentifier(value, "name")
    guard name != "device", !name.hasPrefix("device.") else {
        throw invalidAdmin("device system agents are not managed by hosted agent lifecycle")
    }
    return name
}

private func adminAgentURA(_ value: String) throws -> String {
    let cleaned = try requiredAdminString(value, "agent_ura")
    guard cleaned.contains("/agent/") else { throw invalidAdmin("agent_ura must be an Agent URA") }
    guard !cleaned.contains("/agent/device.") else {
        throw invalidAdmin("device-sponsored System Agents are not managed by hosted agent lifecycle")
    }
    return cleaned
}

private func adminHubURA(_ value: String) throws -> String {
    let cleaned = try requiredAdminString(value, "hub_ura")
    guard cleaned.contains("/hub/") else { throw invalidAdmin("hub_ura must be a Hub URA") }
    return cleaned
}

private func adminDeviceURA(_ value: String) throws -> String {
    let cleaned = try requiredAdminString(value, "device_ura")
    guard cleaned.contains("/device/") else { throw invalidAdmin("device_ura must be a Device URA") }
    return cleaned
}

private func adminReason(_ value: String) throws -> String {
    let cleaned = try requiredAdminString(value, "reason")
    guard cleaned.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x7f }) else {
        throw invalidAdmin("reason must not contain control characters")
    }
    return cleaned
}

private func invalidAdmin(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: adminGatewayProfile, message: message, details: ["profile": adminGatewayProfile])
}

private func adminUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": adminGatewayProfile])
}

private func adminTransport(_ message: String) -> SDKError {
    SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: message, details: ["profile": adminGatewayProfile])
}
