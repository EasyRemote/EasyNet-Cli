package easynet

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"strings"
	"time"
)

const (
	// daemonKeyServiceProtocolVersion is the only accepted managed-signing
	// protocol shape. v2 adds purpose-bound signing intents; there is no v1
	// request fallback.
	daemonKeyServiceProtocolVersion = 2
	// daemonKeyServiceMaxCanonicalSigningBytes is the canonical runtime
	// signing boundary. Base64 expands a maximum payload to roughly 86 MiB;
	// the wire frame therefore reserves 90 MiB for the JSON envelope.
	daemonKeyServiceMaxCanonicalSigningBytes = 64 * 1024 * 1024
	daemonKeyServiceMaxFrameBytes            = 90 * 1024 * 1024
)

var daemonKeyServiceForbiddenCustodyFields = [...]string{
	"seed",
	"private",
	"vault",
	"passphrase",
	"master_key",
	"ciphertext",
}

var (
	// ErrDaemonKeyServiceNotFound identifies a missing public projection in
	// the daemon-owned key service.
	ErrDaemonKeyServiceNotFound = errors.New("daemon key service: entry not found")
	// ErrDaemonKeyServiceUnavailable identifies an unavailable local daemon
	// key-service endpoint.
	ErrDaemonKeyServiceUnavailable = errors.New("daemon key service unavailable")
)

// DaemonKeyServiceRejection preserves the daemon's stable rejection kind
// without forcing callers to parse human-readable messages.
type DaemonKeyServiceRejection struct {
	Kind    string
	Message string
}

func (e *DaemonKeyServiceRejection) Error() string {
	if e == nil {
		return "daemon key service rejection"
	}
	return fmt.Sprintf("daemon key service rejected request (%s): %s", e.Kind, e.Message)
}

func (e *DaemonKeyServiceRejection) Unwrap() error {
	if e != nil && e.Kind == "not_found" {
		return ErrDaemonKeyServiceNotFound
	}
	return nil
}

type daemonKeyServiceClient struct {
	socketPath string
	timeout    time.Duration
}

func newDaemonKeyServiceClient(socketPath string, timeout time.Duration) (daemonKeyServiceClient, error) {
	socketPath = strings.TrimSpace(socketPath)
	if socketPath == "" {
		return daemonKeyServiceClient{}, invalidDaemonKeyServiceInput("daemon key-service endpoint is required")
	}
	if timeout <= 0 {
		timeout = 10 * time.Second
	}
	return daemonKeyServiceClient{socketPath: socketPath, timeout: timeout}, nil
}
func (c daemonKeyServiceClient) call(request map[string]any) (map[string]json.RawMessage, error) {
	encoded, err := encodeDaemonKeyServiceRequest(request)
	if err != nil {
		return nil, err
	}

	deadline := time.Now().Add(c.timeout)
	dialer := net.Dialer{Deadline: deadline}
	connection, err := dialer.Dial("unix", c.socketPath)
	if err != nil {
		cause := fmt.Errorf("%w at %s: %v", ErrDaemonKeyServiceUnavailable, c.socketPath, err)
		return nil, &SDKError{
			Code:      ErrDaemonOffline,
			Stage:     "key_service",
			Retry:     RetrySafe,
			Retryable: true,
			Message:   cause.Error(),
			Cause:     cause,
		}
	}
	defer connection.Close()
	// Dial, request write, and response read consume one absolute transport
	// budget. A successful dial must not reset the timeout window.
	if err := connection.SetDeadline(deadline); err != nil {
		return nil, daemonKeyServiceTransportError("set daemon key-service deadline", err)
	}

	var length [4]byte
	binary.BigEndian.PutUint32(length[:], uint32(len(encoded)))
	if err := writeDaemonKeyServiceBytes(connection, length[:]); err != nil {
		return nil, daemonKeyServiceTransportError("write daemon key-service frame length", err)
	}
	if err := writeDaemonKeyServiceBytes(connection, encoded); err != nil {
		return nil, daemonKeyServiceTransportError("write daemon key-service frame", err)
	}
	if _, err := io.ReadFull(connection, length[:]); err != nil {
		return nil, daemonKeyServiceTransportError("read daemon key-service frame length", err)
	}
	responseLen := binary.BigEndian.Uint32(length[:])
	if responseLen > daemonKeyServiceMaxFrameBytes {
		return nil, invalidDaemonKeyServicePayload("daemon key-service response exceeds frame limit", nil)
	}
	response := make([]byte, responseLen)
	if _, err := io.ReadFull(connection, response); err != nil {
		return nil, daemonKeyServiceTransportError("read daemon key-service frame", err)
	}
	if err := rejectDaemonKeyServiceCustodyFields(response); err != nil {
		return nil, err
	}
	var decoded map[string]json.RawMessage
	if err := decodeDaemonKeyServiceJSON(response, &decoded, false); err != nil {
		return nil, invalidDaemonKeyServicePayload("decode daemon key-service response", err)
	}
	result, err := daemonKeyServiceResponseString(decoded, "result")
	if err != nil {
		return nil, err
	}
	if result != "error" {
		return decoded, nil
	}
	if err := validateDaemonKeyServiceResponseFields(decoded, "kind", "message"); err != nil {
		return nil, err
	}
	kind, err := daemonKeyServiceResponseString(decoded, "kind")
	if err != nil {
		return nil, err
	}
	message, err := daemonKeyServiceResponseString(decoded, "message")
	if err != nil {
		return nil, err
	}
	return nil, daemonKeyServiceRejected(kind, message)
}

func encodeDaemonKeyServiceRequest(request map[string]any) ([]byte, error) {
	encoded, err := json.Marshal(request)
	if err != nil {
		return nil, invalidDaemonKeyServiceInput(fmt.Sprintf("encode daemon key-service request: %v", err))
	}
	if len(encoded) > daemonKeyServiceMaxFrameBytes {
		return nil, invalidDaemonKeyServiceInput("daemon key-service request exceeds frame limit")
	}
	return encoded, nil
}

func requireDaemonKeyServiceResult(
	response map[string]json.RawMessage,
	expected string,
	allowedFields ...string,
) error {
	if err := validateDaemonKeyServiceResponseFields(response, allowedFields...); err != nil {
		return err
	}
	result, err := daemonKeyServiceResponseString(response, "result")
	if err != nil {
		return err
	}
	if result != expected {
		return invalidDaemonKeyServicePayload(
			fmt.Sprintf("daemon key-service response result is %q, want %q", result, expected),
			nil,
		)
	}
	return nil
}

func validateDaemonKeyServiceResponseFields(
	response map[string]json.RawMessage,
	allowedFields ...string,
) error {
	allowed := make(map[string]struct{}, len(allowedFields)+1)
	allowed["result"] = struct{}{}
	for _, field := range allowedFields {
		allowed[field] = struct{}{}
	}
	for field := range response {
		if _, ok := allowed[field]; !ok {
			return invalidDaemonKeyServicePayload(
				fmt.Sprintf("daemon key-service response contains unexpected field %q", field),
				nil,
			)
		}
	}
	return nil
}

func daemonKeyServiceResponseString(response map[string]json.RawMessage, field string) (string, error) {
	raw, ok := response[field]
	if !ok {
		return "", invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key-service response missing %s", field), nil)
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil || value == "" {
		return "", invalidDaemonKeyServicePayload(fmt.Sprintf("daemon key-service response field %s is not a non-empty string", field), err)
	}
	return value, nil
}

func writeDaemonKeyServiceBytes(writer io.Writer, value []byte) error {
	for len(value) > 0 {
		written, err := writer.Write(value)
		if err != nil {
			return err
		}
		if written == 0 {
			return io.ErrShortWrite
		}
		value = value[written:]
	}
	return nil
}

func daemonKeyServiceRejected(kind, message string) error {
	retry := RetryNever
	code := ErrProtocol
	switch kind {
	case "not_found":
		code = ErrNotFound
	case "already_exists":
		code = ErrAlreadyInit
	case "lifecycle", "policy":
		code = ErrPolicyDenied
	case "io", "crypto", "kdf", "durability_uncertain", "fail_stopped":
		code = ErrExecutionFailed
	case "base64", "serde", "corrupt", "bad_seed_len":
		code = ErrProtocol
	}
	rejection := &DaemonKeyServiceRejection{Kind: kind, Message: message}
	return &SDKError{
		Code:      code,
		Stage:     "key_service",
		Retry:     retry,
		Retryable: RetryableForHint(retry),
		Message:   rejection.Error(),
		Details:   map[string]any{"kind": kind},
		Cause:     rejection,
	}
}

func rejectDaemonKeyServiceCustodyFields(encoded []byte) error {
	var value any
	if err := decodeDaemonKeyServiceJSON(encoded, &value, false); err != nil {
		return invalidDaemonKeyServicePayload("decode daemon key-service response", err)
	}
	return walkDaemonKeyServiceResponse(value)
}

func walkDaemonKeyServiceResponse(value any) error {
	switch value := value.(type) {
	case map[string]any:
		for field, child := range value {
			lower := strings.ToLower(field)
			for _, forbidden := range daemonKeyServiceForbiddenCustodyFields {
				if strings.Contains(lower, forbidden) {
					return invalidDaemonKeyServicePayload(
						fmt.Sprintf("daemon key-service response contains forbidden custody field %q", field),
						nil,
					)
				}
			}
			if err := walkDaemonKeyServiceResponse(child); err != nil {
				return err
			}
		}
	case []any:
		for _, child := range value {
			if err := walkDaemonKeyServiceResponse(child); err != nil {
				return err
			}
		}
	}
	return nil
}

func decodeDaemonKeyServiceJSON(encoded []byte, target any, disallowUnknownFields bool) error {
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	if disallowUnknownFields {
		decoder.DisallowUnknownFields()
	}
	if err := decoder.Decode(target); err != nil {
		return err
	}
	if decoder.Decode(&struct{}{}) != io.EOF {
		return errors.New("daemon key-service JSON contains trailing data")
	}
	return nil
}

func invalidDaemonKeyServiceInput(message string) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "key_service",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}

func invalidDaemonKeyServicePayload(message string, cause error) error {
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     "key_service",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}

func daemonKeyServiceTransportError(message string, cause error) error {
	return &SDKError{
		Code:      ErrTransport,
		Stage:     "key_service",
		Retry:     RetrySafe,
		Retryable: true,
		Message:   fmt.Sprintf("%s: %v", message, cause),
		Cause:     cause,
	}
}
