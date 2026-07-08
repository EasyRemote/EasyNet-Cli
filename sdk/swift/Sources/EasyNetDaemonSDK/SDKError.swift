import Foundation

public enum SDKErrorCode: String, Sendable {
    case invalidArgument = "INVALID_ARGUMENT"
    case invalidHandle = "INVALID_HANDLE"
    case daemonOffline = "DAEMON_OFFLINE"
    case permissionDenied = "PERMISSION_DENIED"
    case admissionDenied = "ADMISSION_DENIED"
    case httpAuthDenied = "HTTP_AUTH_DENIED"
    case signatureDenied = "SIGNATURE_DENIED"
    case policyDenied = "POLICY_DENIED"
    case authorityDenied = "AUTHORITY_DENIED"
    case abilityNotFound = "ABILITY_NOT_FOUND"
    case routeUnavailable = "ROUTE_UNAVAILABLE"
    case executionFailed = "EXECUTION_FAILED"
    case timeout = "TIMEOUT"
    case cancelled = "CANCELLED"
    case invalidInvocation = "INVALID_INVOCATION"
    case protocolMismatch = "PROTOCOL_MISMATCH"
    case versionIncompatible = "VERSION_INCOMPATIBLE"
    case controlOnly = "CONTROL_ONLY"
    case transport = "TRANSPORT"
    case protocolFailure = "PROTOCOL"
    case notImplemented = "NOT_IMPLEMENTED"
    case generic = "GENERIC"
}

public enum SDKErrorClass: String, Sendable {
    case validation
    case handle
    case availability
    case permission
    case admission
    case routing
    case timeout
    case cancellation
    case protocolFailure = "protocol"
    case version
    case control
    case unsupported
    case generic
}

public enum RetryHint: String, Sendable {
    case never
    case safe
    case afterBackoff = "after_backoff"
    case unknown
}

public struct SDKError: Error, Sendable, CustomStringConvertible {
    public let code: SDKErrorCode
    public let stage: String
    public let retryHint: RetryHint
    public let retryable: Bool
    public let message: String
    public let source: String
    public let invocationID: String
    public let receiptURA: String
    public let details: [String: String]

    public init(
        code: SDKErrorCode,
        stage: String,
        retryHint: RetryHint = .never,
        retryable: Bool = false,
        message: String,
        source: String = "",
        invocationID: String = "",
        receiptURA: String = "",
        details: [String: String] = [:]
    ) {
        self.code = code
        self.stage = SDKError.required(stage, "stage")
        self.retryHint = retryHint
        self.retryable = retryable
        self.message = SDKError.required(message, "message")
        self.source = source
        self.invocationID = invocationID
        self.receiptURA = receiptURA
        self.details = details
    }

    public static func validation(_ stage: String, _ message: String) -> SDKError {
        SDKError(code: .invalidArgument, stage: stage, message: message)
    }

    public static func closed(_ stage: String) -> SDKError {
        SDKError(code: .invalidHandle, stage: stage, message: "\(stage) is closed")
    }

    public var errorClass: SDKErrorClass {
        switch code {
        case .invalidArgument, .invalidInvocation:
            return .validation
        case .invalidHandle:
            return .handle
        case .daemonOffline, .routeUnavailable, .transport:
            return .availability
        case .permissionDenied, .httpAuthDenied:
            return .permission
        case .admissionDenied, .signatureDenied, .policyDenied, .authorityDenied, .executionFailed:
            return .admission
        case .abilityNotFound:
            return .routing
        case .timeout:
            return .timeout
        case .cancelled:
            return .cancellation
        case .protocolFailure, .protocolMismatch:
            return .protocolFailure
        case .versionIncompatible:
            return .version
        case .controlOnly:
            return .control
        case .notImplemented:
            return .unsupported
        case .generic:
            return .generic
        }
    }

    public var description: String {
        "\(code.rawValue): \(message)"
    }

    static func required(_ value: String?, _ field: String) -> String {
        guard let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return field
        }
        return value
    }
}
