import Foundation

public let surfaceProfile = "surface"
public let defaultSurfacePageSize = 50
public let maxSurfacePageSize = 500

public struct SurfaceCarrierBase: Sendable, Equatable {
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
        self.callerURA = try cleanSurfaceString(callerURA, "caller_ura")
        self.calleeURA = try cleanSurfaceString(calleeURA, "callee_ura")
        self.subjectURA = try cleanSurfaceString(subjectURA, "subject_ura")
        self.descriptorVersion = try cleanSurfaceString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try cleanSurfaceString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidSurface("causal_context is required")
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

public struct SurfaceListPagesRequest: Sendable, Equatable {
    public let base: SurfaceCarrierBase
    public let limit: Int?
    public let cursor: String

    public init(base: SurfaceCarrierBase, limit: Int? = nil, cursor: String = "") throws {
        if let limit, (limit < 1 || limit > maxSurfacePageSize) {
            throw invalidSurface("surface page limit exceeds bounds")
        }
        self.base = base
        self.limit = limit
        self.cursor = try optionalSurfaceString(cursor, "cursor")
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        if let limit { object["limit"] = .number(Double(limit)) }
        if !cursor.isEmpty { object["cursor"] = .string(cursor) }
        return try encodeJSONObject(object)
    }
}

public struct SurfaceCreatePageRequest: Sendable, Equatable {
    public let base: SurfaceCarrierBase
    public let projectID: String
    public let folder: String
    public let visibility: String

    public init(base: SurfaceCarrierBase, projectID: String, folder: String, visibility: String = "public") throws {
        self.base = base
        self.projectID = try cleanSurfaceProjectID(projectID)
        self.folder = try cleanSurfaceString(folder, "folder")
        guard self.folder.hasPrefix("/") else {
            throw invalidSurface("surface folder must be absolute")
        }
        let resolvedVisibility = visibility.isEmpty ? "public" : visibility
        guard resolvedVisibility == "public" || resolvedVisibility == "private" else {
            throw invalidSurface("invalid surface visibility")
        }
        self.visibility = resolvedVisibility
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        object["project_id"] = .string(projectID)
        object["folder"] = .string(folder)
        object["visibility"] = .string(visibility)
        return try encodeJSONObject(object)
    }
}

public struct SurfaceDeletePageRequest: Sendable, Equatable {
    public let base: SurfaceCarrierBase
    public let projectID: String

    public init(base: SurfaceCarrierBase, projectID: String) throws {
        self.base = base
        self.projectID = try cleanSurfaceProjectID(projectID)
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        object["project_id"] = .string(projectID)
        return try encodeJSONObject(object)
    }
}

public struct SurfaceManifestRequest: Sendable, Equatable {
    public let base: SurfaceCarrierBase
    public let projectID: String

    public init(base: SurfaceCarrierBase, projectID: String) throws {
        self.base = base
        self.projectID = try cleanSurfaceProjectID(projectID)
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        object["project_id"] = .string(projectID)
        return try encodeJSONObject(object)
    }
}

public struct SurfaceHealthRequest: Sendable, Equatable {
    public let base: SurfaceCarrierBase
    public let projectID: String
    public let surfaceRef: String

    public init(base: SurfaceCarrierBase, projectID: String = "", surfaceRef: String = "") throws {
        self.base = base
        let cleanedProjectID = try optionalSurfaceString(projectID, "project_id")
        let cleanedSurfaceRef = try optionalSurfaceString(surfaceRef, "surface_ref")
        if cleanedProjectID.isEmpty && cleanedSurfaceRef.isEmpty {
            throw invalidSurface("project_id or surface_ref is required")
        }
        self.projectID = cleanedProjectID.isEmpty ? "" : try cleanSurfaceProjectID(cleanedProjectID)
        self.surfaceRef = cleanedSurfaceRef.isEmpty ? "" : try cleanSurfaceRef(cleanedSurfaceRef)
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        if !projectID.isEmpty { object["project_id"] = .string(projectID) }
        if !surfaceRef.isEmpty { object["surface_ref"] = .string(surfaceRef) }
        return try encodeJSONObject(object)
    }
}

public struct SurfacePageRecord: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let pageID: String
    public let ownerURA: String
    public let surfaceRef: String
    public let publicRef: String?
    public let status: String?
    public let metadata: [String: JSONValue]

    public init(
        profile: String,
        kind: String,
        pageID: String,
        ownerURA: String,
        surfaceRef: String,
        publicRef: String?,
        status: String?,
        metadata: [String: JSONValue]
    ) throws {
        guard profile == surfaceProfile, kind == "page_record" else {
            throw invalidSurface("invalid surface page record projection")
        }
        self.profile = profile
        self.kind = kind
        self.pageID = try cleanSurfaceString(pageID, "page_id")
        self.ownerURA = try cleanSurfaceString(ownerURA, "owner_ura")
        self.surfaceRef = try cleanSurfaceRef(surfaceRef)
        self.publicRef = try optionalSurfaceJSONString(publicRef.map(JSONValue.string), "public_ref")
        self.status = try optionalSurfaceJSONString(status.map(JSONValue.string), "status")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> SurfacePageRecord {
        try fromObject(decodeSurfaceObject(raw, label: "surface page record JSON"))
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> SurfacePageRecord {
        try SurfacePageRecord(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            pageID: requiredSurfaceString(object, "page_id"),
            ownerURA: requiredSurfaceString(object, "owner_ura"),
            surfaceRef: requiredSurfaceString(object, "surface_ref"),
            publicRef: optionalSurfaceJSONString(object["public_ref"], "public_ref"),
            status: optionalSurfaceJSONString(object["status"], "status"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
    }

    func jsonObject() -> [String: JSONValue] {
        [
            "profile": .string(profile),
            "kind": .string(kind),
            "page_id": .string(pageID),
            "owner_ura": .string(ownerURA),
            "surface_ref": .string(surfaceRef),
            "public_ref": publicRef.map(JSONValue.string) ?? .null,
            "status": status.map(JSONValue.string) ?? .null,
            "metadata": .object(metadata),
        ]
    }
}

public struct SurfacePagePage: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let itemKind: String
    public let items: [SurfacePageRecord]
    public let nextCursor: String?
    public let limit: Int
    public let source: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> SurfacePagePage {
        let object = try decodeSurfaceObject(raw, label: "surface page page JSON")
        let records = try requiredSurfaceArray(object, "items").map { item -> SurfacePageRecord in
            guard case let .object(itemObject) = item else {
                throw invalidSurface("items entry must be an object")
            }
            return try SurfacePageRecord.fromObject(itemObject)
        }
        let page = try SurfacePagePage(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            itemKind: requiredSurfaceString(object, "item_kind"),
            items: records,
            nextCursor: optionalSurfaceJSONString(object["next_cursor"], "next_cursor"),
            limit: requiredSurfaceInt(object, "limit"),
            source: requiredSurfaceString(object, "source"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard page.profile == surfaceProfile,
              page.kind == "surface_page_page",
              page.itemKind == "page_record",
              page.source == "pages_read_model",
              page.limit >= 1,
              page.limit <= maxSurfacePageSize
        else {
            throw invalidSurface("invalid surface page projection")
        }
        return page
    }
}

public struct SurfaceManifest: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let pageID: String
    public let ownerURA: String
    public let surfaceRef: String
    public let publicRef: String
    public let page: SurfacePageRecord
    public let entrypoint: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> SurfaceManifest {
        let object = try decodeSurfaceObject(raw, label: "surface manifest JSON")
        let manifest = try SurfaceManifest(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            pageID: requiredSurfaceString(object, "page_id"),
            ownerURA: requiredSurfaceString(object, "owner_ura"),
            surfaceRef: requiredSurfaceString(object, "surface_ref"),
            publicRef: requiredSurfaceString(object, "public_ref"),
            page: SurfacePageRecord.fromObject(requiredSurfaceObject(object, "page")),
            entrypoint: requiredSurfaceObject(object, "entrypoint"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard manifest.profile == surfaceProfile,
              manifest.kind == "surface_manifest",
              manifest.page.pageID == manifest.pageID,
              manifest.page.surfaceRef == manifest.surfaceRef,
              !manifest.entrypoint.isEmpty
        else {
            throw invalidSurface("invalid surface manifest projection")
        }
        return manifest
    }
}

public struct SurfacePublicPageRef: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let pageID: String
    public let ownerURA: String
    public let surfaceRef: String
    public let publicRef: String
    public let routeKind: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> SurfacePublicPageRef {
        let object = try decodeSurfaceObject(raw, label: "surface public page ref JSON")
        let ref = try SurfacePublicPageRef(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            pageID: requiredSurfaceString(object, "page_id"),
            ownerURA: requiredSurfaceString(object, "owner_ura"),
            surfaceRef: requiredSurfaceString(object, "surface_ref"),
            publicRef: requiredSurfaceString(object, "public_ref"),
            routeKind: requiredSurfaceString(object, "route_kind"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard ref.profile == surfaceProfile,
              ref.kind == "public_page_ref",
              ref.routeKind == "hub_web"
        else {
            throw invalidSurface("invalid surface public page ref projection")
        }
        return ref
    }
}

public struct SurfaceMutationResult: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let operation: String
    public let pageID: String
    public let removed: Bool
    public let state: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> SurfaceMutationResult {
        let object = try decodeSurfaceObject(raw, label: "surface mutation result JSON")
        let result = try SurfaceMutationResult(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            operation: requiredSurfaceString(object, "operation"),
            pageID: requiredSurfaceString(object, "page_id"),
            removed: requiredSurfaceBool(object, "removed"),
            state: requiredSurfaceString(object, "state"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard result.profile == surfaceProfile,
              result.kind == "surface_mutation_result",
              result.operation == "delete",
              ["deleted", "unknown"].contains(result.state)
        else {
            throw invalidSurface("invalid surface mutation result projection")
        }
        _ = try cleanSurfaceProjectID(result.pageID)
        return result
    }
}

public struct SurfaceHealthCheck: Sendable, Equatable {
    public let name: String
    public let state: String
    public let ready: Bool
    public let message: String?
    public let latencyMS: Int
    public let metadata: [String: JSONValue]

    static func fromObject(_ object: [String: JSONValue]) throws -> SurfaceHealthCheck {
        let check = try SurfaceHealthCheck(
            name: requiredSurfaceString(object, "name"),
            state: requiredSurfaceString(object, "state"),
            ready: requiredSurfaceBool(object, "ready"),
            message: optionalSurfaceJSONString(object["message"], "message"),
            latencyMS: requiredSurfaceInt(object, "latency_ms"),
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard check.latencyMS >= 0 else {
            throw invalidSurface("latency_ms must be non-negative")
        }
        return check
    }
}

public struct SurfaceHealth: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let state: String
    public let ready: Bool
    public let ownerURA: String
    public let surfaceRef: String
    public let descriptorRef: String
    public let descriptorVersion: String
    public let pageCount: Int
    public let checks: [SurfaceHealthCheck]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> SurfaceHealth {
        let object = try decodeSurfaceObject(raw, label: "surface health JSON")
        let checks = try requiredSurfaceArray(object, "checks").map { item -> SurfaceHealthCheck in
            guard case let .object(checkObject) = item else {
                throw invalidSurface("checks entry must be an object")
            }
            return try SurfaceHealthCheck.fromObject(checkObject)
        }
        let health = try SurfaceHealth(
            profile: requiredSurfaceString(object, "profile"),
            kind: requiredSurfaceString(object, "kind"),
            state: requiredSurfaceString(object, "state"),
            ready: requiredSurfaceBool(object, "ready"),
            ownerURA: requiredSurfaceString(object, "owner_ura"),
            surfaceRef: requiredSurfaceString(object, "surface_ref"),
            descriptorRef: requiredSurfaceString(object, "descriptor_ref"),
            descriptorVersion: requiredSurfaceString(object, "descriptor_version"),
            pageCount: requiredSurfaceInt(object, "page_count"),
            checks: checks,
            metadata: requiredSurfaceObject(object, "metadata")
        )
        guard health.profile == surfaceProfile,
              health.kind == "surface_health",
              health.pageCount >= 0
        else {
            throw invalidSurface("invalid surface health projection")
        }
        _ = try cleanSurfaceRef(health.surfaceRef)
        return health
    }
}

public protocol SurfaceTransport: AnyObject, Sendable {
    func buildListPagesInvocation(_ requestJSON: Data) async throws -> Data
    func buildCreatePageInvocation(_ requestJSON: Data) async throws -> Data
    func buildDeletePageInvocation(_ requestJSON: Data) async throws -> Data
    func buildManifestInvocation(_ requestJSON: Data) async throws -> Data
    func buildHealthInvocation(_ requestJSON: Data) async throws -> Data
    func listPages(_ requestJSON: Data) async throws -> Data
    func createPage(_ requestJSON: Data) async throws -> Data
    func deletePage(_ requestJSON: Data) async throws -> Data
    func surfaceManifest(_ requestJSON: Data) async throws -> Data
    func publicPageRef(_ pageJSON: Data) async throws -> Data
    func surfaceHealth(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension SurfaceTransport {
    func buildListPagesInvocation(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface list-pages invocation transport is not available") }
    func buildCreatePageInvocation(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface create-page invocation transport is not available") }
    func buildDeletePageInvocation(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface delete-page invocation transport is not available") }
    func buildManifestInvocation(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface manifest invocation transport is not available") }
    func buildHealthInvocation(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface health invocation transport is not available") }
    func listPages(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface list pages transport is not available") }
    func createPage(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface create page transport is not available") }
    func deletePage(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface delete page transport is not available") }
    func surfaceManifest(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface manifest transport is not available") }
    func publicPageRef(_ pageJSON: Data) async throws -> Data { throw surfaceUnsupported("surface public page ref transport is not available") }
    func surfaceHealth(_ requestJSON: Data) async throws -> Data { throw surfaceUnsupported("surface health transport is not available") }
    func close() async throws {}
}

public final class SurfaceClient: @unchecked Sendable {
    private let transport: SurfaceTransport
    private var closed = false

    public init(transport: SurfaceTransport) {
        self.transport = transport
    }

    public func buildListPagesInvocation(_ request: SurfaceListPagesRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildListPagesInvocation(request.jsonData()) }
    }

    public func buildCreatePageInvocation(_ request: SurfaceCreatePageRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildCreatePageInvocation(request.jsonData()) }
    }

    public func buildDeletePageInvocation(_ request: SurfaceDeletePageRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildDeletePageInvocation(request.jsonData()) }
    }

    public func buildManifestInvocation(_ request: SurfaceManifestRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildManifestInvocation(request.jsonData()) }
    }

    public func buildHealthInvocation(_ request: SurfaceHealthRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildHealthInvocation(request.jsonData()) }
    }

    public func listPages(_ request: SurfaceListPagesRequest) async throws -> SurfacePagePage {
        try await SurfacePagePage.fromJSON(raw { try await transport.listPages(request.jsonData()) })
    }

    public func createPage(_ request: SurfaceCreatePageRequest) async throws -> SurfacePageRecord {
        try await SurfacePageRecord.fromJSON(raw { try await transport.createPage(request.jsonData()) })
    }

    public func deletePage(_ request: SurfaceDeletePageRequest) async throws -> SurfaceMutationResult {
        try await SurfaceMutationResult.fromJSON(raw { try await transport.deletePage(request.jsonData()) })
    }

    public func surfaceManifest(_ request: SurfaceManifestRequest) async throws -> SurfaceManifest {
        try await SurfaceManifest.fromJSON(raw { try await transport.surfaceManifest(request.jsonData()) })
    }

    public func publicPageRef(_ page: SurfacePageRecord) async throws -> SurfacePublicPageRef {
        try await SurfacePublicPageRef.fromJSON(raw { try await transport.publicPageRef(encodeJSONObject(page.jsonObject())) })
    }

    public func surfaceHealth(_ request: SurfaceHealthRequest) async throws -> SurfaceHealth {
        try await SurfaceHealth.fromJSON(raw { try await transport.surfaceHealth(request.jsonData()) })
    }

    public func surfaceStatus(_ request: SurfaceHealthRequest) async throws -> SurfaceHealth {
        try await surfaceHealth(request)
    }

    public func projectPagePage(_ raw: Data) throws -> SurfacePagePage {
        try SurfacePagePage.fromJSON(raw)
    }

    public func projectManifest(_ raw: Data) throws -> SurfaceManifest {
        try SurfaceManifest.fromJSON(raw)
    }

    public func projectHealth(_ raw: Data) throws -> SurfaceHealth {
        try SurfaceHealth.fromJSON(raw)
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodeSurfaceObject(try await raw(call), label: "surface invocation JSON")
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
                message: "surface transport failed",
                details: ["profile": surfaceProfile]
            )
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("surface")
        }
    }
}

private func decodeSurfaceObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidSurface("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)")
    }
}

private func requiredSurfaceString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty {
        return value
    }
    throw invalidSurface("\(name) must be a non-empty string")
}

private func requiredSurfaceBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidSurface("\(name) must be a boolean")
}

private func requiredSurfaceInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if case let .number(value) = object[name],
       value >= 0,
       value.rounded() == value,
       value <= Double(Int.max)
    {
        return Int(value)
    }
    throw invalidSurface("\(name) must be a non-negative integer")
}

private func requiredSurfaceArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    if case let .array(value) = object[name] {
        return value
    }
    throw invalidSurface("\(name) must be a list")
}

private func requiredSurfaceObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] {
        return value
    }
    throw invalidSurface("\(name) must be an object")
}

private func optionalSurfaceJSONString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidSurface("\(name) must be a string or null")
    }
}

private func cleanSurfaceString(_ value: String, _ field: String) throws -> String {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
          value == value.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        throw invalidSurface("\(field) is required")
    }
    return value
}

private func optionalSurfaceString(_ value: String, _ field: String) throws -> String {
    guard value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidSurface("\(field) must not contain surrounding whitespace")
    }
    return value
}

private func cleanSurfaceProjectID(_ value: String) throws -> String {
    let cleaned = try cleanSurfaceString(value, "project_id")
    guard cleaned.range(of: #"^[A-Za-z0-9_-]{1,64}$"#, options: .regularExpression) != nil else {
        throw invalidSurface("invalid surface project_id")
    }
    return cleaned
}

private func cleanSurfaceRef(_ value: String) throws -> String {
    let cleaned = try cleanSurfaceString(value, "surface_ref")
    guard !cleaned.hasPrefix("http://"), !cleaned.hasPrefix("https://") else {
        throw invalidSurface("surface_ref must not be an HTTP route")
    }
    return cleaned
}

private func invalidSurface(_ message: String) -> SDKError {
    SDKError(
        code: .invalidArgument,
        stage: surfaceProfile,
        message: message,
        details: ["profile": surfaceProfile]
    )
}

private func surfaceUnsupported(_ message: String) -> SDKError {
    SDKError(
        code: .notImplemented,
        stage: "transport",
        message: message,
        details: ["profile": surfaceProfile]
    )
}
