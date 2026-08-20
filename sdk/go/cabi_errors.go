package easynet

import "fmt"

// cabiErrorFromLastErrorJSON projects the generic C ABI error slot into the
// SDK's typed error model. It is shared by discovery and invocation handles;
// no product profile owns C ABI error decoding.
func cabiErrorFromLastErrorJSON(raw []byte, ok bool, code int32, operation string) error {
	if ok {
		if decoded, err := decodeRuntimeErrorJSON(raw); err == nil && decoded != nil {
			return decoded
		}
	}
	metadata := cabiErrorMetadataForCode(code)
	return &SDKError{
		Code:      metadata.code,
		Stage:     metadata.stage,
		Retry:     metadata.retry,
		Retryable: RetryableForHint(metadata.retry),
		Message:   fmt.Sprintf("%s with code %d", operation, code),
		Source:    "c_abi",
		Details: map[string]any{
			"abi_code":   code,
			"abi_symbol": metadata.abiSymbol,
		},
	}
}

type cabiErrorMetadata struct {
	code      ErrorCode
	abiSymbol string
	stage     string
	retry     RetryHint
}

func cabiErrorMetadataForCode(code int32) cabiErrorMetadata {
	switch code {
	case 2:
		return cabiErrorMetadata{code: ErrNullPointer, abiSymbol: "ERR_NULL_POINTER", stage: "sdk", retry: RetryNever}
	case 3:
		return cabiErrorMetadata{code: ErrInvalidUTF8, abiSymbol: "ERR_INVALID_UTF8", stage: "sdk", retry: RetryNever}
	case 4:
		return cabiErrorMetadata{code: ErrInvalidHandle, abiSymbol: "ERR_INVALID_HANDLE", stage: "sdk", retry: RetryNever}
	case 5:
		return cabiErrorMetadata{code: ErrNotInitialized, abiSymbol: "ERR_NOT_INITIALIZED", stage: "sdk", retry: RetryNever}
	case 6:
		return cabiErrorMetadata{code: ErrAlreadyInit, abiSymbol: "ERR_ALREADY_INIT", stage: "sdk", retry: RetryNever}
	case 7:
		return cabiErrorMetadata{code: ErrRuntimeOffline, abiSymbol: "ERR_DAEMON_DOWN", stage: "transport", retry: RetryAfterBackoff}
	case 8:
		return cabiErrorMetadata{code: ErrVersionMismatch, abiSymbol: "ERR_VERSION_INCOMPATIBLE", stage: "sdk", retry: RetryNever}
	case 9:
		return cabiErrorMetadata{code: ErrAdmissionDenied, abiSymbol: "ERR_ABILITY_FAILED", stage: "runtime", retry: RetryUnknown}
	case 10:
		return cabiErrorMetadata{code: ErrNotImplemented, abiSymbol: "ERR_NOT_IMPLEMENTED", stage: "sdk", retry: RetryNever}
	case 11:
		return cabiErrorMetadata{code: ErrInvalidArgument, abiSymbol: "ERR_INVALID_ARG", stage: "sdk", retry: RetryNever}
	case 12:
		return cabiErrorMetadata{code: ErrPermissionDenied, abiSymbol: "ERR_PERMISSION_DENIED", stage: "runtime", retry: RetryNever}
	case 13:
		return cabiErrorMetadata{code: ErrAbilityNotFound, abiSymbol: "ERR_NOT_FOUND", stage: "runtime", retry: RetryNever}
	case 14:
		return cabiErrorMetadata{code: ErrCancelled, abiSymbol: "ERR_CANCELLED", stage: "client", retry: RetryNever}
	case 15:
		return cabiErrorMetadata{code: ErrProtocolMismatch, abiSymbol: "ERR_PROTOCOL", stage: "protocol", retry: RetryNever}
	case 16:
		return cabiErrorMetadata{code: ErrTimeout, abiSymbol: "ERR_TIMEOUT", stage: "transport", retry: RetrySafe}
	default:
		return cabiErrorMetadata{code: ErrGeneric, abiSymbol: "ERR_GENERIC", stage: "sdk", retry: RetryUnknown}
	}
}
