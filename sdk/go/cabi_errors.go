package easynet

import "fmt"

// cabiErrorFromLastErrorJSON projects the generic C ABI error slot into the
// SDK's typed error model. It is shared by discovery and invocation handles;
// no product profile owns C ABI error decoding.
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
