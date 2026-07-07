import Foundation

public enum CompanionDesiredState: String, Sendable {
    case enabled
    case disabled
}

public enum CompanionSupervisorState: String, Sendable {
    case unsupportedPlatform = "unsupported_platform"
    case unsupportedSession = "unsupported_session"
    case notInstalled = "not_installed"
    case installedDisabled = "installed_disabled"
    case installedEnabled = "installed_enabled"
    case installError = "install_error"
    case enableError = "enable_error"
    case disableError = "disable_error"
}

public enum CompanionObservedState: String, Sendable {
    case unknown
    case notRunning = "not_running"
    case starting
    case running
    case stale
    case exited
    case versionMismatch = "version_mismatch"
    case healthError = "health_error"
}

public enum CompanionProjectedState: String, Sendable {
    case disabled
    case unsupportedPlatform = "unsupported_platform"
    case unsupportedSession = "unsupported_session"
    case notInstalled = "not_installed"
    case installedDisabled = "installed_disabled"
    case readyStopped = "ready_stopped"
    case starting
    case running
    case stale
    case error
}

public enum CompanionBootPolicy: String, Sendable {
    case manual
    case ensureRunningAfterDaemonReady = "ensure_running_after_daemon_ready"
}

public enum CompanionStopPolicy: String, Sendable {
    case keepRunning = "keep_running"
    case stopOnRuntimeStop = "stop_on_runtime_stop"
    case stopOnPluginDisable = "stop_on_plugin_disable"
}

public enum CompanionHealthMode: String, Sendable {
    case processName = "process_name"
    case statusFile = "status_file"
    case localIPC = "local_ipc"
}

public struct DesktopCompanionStatus: Equatable, Sendable {
    public let packageID: String
    public let packageVersion: String
    public let displayName: String
    public let platform: String
    public let desiredState: CompanionDesiredState
    public let supervisorState: CompanionSupervisorState
    public let observedState: CompanionObservedState
    public let projectedState: CompanionProjectedState
    public let bootPolicy: CompanionBootPolicy
    public let stopPolicy: CompanionStopPolicy
    public let health: CompanionHealthMode
    public let pid: UInt64?
    public let version: String?
    public let lastSeenUnixMS: UInt64?
    public let launchMethod: String?
    public let error: [String: JSONValue]?
    public let metadata: [String: JSONValue]

    public init(
        packageID: String,
        packageVersion: String,
        displayName: String,
        platform: String,
        desiredState: CompanionDesiredState,
        supervisorState: CompanionSupervisorState,
        observedState: CompanionObservedState,
        projectedState: CompanionProjectedState,
        bootPolicy: CompanionBootPolicy,
        stopPolicy: CompanionStopPolicy,
        health: CompanionHealthMode,
        pid: UInt64? = nil,
        version: String? = nil,
        lastSeenUnixMS: UInt64? = nil,
        launchMethod: String? = nil,
        error: [String: JSONValue]? = nil,
        metadata: [String: JSONValue] = [:]
    ) throws {
        guard !packageID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("package_id is required")
        }
        guard !packageVersion.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("package_version is required")
        }
        guard !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("display_name is required")
        }
        guard !platform.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("platform is required")
        }
        self.packageID = packageID
        self.packageVersion = packageVersion
        self.displayName = displayName
        self.platform = platform
        self.desiredState = desiredState
        self.supervisorState = supervisorState
        self.observedState = observedState
        self.projectedState = projectedState
        self.bootPolicy = bootPolicy
        self.stopPolicy = stopPolicy
        self.health = health
        self.pid = pid
        self.version = version
        self.lastSeenUnixMS = lastSeenUnixMS
        self.launchMethod = launchMethod
        self.error = error
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> DesktopCompanionStatus {
        try fromObject(decodeObject(raw, label: "desktop companion status JSON"))
    }

    public static func fromJSON(_ raw: String) throws -> DesktopCompanionStatus {
        try fromJSON(Data(raw.utf8))
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> DesktopCompanionStatus {
        try DesktopCompanionStatus(
            packageID: companionRequiredString(object, "package_id"),
            packageVersion: companionRequiredString(object, "package_version"),
            displayName: companionRequiredString(object, "display_name"),
            platform: companionRequiredString(object, "platform"),
            desiredState: companionRequiredEnum(object, "desired_state", CompanionDesiredState.self),
            supervisorState: companionRequiredEnum(object, "supervisor_state", CompanionSupervisorState.self),
            observedState: companionRequiredEnum(object, "observed_state", CompanionObservedState.self),
            projectedState: companionRequiredEnum(object, "projected_state", CompanionProjectedState.self),
            bootPolicy: companionRequiredEnum(object, "boot_policy", CompanionBootPolicy.self),
            stopPolicy: companionRequiredEnum(object, "stop_policy", CompanionStopPolicy.self),
            health: companionRequiredEnum(object, "health", CompanionHealthMode.self),
            pid: companionOptionalUInt(object["pid"], "pid"),
            version: companionOptionalString(object["version"], "version"),
            lastSeenUnixMS: companionOptionalUInt(object["last_seen_unix_ms"], "last_seen_unix_ms"),
            launchMethod: companionOptionalString(object["launch_method"], "launch_method"),
            error: companionOptionalObject(object["error"], "error"),
            metadata: companionOptionalObject(object["metadata"], "metadata") ?? [:]
        )
    }
}

public struct DesktopCompanionList: Equatable, Sendable {
    public let companions: [DesktopCompanionStatus]

    public init(companions: [DesktopCompanionStatus]) {
        self.companions = companions
    }

    public static func fromJSON(_ raw: Data) throws -> DesktopCompanionList {
        let object = try decodeObject(raw, label: "desktop companion list JSON")
        guard case let .array(items) = object["companions"] else {
            throw invalidCompanion("companions must be an array")
        }
        return try DesktopCompanionList(
            companions: items.map { item in
                guard case let .object(status) = item else {
                    throw invalidCompanion("companions entries must be objects")
                }
                return try DesktopCompanionStatus.fromObject(status)
            }
        )
    }

    public static func fromJSON(_ raw: String) throws -> DesktopCompanionList {
        try fromJSON(Data(raw.utf8))
    }
}

public struct DesktopCompanionActionResult: Equatable, Sendable {
    public let packageID: String
    public let action: String
    public let changed: Bool
    public let statusBefore: DesktopCompanionStatus?
    public let statusAfter: DesktopCompanionStatus?
    public let error: [String: JSONValue]?
    public let metadata: [String: JSONValue]

    public init(
        packageID: String,
        action: String,
        changed: Bool,
        statusBefore: DesktopCompanionStatus? = nil,
        statusAfter: DesktopCompanionStatus? = nil,
        error: [String: JSONValue]? = nil,
        metadata: [String: JSONValue] = [:]
    ) throws {
        guard !packageID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("package_id is required")
        }
        guard !action.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw invalidCompanion("action is required")
        }
        self.packageID = packageID
        self.action = action
        self.changed = changed
        self.statusBefore = statusBefore
        self.statusAfter = statusAfter
        self.error = error
        self.metadata = metadata
    }

    public static func fromJSON(_ raw: Data) throws -> DesktopCompanionActionResult {
        let object = try decodeObject(raw, label: "desktop companion action JSON")
        return try DesktopCompanionActionResult(
            packageID: companionRequiredString(object, "package_id"),
            action: companionRequiredString(object, "action"),
            changed: companionRequiredBool(object, "changed"),
            statusBefore: companionOptionalStatus(object["status_before"], "status_before"),
            statusAfter: companionOptionalStatus(object["status_after"], "status_after"),
            error: companionOptionalObject(object["error"], "error"),
            metadata: companionOptionalObject(object["metadata"], "metadata") ?? [:]
        )
    }

    public static func fromJSON(_ raw: String) throws -> DesktopCompanionActionResult {
        try fromJSON(Data(raw.utf8))
    }
}

public protocol CompanionTransport: AnyObject, Sendable {
    func companionList() async throws -> Data
    func companionStatus(packageID: String, packageVersion: String) async throws -> Data
    func companionEnable(packageID: String, packageVersion: String) async throws -> Data
    func companionDisable(packageID: String, packageVersion: String) async throws -> Data
    func companionStart(packageID: String, packageVersion: String) async throws -> Data
    func companionStop(packageID: String, packageVersion: String) async throws -> Data
    func close() async throws
}

public extension CompanionTransport {
    func close() async throws {}
}

public final class CompanionClient: @unchecked Sendable {
    private let transport: CompanionTransport
    private var closed = false

    public init(transport: CompanionTransport) {
        self.transport = transport
    }

    public func list() async throws -> DesktopCompanionList {
        try requireOpen()
        return try await companionCall("desktop companion list failed") {
            try DesktopCompanionList.fromJSON(try await transport.companionList())
        }
    }

    public func status(packageID: String, packageVersion: String = "") async throws -> DesktopCompanionStatus {
        try requireOpen()
        let input = try companionInput(packageID: packageID, packageVersion: packageVersion)
        return try await companionCall("desktop companion status failed") {
            try DesktopCompanionStatus.fromJSON(
                try await transport.companionStatus(packageID: input.packageID, packageVersion: input.packageVersion)
            )
        }
    }

    public func enable(packageID: String, packageVersion: String = "") async throws -> DesktopCompanionActionResult {
        try await action("enable", packageID: packageID, packageVersion: packageVersion)
    }

    public func disable(packageID: String, packageVersion: String = "") async throws -> DesktopCompanionActionResult {
        try await action("disable", packageID: packageID, packageVersion: packageVersion)
    }

    public func start(packageID: String, packageVersion: String = "") async throws -> DesktopCompanionActionResult {
        try await action("start", packageID: packageID, packageVersion: packageVersion)
    }

    public func stop(packageID: String, packageVersion: String = "") async throws -> DesktopCompanionActionResult {
        try await action("stop", packageID: packageID, packageVersion: packageVersion)
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await transport.close()
    }

    private func action(_ name: String, packageID: String, packageVersion: String) async throws -> DesktopCompanionActionResult {
        try requireOpen()
        let input = try companionInput(packageID: packageID, packageVersion: packageVersion)
        return try await companionCall("desktop companion \(name) failed") {
            let raw: Data
            switch name {
            case "enable":
                raw = try await transport.companionEnable(packageID: input.packageID, packageVersion: input.packageVersion)
            case "disable":
                raw = try await transport.companionDisable(packageID: input.packageID, packageVersion: input.packageVersion)
            case "start":
                raw = try await transport.companionStart(packageID: input.packageID, packageVersion: input.packageVersion)
            case "stop":
                raw = try await transport.companionStop(packageID: input.packageID, packageVersion: input.packageVersion)
            default:
                throw invalidCompanion("unsupported desktop companion action")
            }
            return try DesktopCompanionActionResult.fromJSON(raw)
        }
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("desktop_companion")
        }
    }
}

private func companionInput(packageID: String, packageVersion: String) throws -> (packageID: String, packageVersion: String) {
    let cleanedID = packageID.trimmingCharacters(in: .whitespacesAndNewlines)
    if cleanedID.isEmpty {
        throw invalidCompanion("package_id is required")
    }
    return (cleanedID, packageVersion.trimmingCharacters(in: .whitespacesAndNewlines))
}

private func companionCall<T>(_ message: String, operation: () async throws -> T) async throws -> T {
    do {
        return try await operation()
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(
            code: .transport,
            stage: "transport",
            retryHint: .safe,
            retryable: true,
            message: message
        )
    }
}

private func companionRequiredString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return value
    }
    throw invalidCompanion("\(name) must be a non-empty string")
}

private func companionOptionalString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .string(string):
        return string
    default:
        throw invalidCompanion("\(name) must be a string or null")
    }
}

private func companionRequiredBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidCompanion("\(name) must be a boolean")
}

private func companionRequiredEnum<T: RawRepresentable>(_ object: [String: JSONValue], _ name: String, _ type: T.Type) throws -> T where T.RawValue == String {
    let raw = try companionRequiredString(object, name)
    guard let value = T(rawValue: raw) else {
        throw invalidCompanion("\(name) is unsupported")
    }
    return value
}

private func companionOptionalUInt(_ value: JSONValue?, _ name: String) throws -> UInt64? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .number(number):
        guard number >= 0, number.rounded() == number, number <= Double(UInt64.max) else {
            throw invalidCompanion("\(name) must be a non-negative integer or null")
        }
        return UInt64(number)
    default:
        throw invalidCompanion("\(name) must be a non-negative integer or null")
    }
}

private func companionOptionalObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidCompanion("\(name) must be an object or null")
    }
}

private func companionOptionalStatus(_ value: JSONValue?, _ name: String) throws -> DesktopCompanionStatus? {
    guard let value else {
        return nil
    }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return try DesktopCompanionStatus.fromObject(object)
    default:
        throw invalidCompanion("\(name) must be an object or null")
    }
}

private func invalidCompanion(_ message: String) -> SDKError {
    SDKError(
        code: .invalidArgument,
        stage: "desktop_companion",
        retryHint: .never,
        retryable: false,
        message: message,
        details: ["profile": "desktop_companion"]
    )
}
