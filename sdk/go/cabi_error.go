package easynet

import "fmt"

func cabiErrorFromLastErrorJSON(raw []byte, ok bool, code int32, operation string) error {
	if ok {
		if decoded, err := DecodeDaemonErrorJSON(raw); err == nil && decoded != nil {
			return decoded
		}
	}
	return &SDKError{
		Code:      ErrGeneric,
		Stage:     "cabi",
		Retry:     RetryUnknown,
		Retryable: false,
		Message:   fmt.Sprintf("%s with code %d", operation, code),
	}
}
