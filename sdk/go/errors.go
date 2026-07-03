package easynet

import (
	"errors"
	"fmt"
)

// ErrorCode is the stable Go SDK error classification.
type ErrorCode string

const (
	ErrorInvalidArgument     ErrorCode = "InvalidArgument"
	ErrorDaemonDown          ErrorCode = "DaemonDown"
	ErrorVersionIncompatible ErrorCode = "VersionIncompatible"
	ErrorTransport           ErrorCode = "Transport"
)

// RetryHint describes whether a failed operation may be retried.
type RetryHint string

const (
	RetryNever RetryHint = "never"
	RetrySafe  RetryHint = "safe"
)

// SDKError is the typed error boundary used by Go SDK callers.
type SDKError struct {
	Code    ErrorCode
	Stage   string
	Retry   RetryHint
	Message string
	Cause   error
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

// IsCode reports whether err is an SDKError with the requested code.
func IsCode(err error, code ErrorCode) bool {
	var sdkErr *SDKError
	return errors.As(err, &sdkErr) && sdkErr.Code == code
}
