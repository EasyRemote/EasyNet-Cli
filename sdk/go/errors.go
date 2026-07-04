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

	ErrorInvalidArgument     = ErrInvalidArgument
	ErrorDaemonDown          = ErrDaemonOffline
	ErrorVersionIncompatible = ErrVersionIncompatible
	ErrorTransport           = ErrTransport
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
	return &SDKError{
		Code:         NormalizeErrorCode(dto.Code),
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

// NormalizeErrorCode maps daemon/C ABI wire codes into the SDK taxonomy.
func NormalizeErrorCode(code string) ErrorCode {
	switch code {
	case "InvalidArgument", "INVALID_ARGUMENT":
		return ErrInvalidArgument
	case "InvalidHandle", "INVALID_HANDLE":
		return ErrInvalidHandle
	case "NullPointer", "NULL_POINTER":
		return ErrNullPointer
	case "InvalidUTF8", "INVALID_UTF8":
		return ErrInvalidUTF8
	case "NotInitialized", "NOT_INITIALIZED":
		return ErrNotInitialized
	case "AlreadyInit", "ALREADY_INIT":
		return ErrAlreadyInit
	case "DaemonDown", "DAEMON_DOWN", "DAEMON_OFFLINE":
		return ErrDaemonOffline
	case "PermissionDenied", "PERMISSION_DENIED":
		return ErrPermissionDenied
	case "AdmissionDenied", "ADMISSION_DENIED":
		return ErrAdmissionDenied
	case "AbilityNotFound", "ABILITY_NOT_FOUND":
		return ErrAbilityNotFound
	case "RouteUnavailable", "ROUTE_UNAVAILABLE":
		return ErrRouteUnavailable
	case "Timeout", "TIMEOUT":
		return ErrTimeout
	case "Cancelled", "CANCELLED":
		return ErrCancelled
	case "InvalidInvocation", "INVALID_INVOCATION":
		return ErrInvalidInvocation
	case "ProtocolMismatch", "PROTOCOL_MISMATCH":
		return ErrProtocolMismatch
	case "VersionMismatch", "VERSION_MISMATCH":
		return ErrVersionMismatch
	case "VersionIncompatible", "VERSION_INCOMPATIBLE":
		return ErrVersionIncompatible
	case "ControlOnly", "CONTROL_ONLY":
		return ErrControlOnly
	case "Transport", "TRANSPORT":
		return ErrTransport
	case "Protocol", "PROTOCOL":
		return ErrProtocol
	case "NotFound", "NOT_FOUND":
		return ErrNotFound
	case "AbilityFailed", "ABILITY_FAILED":
		return ErrAbilityFailed
	case "NotImplemented", "NOT_IMPLEMENTED":
		return ErrNotImplemented
	case "Generic", "GENERIC":
		return ErrGeneric
	default:
		return ErrorCode(code)
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
		value["source_ref"] = "go_sdk.profile." + profile
	}
	return value
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
