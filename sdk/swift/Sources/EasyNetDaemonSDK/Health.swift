import Foundation

public indirect enum JSONValue: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])
}

public struct RuntimeHealth: Equatable, Sendable {
    public let apiReady: Bool
    public let daemonReady: Bool
    public let invocationReady: Bool
    public let directoryReady: Bool
    public let trustReady: Bool
    public let runtimeReady: Bool
    public let version: String?
    public let abiVersion: Int?
    public let mismatch: [String: JSONValue]?
    public let diagnostics: [String]

    public init(
        apiReady: Bool,
        daemonReady: Bool,
        invocationReady: Bool,
        directoryReady: Bool,
        trustReady: Bool,
        runtimeReady: Bool,
        version: String? = nil,
        abiVersion: Int? = nil,
        mismatch: [String: JSONValue]? = nil,
        diagnostics: [String] = []
    ) {
        self.apiReady = apiReady
        self.daemonReady = daemonReady
        self.invocationReady = invocationReady
        self.directoryReady = directoryReady
        self.trustReady = trustReady
        self.runtimeReady = runtimeReady
        self.version = version
        self.abiVersion = abiVersion
        self.mismatch = mismatch
        self.diagnostics = diagnostics
    }

    public static func fromJSON(_ raw: Data) throws -> RuntimeHealth {
        let object = try decodeObject(raw, label: "runtime health JSON")
        return RuntimeHealth(
            apiReady: try requiredBool(object, "api_ready"),
            daemonReady: try requiredBool(object, "daemon_ready"),
            invocationReady: try requiredBool(object, "invocation_ready"),
            directoryReady: try requiredBool(object, "directory_ready"),
            trustReady: try requiredBool(object, "trust_ready"),
            runtimeReady: try requiredBool(object, "runtime_ready"),
            version: try optionalString(object["version"], "version"),
            abiVersion: try optionalInt(object["abi_version"], "abi_version"),
            mismatch: try optionalObject(object["mismatch"], "mismatch"),
            diagnostics: try diagnosticsList(object["diagnostics"])
        )
    }

    public static func fromJSON(_ raw: String) throws -> RuntimeHealth {
        try fromJSON(Data(raw.utf8))
    }

    public var apiAlive: Bool {
        apiReady && daemonReady
    }

    public var ready: Bool {
        runtimeReady
    }
}

public struct DiagnosticCheck: Equatable, Sendable {
    public let name: String
    public let ready: Bool
    public let message: String?

    public init(name: String, ready: Bool, message: String? = nil) throws {
        guard !name.isEmpty else {
            throw invalidHealthField("checks", "name must be a non-empty string")
        }
        self.name = name
        self.ready = ready
        self.message = message
    }
}

public struct DiagnosticsReport: Equatable, Sendable {
    public let profile: String
    public let kind: String
    public let state: String
    public let ready: Bool
    public let version: String
    public let abiVersion: Int
    public let controlEndpoint: String
    public let invocationEndpoint: String?
    public let checks: [DiagnosticCheck]
    public let diagnostics: [String]

    public init(
        profile: String,
        kind: String,
        state: String,
        ready: Bool,
        version: String,
        abiVersion: Int,
        controlEndpoint: String,
        invocationEndpoint: String?,
        checks: [DiagnosticCheck],
        diagnostics: [String] = []
    ) throws {
        guard profile == "health" else {
            throw invalidHealthField("profile", "must be health")
        }
        guard kind == "diagnostics_report" else {
            throw invalidHealthField("kind", "must be diagnostics_report")
        }
        guard !checks.isEmpty else {
            throw invalidHealthField("checks", "must be non-empty")
        }
        self.profile = profile
        self.kind = kind
        self.state = state
        self.ready = ready
        self.version = version
        self.abiVersion = abiVersion
        self.controlEndpoint = controlEndpoint
        self.invocationEndpoint = invocationEndpoint
        self.checks = checks
        self.diagnostics = diagnostics
    }

    public static func fromJSON(_ raw: Data) throws -> DiagnosticsReport {
        let object = try decodeObject(raw, label: "diagnostics JSON")
        return try DiagnosticsReport(
            profile: requiredString(object, "profile"),
            kind: requiredString(object, "kind"),
            state: requiredString(object, "state"),
            ready: requiredBool(object, "ready"),
            version: requiredString(object, "version"),
            abiVersion: requiredInt(object, "abi_version"),
            controlEndpoint: requiredString(object, "control_endpoint"),
            invocationEndpoint: optionalString(object["invocation_endpoint"], "invocation_endpoint"),
            checks: diagnosticChecks(object["checks"]),
            diagnostics: diagnosticsList(object["diagnostics"])
        )
    }

    public static func fromJSON(_ raw: String) throws -> DiagnosticsReport {
        try fromJSON(Data(raw.utf8))
    }
}

public protocol HealthTransport: AnyObject, Sendable {
    func runtimeHealth() async throws -> Data
    func close() async throws
}

public extension HealthTransport {
    func close() async throws {}
}

public protocol DiagnosticsTransport: AnyObject, Sendable {
    func runtimeDiagnostics() async throws -> Data
}

public final class HealthClient: @unchecked Sendable {
    private let transport: HealthTransport
    private var closed = false

    public init(transport: HealthTransport) {
        self.transport = transport
    }

    public func runtimeHealth() async throws -> RuntimeHealth {
        try requireOpen()
        do {
            return try RuntimeHealth.fromJSON(try await transport.runtimeHealth())
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "runtime health transport failed"
            )
        }
    }

    public func diagnostics() async throws -> DiagnosticsReport {
        try requireOpen()
        guard let diagnosticsTransport = transport as? DiagnosticsTransport else {
            throw SDKError(
                code: .notImplemented,
                stage: "transport",
                message: "health diagnostics transport is not available"
            )
        }
        do {
            return try DiagnosticsReport.fromJSON(try await diagnosticsTransport.runtimeDiagnostics())
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "runtime diagnostics transport failed"
            )
        }
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await transport.close()
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("health")
        }
    }
}

func decodeObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidHealthField(label, "must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(
            code: .invalidArgument,
            stage: "decode",
            message: "decode \(label): \(error)"
        )
    }
}

func jsonValue(_ value: Any) throws -> JSONValue {
    if value is NSNull {
        return .null
    }
    if let bool = value as? Bool {
        return .bool(bool)
    }
    if let number = value as? NSNumber {
        return .number(number.doubleValue)
    }
    if let string = value as? String {
        return .string(string)
    }
    if let array = value as? [Any] {
        return .array(try array.map(jsonValue))
    }
    if let object = value as? [String: Any] {
        return .object(try object.mapValues(jsonValue))
    }
    throw invalidHealthField("json", "contains unsupported value")
}

extension JSONValue {
    var objectValue: [String: JSONValue]? {
        if case let .object(value) = self {
            return value
        }
        return nil
    }
}

private func requiredBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidHealthField(name, "must be a boolean")
}

private func requiredString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty {
        return value
    }
    throw invalidHealthField(name, "must be a non-empty string")
}

private func requiredInt(_ object: [String: JSONValue], _ name: String) throws -> Int {
    if let value = try optionalInt(object[name], name) {
        return value
    }
    throw invalidHealthField(name, "must be a non-negative integer")
}

private func optionalString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidHealthField(name, "must be a string or null")
    }
}

private func optionalInt(_ value: JSONValue?, _ name: String) throws -> Int? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .number(number):
        guard number >= 0, number.rounded() == number, number <= Double(Int.max) else {
            throw invalidHealthField(name, "must be a non-negative integer or null")
        }
        return Int(number)
    default:
        throw invalidHealthField(name, "must be a non-negative integer or null")
    }
}

private func optionalObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidHealthField(name, "must be an object or null")
    }
}

private func diagnosticsList(_ value: JSONValue?) throws -> [String] {
    guard let value else {
        return []
    }
    guard case let .array(items) = value else {
        throw invalidHealthField("diagnostics", "must be an array")
    }
    return try items.map { item in
        guard case let .string(diagnostic) = item else {
            throw invalidHealthField("diagnostics", "items must be strings")
        }
        return diagnostic
    }
}

private func diagnosticChecks(_ value: JSONValue?) throws -> [DiagnosticCheck] {
    guard case let .array(items) = value, !items.isEmpty else {
        throw invalidHealthField("checks", "must be non-empty")
    }
    return try items.map { item in
        guard case let .object(check) = item else {
            throw invalidHealthField("checks", "items must be objects")
        }
        return try DiagnosticCheck(
            name: requiredString(check, "name"),
            ready: requiredBool(check, "ready"),
            message: optionalString(check["message"], "message")
        )
    }
}

private func invalidHealthField(_ name: String, _ message: String) -> SDKError {
    SDKError(
        code: .invalidArgument,
        stage: "decode",
        message: "runtime health field \(name) \(message)",
        details: ["field": name]
    )
}
