//go:build !easynet_cabi || !cgo || windows

package easynet

import "context"

// OpenNativeRuntime reports that the native daemon Runtime provider is not
// compiled into this SDK build.
func OpenNativeRuntime(ctx context.Context, _ NativeRuntimeOptions) (*NativeRuntimeHandle, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	return nil, &SDKError{
		Code:      ErrNotImplemented,
		Stage:     "native_runtime",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "native runtime provider is not available in this build",
	}
}
