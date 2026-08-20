import Foundation

public enum SDKErrorCode: String, Sendable {
    case invalidArgument = "INVALID_ARGUMENT"
    case invalidHandle = "INVALID_HANDLE"
    case nullPointer = "NULL_POINTER"
    case invalidUTF8 = "INVALID_UTF8"
    case notInitialized = "NOT_INITIALIZED"
    case alreadyInit = "ALREADY_INIT"
    case runtimeOffline = "RUNTIME_OFFLINE"
    case permissionDenied = "PERMISSION_DENIED"
    case admissionDenied = "ADMISSION_DENIED"
    case httpAuthDenied = "HTTP_AUTH_DENIED"
    case signatureDenied = "SIGNATURE_DENIED"
    case policyDenied = "POLICY_DENIED"
    case authorityDenied = "AUTHORITY_DENIED"
    case authoritySubjectMismatch = "AUTHORITY_SUBJECT_MISMATCH"
    case abilityNotFound = "ABILITY_NOT_FOUND"
    case routeUnavailable = "ROUTE_UNAVAILABLE"
    case executionFailed = "EXECUTION_FAILED"
    case timeout = "TIMEOUT"
    case cancelled = "CANCELLED"
    case invalidInvocation = "INVALID_INVOCATION"
    case protocolMismatch = "PROTOCOL_MISMATCH"
    case versionMismatch = "VERSION_MISMATCH"
    case versionIncompatible = "VERSION_INCOMPATIBLE"
    case controlOnly = "CONTROL_ONLY"
    case transport = "TRANSPORT"
    case protocolFailure = "PROTOCOL"
    case notFound = "NOT_FOUND"
    case abilityFailed = "ABILITY_FAILED"
    case notImplemented = "NOT_IMPLEMENTED"
    case generic = "GENERIC"
    case callerIdentityUnavailable = "CALLER_IDENTITY_UNAVAILABLE"
    case callerSignerUnavailable = "CALLER_SIGNER_UNAVAILABLE"
    case descriptorNotFound = "DESCRIPTOR_NOT_FOUND"
    case descriptorOwnerOffline = "DESCRIPTOR_OWNER_OFFLINE"
    case descriptorModeUnsupported = "DESCRIPTOR_MODE_UNSUPPORTED"
    case descriptorStale = "DESCRIPTOR_STALE"
    case runtimeRouteUnavailable = "RUNTIME_ROUTE_UNAVAILABLE"
    case invocationCancelled = "INVOCATION_CANCELLED"
    case invocationTimeout = "INVOCATION_TIMEOUT"
    case terminalReceiptUnavailable = "TERMINAL_RECEIPT_UNAVAILABLE"
    case receiptProofFactsMissing = "RECEIPT_PROOF_FACTS_MISSING"
    case providerUnavailable = "PROVIDER_UNAVAILABLE"
}

public enum SDKErrorClass: String, Sendable {
    case validation
    case handle
    case lifecycle
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
        case .invalidArgument, .nullPointer, .invalidUTF8, .invalidInvocation:
            return .validation
        case .invalidHandle:
            return .handle
        case .notInitialized, .alreadyInit:
            return .lifecycle
        case .runtimeOffline, .transport:
            return .availability
        case .permissionDenied, .httpAuthDenied, .callerIdentityUnavailable:
            return .permission
        case .admissionDenied, .signatureDenied, .policyDenied, .authorityDenied, .authoritySubjectMismatch, .executionFailed, .abilityFailed, .callerSignerUnavailable, .receiptProofFactsMissing:
            return .admission
        case .abilityNotFound, .routeUnavailable, .notFound, .descriptorNotFound, .descriptorOwnerOffline, .descriptorModeUnsupported, .descriptorStale, .runtimeRouteUnavailable, .providerUnavailable:
            return .routing
        case .timeout, .invocationTimeout:
            return .timeout
        case .cancelled, .invocationCancelled:
            return .cancellation
        case .protocolFailure, .protocolMismatch:
            return .protocolFailure
        case .versionMismatch, .versionIncompatible:
            return .version
        case .controlOnly:
            return .control
        case .notImplemented:
            return .unsupported
        case .terminalReceiptUnavailable, .generic:
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
