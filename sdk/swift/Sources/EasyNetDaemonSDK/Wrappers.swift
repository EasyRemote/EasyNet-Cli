import Foundation

public let wrappersProfile = "wrappers"

public struct WrapperFileRecord: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let fileRef: String
    public let ownerURA: String
    public let contentType: String
    public let sizeBytes: Int64?
    public let contentHash: String?
    public let metadata: [String: JSONValue]

    public init(
        profile: String,
        kind: String,
        fileRef: String,
        ownerURA: String,
        contentType: String,
        sizeBytes: Int64?,
        contentHash: String?,
        metadata: [String: JSONValue]
    ) throws {
        guard profile == wrappersProfile, kind == "file_record" else {
            throw invalidWrapper("invalid file_record projection")
        }
        self.profile = profile
        self.kind = kind
        self.fileRef = try requiredWrapperURA(fileRef, "file_ref")
        self.ownerURA = try requiredWrapperURA(ownerURA, "owner_ura")
        self.contentType = try requiredWrapperString(contentType, "content_type")
        if let sizeBytes, sizeBytes < 0 {
            throw invalidWrapper("size_bytes must be a non-negative integer or null")
        }
        self.sizeBytes = sizeBytes
        self.contentHash = try optionalWrapperString(contentHash.map(JSONValue.string), "content_hash")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> WrapperFileRecord {
        let object = try decodeWrapperObject(raw, label: "wrapper file record JSON")
        try validateWrapperKind(object, "file_record")
        return try WrapperFileRecord(
            profile: requiredWrapperString(object, "profile"),
            kind: requiredWrapperString(object, "kind"),
            fileRef: requiredWrapperString(object, "file_ref"),
            ownerURA: requiredWrapperString(object, "owner_ura"),
            contentType: requiredWrapperString(object, "content_type"),
            sizeBytes: optionalWrapperInteger(object["size_bytes"], "size_bytes"),
            contentHash: optionalWrapperString(object["content_hash"], "content_hash"),
            metadata: requiredWrapperObject(object, "metadata")
        )
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "file_ref": .string(fileRef),
            "owner_ura": .string(ownerURA),
            "content_type": .string(contentType),
            "size_bytes": sizeBytes.map { .number(Double($0)) } ?? .null,
            "content_hash": contentHash.map(JSONValue.string) ?? .null,
            "metadata": .object(metadata),
        ])
    }
}

public struct WrapperTerminalSession: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let sessionID: String
    public let ownerURA: String
    public let state: String
    public let terminalRef: String?
    public let metadata: [String: JSONValue]

    public init(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, terminalRef: String?, metadata: [String: JSONValue]) throws {
        guard profile == wrappersProfile, kind == "terminal_session" else {
            throw invalidWrapper("invalid terminal_session projection")
        }
        self.profile = profile
        self.kind = kind
        self.sessionID = try requiredWrapperString(sessionID, "session_id")
        self.ownerURA = try requiredWrapperURA(ownerURA, "owner_ura")
        self.state = try requiredWrapperString(state, "state")
        self.terminalRef = try optionalWrapperString(terminalRef.map(JSONValue.string), "terminal_ref")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> WrapperTerminalSession {
        let object = try decodeWrapperObject(raw, label: "wrapper terminal session JSON")
        try validateWrapperKind(object, "terminal_session")
        return try WrapperTerminalSession(
            profile: requiredWrapperString(object, "profile"),
            kind: requiredWrapperString(object, "kind"),
            sessionID: requiredWrapperString(object, "session_id"),
            ownerURA: requiredWrapperString(object, "owner_ura"),
            state: requiredWrapperString(object, "state"),
            terminalRef: optionalWrapperString(object["terminal_ref"], "terminal_ref"),
            metadata: requiredWrapperObject(object, "metadata")
        )
    }

    func jsonData() throws -> Data {
        try encodeWrapperSession(profile: profile, kind: kind, sessionID: sessionID, ownerURA: ownerURA, state: state, refField: "terminal_ref", refValue: terminalRef, metadata: metadata)
    }
}

public struct WrapperRemoteDesktopSession: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let sessionID: String
    public let ownerURA: String
    public let state: String
    public let displayRef: String?
    public let metadata: [String: JSONValue]

    public init(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, displayRef: String?, metadata: [String: JSONValue]) throws {
        guard profile == wrappersProfile, kind == "remote_desktop_session" else {
            throw invalidWrapper("invalid remote_desktop_session projection")
        }
        self.profile = profile
        self.kind = kind
        self.sessionID = try requiredWrapperString(sessionID, "session_id")
        self.ownerURA = try requiredWrapperURA(ownerURA, "owner_ura")
        self.state = try requiredWrapperString(state, "state")
        self.displayRef = try optionalWrapperString(displayRef.map(JSONValue.string), "display_ref")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> WrapperRemoteDesktopSession {
        let object = try decodeWrapperObject(raw, label: "wrapper remote desktop session JSON")
        try validateWrapperKind(object, "remote_desktop_session")
        return try WrapperRemoteDesktopSession(
            profile: requiredWrapperString(object, "profile"),
            kind: requiredWrapperString(object, "kind"),
            sessionID: requiredWrapperString(object, "session_id"),
            ownerURA: requiredWrapperString(object, "owner_ura"),
            state: requiredWrapperString(object, "state"),
            displayRef: optionalWrapperString(object["display_ref"], "display_ref"),
            metadata: requiredWrapperObject(object, "metadata")
        )
    }

    func jsonData() throws -> Data {
        try encodeWrapperSession(profile: profile, kind: kind, sessionID: sessionID, ownerURA: ownerURA, state: state, refField: "display_ref", refValue: displayRef, metadata: metadata)
    }
}

public struct WrapperBrowserSession: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let sessionID: String
    public let ownerURA: String
    public let state: String
    public let browserRef: String?
    public let metadata: [String: JSONValue]

    public init(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, browserRef: String?, metadata: [String: JSONValue]) throws {
        guard profile == wrappersProfile, kind == "browser_session" else {
            throw invalidWrapper("invalid browser_session projection")
        }
        self.profile = profile
        self.kind = kind
        self.sessionID = try requiredWrapperString(sessionID, "session_id")
        self.ownerURA = try requiredWrapperURA(ownerURA, "owner_ura")
        self.state = try requiredWrapperString(state, "state")
        self.browserRef = try optionalWrapperString(browserRef.map(JSONValue.string), "browser_ref")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> WrapperBrowserSession {
        let object = try decodeWrapperObject(raw, label: "wrapper browser session JSON")
        try validateWrapperKind(object, "browser_session")
        return try WrapperBrowserSession(
            profile: requiredWrapperString(object, "profile"),
            kind: requiredWrapperString(object, "kind"),
            sessionID: requiredWrapperString(object, "session_id"),
            ownerURA: requiredWrapperString(object, "owner_ura"),
            state: requiredWrapperString(object, "state"),
            browserRef: optionalWrapperString(object["browser_ref"], "browser_ref"),
            metadata: requiredWrapperObject(object, "metadata")
        )
    }

    func jsonData() throws -> Data {
        try encodeWrapperSession(profile: profile, kind: kind, sessionID: sessionID, ownerURA: ownerURA, state: state, refField: "browser_ref", refValue: browserRef, metadata: metadata)
    }
}

public struct WrapperMediaSession: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let sessionID: String
    public let ownerURA: String
    public let state: String
    public let mediaKind: String
    public let streamRef: String?
    public let metadata: [String: JSONValue]

    public init(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, mediaKind: String, streamRef: String?, metadata: [String: JSONValue]) throws {
        guard profile == wrappersProfile, kind == "media_session" else {
            throw invalidWrapper("invalid media_session projection")
        }
        self.profile = profile
        self.kind = kind
        self.sessionID = try requiredWrapperString(sessionID, "session_id")
        self.ownerURA = try requiredWrapperURA(ownerURA, "owner_ura")
        self.state = try requiredWrapperString(state, "state")
        self.mediaKind = try requiredWrapperString(mediaKind, "media_kind")
        self.streamRef = try optionalWrapperString(streamRef.map(JSONValue.string), "stream_ref")
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> WrapperMediaSession {
        let object = try decodeWrapperObject(raw, label: "wrapper media session JSON")
        try validateWrapperKind(object, "media_session")
        return try WrapperMediaSession(
            profile: requiredWrapperString(object, "profile"),
            kind: requiredWrapperString(object, "kind"),
            sessionID: requiredWrapperString(object, "session_id"),
            ownerURA: requiredWrapperString(object, "owner_ura"),
            state: requiredWrapperString(object, "state"),
            mediaKind: requiredWrapperString(object, "media_kind"),
            streamRef: optionalWrapperString(object["stream_ref"], "stream_ref"),
            metadata: requiredWrapperObject(object, "metadata")
        )
    }

    func jsonData() throws -> Data {
        var object = try wrapperSessionObject(profile: profile, kind: kind, sessionID: sessionID, ownerURA: ownerURA, state: state, refField: "stream_ref", refValue: streamRef, metadata: metadata)
        object["media_kind"] = .string(mediaKind)
        return try encodeJSONObject(object)
    }
}

public protocol WrapperTransport: AnyObject, Sendable {
    func projectFileRecord(_ requestJSON: Data) async throws -> Data
    func projectTerminalSession(_ requestJSON: Data) async throws -> Data
    func projectRemoteDesktopSession(_ requestJSON: Data) async throws -> Data
    func projectBrowserSession(_ requestJSON: Data) async throws -> Data
    func projectMediaSession(_ requestJSON: Data) async throws -> Data
    func close() async throws
}

public extension WrapperTransport {
    func projectFileRecord(_ requestJSON: Data) async throws -> Data { throw wrapperUnsupported("wrapper file-record projection transport is not available") }
    func projectTerminalSession(_ requestJSON: Data) async throws -> Data { throw wrapperUnsupported("wrapper terminal-session projection transport is not available") }
    func projectRemoteDesktopSession(_ requestJSON: Data) async throws -> Data { throw wrapperUnsupported("wrapper remote-desktop-session projection transport is not available") }
    func projectBrowserSession(_ requestJSON: Data) async throws -> Data { throw wrapperUnsupported("wrapper browser-session projection transport is not available") }
    func projectMediaSession(_ requestJSON: Data) async throws -> Data { throw wrapperUnsupported("wrapper media-session projection transport is not available") }
    func close() async throws {}
}

public final class WrapperClient: @unchecked Sendable {
    private let transport: WrapperTransport
    private var closed = false

    public init(transport: WrapperTransport) {
        self.transport = transport
    }

    public func projectFileRecord(_ raw: Data) async throws -> WrapperFileRecord {
        try await WrapperFileRecord.fromJSON(project(raw) { try await transport.projectFileRecord($0) })
    }

    public func projectFileRecord(_ record: WrapperFileRecord) async throws -> WrapperFileRecord {
        try await projectFileRecord(record.jsonData())
    }

    public func projectTerminalSession(_ raw: Data) async throws -> WrapperTerminalSession {
        try await WrapperTerminalSession.fromJSON(project(raw) { try await transport.projectTerminalSession($0) })
    }

    public func projectTerminalSession(_ session: WrapperTerminalSession) async throws -> WrapperTerminalSession {
        try await projectTerminalSession(session.jsonData())
    }

    public func projectRemoteDesktopSession(_ raw: Data) async throws -> WrapperRemoteDesktopSession {
        try await WrapperRemoteDesktopSession.fromJSON(project(raw) { try await transport.projectRemoteDesktopSession($0) })
    }

    public func projectRemoteDesktopSession(_ session: WrapperRemoteDesktopSession) async throws -> WrapperRemoteDesktopSession {
        try await projectRemoteDesktopSession(session.jsonData())
    }

    public func projectBrowserSession(_ raw: Data) async throws -> WrapperBrowserSession {
        try await WrapperBrowserSession.fromJSON(project(raw) { try await transport.projectBrowserSession($0) })
    }

    public func projectBrowserSession(_ session: WrapperBrowserSession) async throws -> WrapperBrowserSession {
        try await projectBrowserSession(session.jsonData())
    }

    public func projectMediaSession(_ raw: Data) async throws -> WrapperMediaSession {
        try await WrapperMediaSession.fromJSON(project(raw) { try await transport.projectMediaSession($0) })
    }

    public func projectMediaSession(_ session: WrapperMediaSession) async throws -> WrapperMediaSession {
        try await projectMediaSession(session.jsonData())
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func project(_ raw: Data, _ call: (Data) async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call(raw)
        } catch let error as SDKError {
            throw error
        } catch {
            throw wrapperTransport("wrapper transport failed")
        }
    }

    private func requireOpen() throws {
        if closed { throw SDKError.closed(wrappersProfile) }
    }
}

private func encodeWrapperSession(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, refField: String, refValue: String?, metadata: [String: JSONValue]) throws -> Data {
    try encodeJSONObject(wrapperSessionObject(profile: profile, kind: kind, sessionID: sessionID, ownerURA: ownerURA, state: state, refField: refField, refValue: refValue, metadata: metadata))
}

private func wrapperSessionObject(profile: String, kind: String, sessionID: String, ownerURA: String, state: String, refField: String, refValue: String?, metadata: [String: JSONValue]) throws -> [String: JSONValue] {
    [
        "profile": .string(profile),
        "kind": .string(kind),
        "session_id": .string(sessionID),
        "owner_ura": .string(ownerURA),
        "state": .string(state),
        refField: refValue.map(JSONValue.string) ?? .null,
        "metadata": .object(metadata),
    ]
}

private func decodeWrapperObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else { throw invalidWrapper("\(label) must be an object") }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)", details: ["profile": wrappersProfile])
    }
}

private func validateWrapperKind(_ object: [String: JSONValue], _ kind: String) throws {
    guard try requiredWrapperString(object, "profile") == wrappersProfile,
          try requiredWrapperString(object, "kind") == kind
    else {
        throw invalidWrapper("invalid \(kind) projection")
    }
}

private func requiredWrapperString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidWrapper("\(field) is required")
    }
    return value
}

private func requiredWrapperString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) {
        return value
    }
    throw invalidWrapper("\(name) is required")
}

private func optionalWrapperString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        guard string == string.trimmingCharacters(in: .whitespacesAndNewlines) else {
            throw invalidWrapper("\(name) must be a string or null")
        }
        return string
    default:
        throw invalidWrapper("\(name) must be a string or null")
    }
}

private func requiredWrapperURA(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredWrapperString(value, field)
    guard cleaned.hasPrefix("easynet:///r/") else {
        throw invalidWrapper("\(field) must be a URA")
    }
    return cleaned
}

private func requiredWrapperObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    guard case let .object(value) = object[name] else { throw invalidWrapper("\(name) must be an object") }
    return value
}

private func optionalWrapperInteger(_ value: JSONValue?, _ name: String) throws -> Int64? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .number(number):
        let integer = Int64(number)
        guard number >= 0, Double(integer) == number else {
            throw invalidWrapper("\(name) must be a non-negative integer or null")
        }
        return integer
    default:
        throw invalidWrapper("\(name) must be a non-negative integer or null")
    }
}

private func invalidWrapper(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: wrappersProfile, message: message, details: ["profile": wrappersProfile])
}

private func wrapperUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": wrappersProfile])
}

private func wrapperTransport(_ message: String) -> SDKError {
    SDKError(code: .transport, stage: "transport", retryHint: .safe, retryable: true, message: message, details: ["profile": wrappersProfile])
}
