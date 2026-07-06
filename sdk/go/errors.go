package easynet

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

// ErrorCode is the stable Go SDK error classification.
type ErrorCode string

const (
	ErrInvalidArgument     ErrorCode = "INVALID_ARGUMENT"
	ErrInvalidHandle       ErrorCode = "INVALID_HANDLE"
	ErrNullPointer         ErrorCode = "NULL_POINTER"
	ErrInvalidUTF8         ErrorCode = "INVALID_UTF8"
	ErrNotInitialized      ErrorCode = "NOT_INITIALIZED"
	ErrAlreadyInit         ErrorCode = "ALREADY_INIT"
	ErrDaemonOffline       ErrorCode = "DAEMON_OFFLINE"
	ErrPermissionDenied    ErrorCode = "PERMISSION_DENIED"
	ErrAdmissionDenied     ErrorCode = "ADMISSION_DENIED"
	ErrAbilityNotFound     ErrorCode = "ABILITY_NOT_FOUND"
	ErrRouteUnavailable    ErrorCode = "ROUTE_UNAVAILABLE"
	ErrTimeout             ErrorCode = "TIMEOUT"
	ErrCancelled           ErrorCode = "CANCELLED"
	ErrInvalidInvocation   ErrorCode = "INVALID_INVOCATION"
	ErrProtocolMismatch    ErrorCode = "PROTOCOL_MISMATCH"
	ErrVersionMismatch     ErrorCode = "VERSION_MISMATCH"
	ErrControlOnly         ErrorCode = "CONTROL_ONLY"
	ErrTransport           ErrorCode = "TRANSPORT"
	ErrProtocol            ErrorCode = "PROTOCOL"
	ErrNotFound            ErrorCode = "NOT_FOUND"
	ErrAbilityFailed       ErrorCode = "ABILITY_FAILED"
	ErrNotImplemented      ErrorCode = "NOT_IMPLEMENTED"
	ErrGeneric             ErrorCode = "GENERIC"
	ErrVersionIncompatible ErrorCode = "VERSION_INCOMPATIBLE"
)

// ErrorClass is a language-side grouping derived from canonical ErrorCode.
type ErrorClass string

const (
	ErrorClassValidation   ErrorClass = "validation"
	ErrorClassHandle       ErrorClass = "handle"
	ErrorClassLifecycle    ErrorClass = "lifecycle"
	ErrorClassAvailability ErrorClass = "availability"
	ErrorClassPermission   ErrorClass = "permission"
	ErrorClassAdmission    ErrorClass = "admission"
	ErrorClassRouting      ErrorClass = "routing"
	ErrorClassTimeout      ErrorClass = "timeout"
	ErrorClassCancellation ErrorClass = "cancellation"
	ErrorClassProtocol     ErrorClass = "protocol"
	ErrorClassVersion      ErrorClass = "version"
	ErrorClassControl      ErrorClass = "control"
	ErrorClassUnsupported  ErrorClass = "unsupported"
	ErrorClassGeneric      ErrorClass = "generic"
)

// RetryHint describes whether a failed operation may be retried.
type RetryHint string

const (
	RetryNever        RetryHint = "never"
	RetrySafe         RetryHint = "safe"
	RetryAfterBackoff RetryHint = "after_backoff"
	RetryUnknown      RetryHint = "unknown"
)

// SDKError is the typed error boundary used by Go SDK callers.
type SDKError struct {
	Code         ErrorCode
	Stage        string
	Retry        RetryHint
	Retryable    bool
	Message      string
	Source       string
	InvocationID string
	ReceiptURA   string
	Details      map[string]any
	Cause        error
}

func (e *SDKError) Error() string {
	if e == nil {
		return "<nil>"
	}
	if e.Message == "" {
		return string(e.Code)
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}

func (e *SDKError) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Cause
}

// Class returns the stable language-side error grouping for this SDK error.
func (e *SDKError) Class() ErrorClass {
	if e == nil {
		return ErrorClassGeneric
	}
	return ErrorClassForCode(e.Code)
}

// Profile returns the profile detail attached to profile-originated SDK errors.
func (e *SDKError) Profile() string {
	if e == nil {
		return ""
	}
	return detailString(e.Details, "profile")
}

// SourceRef returns the stable language/package source reference for this error.
func (e *SDKError) SourceRef() string {
	if e == nil {
		return ""
	}
	return detailString(e.Details, "source_ref")
}

// RuntimeError is the Runtime Core error projection required by the daemon SDK.
type RuntimeError = SDKError

// RetryableForHint returns the explicit retryability represented by a retry hint.
func RetryableForHint(retry RetryHint) bool {
	return retry == RetrySafe || retry == RetryAfterBackoff
}

// IsCode reports whether err is an SDKError with the requested code.
func IsCode(err error, code ErrorCode) bool {
	var sdkErr *SDKError
	return errors.As(err, &sdkErr) && sdkErr.Code == code
}

// ErrorClassForCode projects an SDK error code into a stable language class.
func ErrorClassForCode(code ErrorCode) ErrorClass {
	switch code {
	case ErrInvalidArgument, ErrNullPointer, ErrInvalidUTF8, ErrInvalidInvocation:
		return ErrorClassValidation
	case ErrInvalidHandle:
		return ErrorClassHandle
	case ErrNotInitialized, ErrAlreadyInit:
		return ErrorClassLifecycle
	case ErrDaemonOffline, ErrTransport:
		return ErrorClassAvailability
	case ErrPermissionDenied:
		return ErrorClassPermission
	case ErrAdmissionDenied, ErrAbilityFailed:
		return ErrorClassAdmission
	case ErrAbilityNotFound, ErrRouteUnavailable, ErrNotFound:
		return ErrorClassRouting
	case ErrTimeout:
		return ErrorClassTimeout
	case ErrCancelled:
		return ErrorClassCancellation
	case ErrProtocolMismatch, ErrProtocol:
		return ErrorClassProtocol
	case ErrVersionMismatch, ErrVersionIncompatible:
		return ErrorClassVersion
	case ErrControlOnly:
		return ErrorClassControl
	case ErrNotImplemented:
		return ErrorClassUnsupported
	default:
		return ErrorClassGeneric
	}
}

// DecodeDaemonErrorJSON decodes the shared sdk/schemas/error.schema.json DTO.
func DecodeDaemonErrorJSON(raw []byte) (*SDKError, error) {
	if strings.TrimSpace(string(raw)) == "null" {
		return nil, nil
	}
	var dto daemonErrorDTO
	if err := json.Unmarshal(raw, &dto); err != nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "decode",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("decode daemon error JSON: %v", err),
			Cause:     err,
		}
	}
	if dto.Code == "" {
		return nil, invalidDaemonError("code is required")
	}
	if dto.Stage == "" {
		return nil, invalidDaemonError("stage is required")
	}
	if dto.Message == nil {
		return nil, invalidDaemonError("message is required")
	}
	retry, err := parseRetryHint(dto.Retry)
	if err != nil {
		return nil, err
	}
	details := dto.Details
	if details == nil {
		details = map[string]any{}
	}
	code, err := ParseErrorCode(dto.Code)
	if err != nil {
		return nil, err
	}
	return &SDKError{
		Code:         code,
		Stage:        dto.Stage,
		Retry:        retry,
		Retryable:    RetryableForHint(retry),
		Message:      *dto.Message,
		Source:       optionalString(dto.Source),
		InvocationID: optionalString(dto.InvocationID),
		ReceiptURA:   optionalString(dto.ReceiptURA),
		Details:      details,
	}, nil
}

type daemonErrorDTO struct {
	Code         string         `json:"code"`
	Stage        string         `json:"stage"`
	Message      *string        `json:"message"`
	Retry        string         `json:"retry"`
	Source       *string        `json:"source"`
	InvocationID *string        `json:"invocation_id"`
	ReceiptURA   *string        `json:"receipt_ura"`
	Details      map[string]any `json:"details"`
}

// ParseErrorCode validates the current canonical SDK error-code schema value.
func ParseErrorCode(code string) (ErrorCode, error) {
	switch code {
	case "INVALID_ARGUMENT":
		return ErrInvalidArgument, nil
	case "INVALID_HANDLE":
		return ErrInvalidHandle, nil
	case "NULL_POINTER":
		return ErrNullPointer, nil
	case "INVALID_UTF8":
		return ErrInvalidUTF8, nil
	case "NOT_INITIALIZED":
		return ErrNotInitialized, nil
	case "ALREADY_INIT":
		return ErrAlreadyInit, nil
	case "DAEMON_OFFLINE":
		return ErrDaemonOffline, nil
	case "PERMISSION_DENIED":
		return ErrPermissionDenied, nil
	case "ADMISSION_DENIED":
		return ErrAdmissionDenied, nil
	case "ABILITY_NOT_FOUND":
		return ErrAbilityNotFound, nil
	case "ROUTE_UNAVAILABLE":
		return ErrRouteUnavailable, nil
	case "TIMEOUT":
		return ErrTimeout, nil
	case "CANCELLED":
		return ErrCancelled, nil
	case "INVALID_INVOCATION":
		return ErrInvalidInvocation, nil
	case "PROTOCOL_MISMATCH":
		return ErrProtocolMismatch, nil
	case "VERSION_MISMATCH":
		return ErrVersionMismatch, nil
	case "VERSION_INCOMPATIBLE":
		return ErrVersionIncompatible, nil
	case "CONTROL_ONLY":
		return ErrControlOnly, nil
	case "TRANSPORT":
		return ErrTransport, nil
	case "PROTOCOL":
		return ErrProtocol, nil
	case "NOT_FOUND":
		return ErrNotFound, nil
	case "ABILITY_FAILED":
		return ErrAbilityFailed, nil
	case "NOT_IMPLEMENTED":
		return ErrNotImplemented, nil
	case "GENERIC":
		return ErrGeneric, nil
	default:
		return "", invalidDaemonError(fmt.Sprintf("unknown daemon error code: %s", code))
	}
}

func runtimeFailureCode(code string, fallback ErrorCode) ErrorCode {
	code = strings.TrimSpace(code)
	if code == "" {
		return fallback
	}
	parsed, err := ParseErrorCode(code)
	if err == nil {
		return parsed
	}
	if isLegacyErrorCodeAlias(code) {
		return ErrProtocolMismatch
	}
	return ErrorCode(code)
}

func isLegacyErrorCodeAlias(code string) bool {
	switch code {
	case "InvalidArgument",
		"InvalidHandle",
		"NullPointer",
		"InvalidUTF8",
		"NotInitialized",
		"AlreadyInit",
		"DaemonDown",
		"DAEMON_DOWN",
		"PermissionDenied",
		"AdmissionDenied",
		"AbilityNotFound",
		"RouteUnavailable",
		"Timeout",
		"Cancelled",
		"InvalidInvocation",
		"ProtocolMismatch",
		"VersionMismatch",
		"VersionIncompatible",
		"ControlOnly",
		"Transport",
		"Protocol",
		"NotFound",
		"AbilityFailed",
		"NotImplemented",
		"Generic":
		return true
	default:
		return false
	}
}

func profileErrorDetails(profile string, details map[string]any) map[string]any {
	value := make(map[string]any, len(details)+2)
	for key, item := range details {
		value[key] = item
	}
	if _, ok := value["profile"]; !ok {
		value["profile"] = profile
	}
	if _, ok := value["source_ref"]; !ok {
		value["source_ref"] = ProfileSourceRef(profile)
	}
	return value
}

// ProfileSourceRef returns the stable Go package source reference for a profile.
func ProfileSourceRef(profile string) string {
	clean := strings.TrimSpace(profile)
	if clean == "" {
		return ""
	}
	return "go_sdk.profile." + clean
}

func withProfileErrorDetails(err error, profile string) error {
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		return err
	}
	copy := *sdkErr
	copy.Details = profileErrorDetails(profile, sdkErr.Details)
	return &copy
}

func invalidProfileClient(profile string, message string) error {
	return withProfileErrorDetails(invalidRuntimeClient(message), profile)
}

func invalidProfilePayload(profile string, message string, cause error) error {
	return withProfileErrorDetails(invalidRuntimePayload(message, cause), profile)
}

func invalidProfilePayloadWithDetails(profile string, message string, details map[string]any, cause error) error {
	err := invalidRuntimePayload(message, cause)
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		return err
	}
	copy := *sdkErr
	copy.Details = profileErrorDetails(profile, details)
	return &copy
}

func transportProfileError(profile string, message string, cause error) error {
	return withProfileErrorDetails(transportRuntimeError(message, cause), profile)
}

func parseRetryHint(value string) (RetryHint, error) {
	switch RetryHint(value) {
	case RetryNever, RetrySafe, RetryAfterBackoff, RetryUnknown:
		return RetryHint(value), nil
	default:
		return "", invalidDaemonError("retry must be never, safe, after_backoff, or unknown")
	}
}

func invalidDaemonError(message string) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "decode",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}

func optionalString(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

func detailString(details map[string]any, key string) string {
	if details == nil {
		return ""
	}
	value, ok := details[key].(string)
	if !ok {
		return ""
	}
	return value
}
