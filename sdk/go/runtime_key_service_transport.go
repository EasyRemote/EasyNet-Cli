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
	// runtimeKeyServiceProtocolVersion is the only accepted managed-signing
	// protocol shape. v2 adds purpose-bound signing intents; v1 downgrade is
	// rejected.
	runtimeKeyServiceProtocolVersion = 2
	// runtimeKeyServiceMaxCanonicalSigningBytes is the canonical runtime
	// signing boundary. Base64 expands a maximum payload to roughly 86 MiB;
	// the wire frame therefore reserves 90 MiB for the JSON envelope.
	runtimeKeyServiceMaxCanonicalSigningBytes = 64 * 1024 * 1024
	runtimeKeyServiceMaxFrameBytes            = 90 * 1024 * 1024
)

var runtimeKeyServiceForbiddenCustodyFields = [...]string{
	"seed",
	"private",
	"vault",
	"passphrase",
	"master_key",
	"ciphertext",
}

var (
	errRuntimeKeyServiceNotFound    = errors.New("runtime key service: entry not found")
	errRuntimeKeyServiceUnavailable = errors.New("runtime key service unavailable")
)

type runtimeKeyServiceRejection struct {
	kind    string
	message string
}

func (e *runtimeKeyServiceRejection) Error() string {
	if e == nil {
		return "runtime key service rejection"
	}
	return fmt.Sprintf("runtime key service rejected request (%s): %s", e.kind, e.message)
}

func (e *runtimeKeyServiceRejection) Unwrap() error {
	if e != nil && e.kind == "not_found" {
		return errRuntimeKeyServiceNotFound
	}
	return nil
}

type runtimeKeyServiceClient struct {
	socketPath string
	timeout    time.Duration
}

func newRuntimeKeyServiceClient(socketPath string, timeout time.Duration) (runtimeKeyServiceClient, error) {
	socketPath = strings.TrimSpace(socketPath)
	if socketPath == "" {
		return runtimeKeyServiceClient{}, invalidRuntimeKeyServiceInput("runtime key-service endpoint is required")
	}
	if timeout <= 0 {
		timeout = 10 * time.Second
	}
	return runtimeKeyServiceClient{socketPath: socketPath, timeout: timeout}, nil
}
func (c runtimeKeyServiceClient) call(request map[string]any) (map[string]json.RawMessage, error) {
	encoded, err := encodeRuntimeKeyServiceRequest(request)
	if err != nil {
		return nil, err
	}

	deadline := time.Now().Add(c.timeout)
	dialer := net.Dialer{Deadline: deadline}
	connection, err := dialer.Dial("unix", c.socketPath)
	if err != nil {
		cause := fmt.Errorf("%w at %s: %v", errRuntimeKeyServiceUnavailable, c.socketPath, err)
		return nil, &SDKError{
			Code:      ErrRuntimeOffline,
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
		return nil, runtimeKeyServiceTransportError("set runtime key-service deadline", err)
	}

	var length [4]byte
	binary.BigEndian.PutUint32(length[:], uint32(len(encoded)))
	if err := writeRuntimeKeyServiceBytes(connection, length[:]); err != nil {
		return nil, runtimeKeyServiceTransportError("write runtime key-service frame length", err)
	}
	if err := writeRuntimeKeyServiceBytes(connection, encoded); err != nil {
		return nil, runtimeKeyServiceTransportError("write runtime key-service frame", err)
	}
	if _, err := io.ReadFull(connection, length[:]); err != nil {
		return nil, runtimeKeyServiceTransportError("read runtime key-service frame length", err)
	}
	responseLen := binary.BigEndian.Uint32(length[:])
	if responseLen > runtimeKeyServiceMaxFrameBytes {
		return nil, invalidRuntimeKeyServicePayload("runtime key-service response exceeds frame limit", nil)
	}
	response := make([]byte, responseLen)
	if _, err := io.ReadFull(connection, response); err != nil {
		return nil, runtimeKeyServiceTransportError("read runtime key-service frame", err)
	}
	if err := rejectRuntimeKeyServiceCustodyFields(response); err != nil {
		return nil, err
	}
	var decoded map[string]json.RawMessage
	if err := decodeRuntimeKeyServiceJSON(response, &decoded, false); err != nil {
		return nil, invalidRuntimeKeyServicePayload("decode runtime key-service response", err)
	}
	result, err := runtimeKeyServiceResponseString(decoded, "result")
	if err != nil {
		return nil, err
	}
	if result != "error" {
		return decoded, nil
	}
	if err := validateRuntimeKeyServiceResponseFields(decoded, "kind", "message"); err != nil {
		return nil, err
	}
	kind, err := runtimeKeyServiceResponseString(decoded, "kind")
	if err != nil {
		return nil, err
	}
	message, err := runtimeKeyServiceResponseString(decoded, "message")
	if err != nil {
		return nil, err
	}
	return nil, runtimeKeyServiceRejected(kind, message)
}

func encodeRuntimeKeyServiceRequest(request map[string]any) ([]byte, error) {
	encoded, err := json.Marshal(request)
	if err != nil {
		return nil, invalidRuntimeKeyServiceInput(fmt.Sprintf("encode runtime key-service request: %v", err))
	}
	if len(encoded) > runtimeKeyServiceMaxFrameBytes {
		return nil, invalidRuntimeKeyServiceInput("runtime key-service request exceeds frame limit")
	}
	return encoded, nil
}

func requireRuntimeKeyServiceResult(
	response map[string]json.RawMessage,
	expected string,
	allowedFields ...string,
) error {
	if err := validateRuntimeKeyServiceResponseFields(response, allowedFields...); err != nil {
		return err
	}
	result, err := runtimeKeyServiceResponseString(response, "result")
	if err != nil {
		return err
	}
	if result != expected {
		return invalidRuntimeKeyServicePayload(
			fmt.Sprintf("runtime key-service response result is %q, want %q", result, expected),
			nil,
		)
	}
	return nil
}

func validateRuntimeKeyServiceResponseFields(
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
			return invalidRuntimeKeyServicePayload(
				fmt.Sprintf("runtime key-service response contains unexpected field %q", field),
				nil,
			)
		}
	}
	return nil
}

func runtimeKeyServiceResponseString(response map[string]json.RawMessage, field string) (string, error) {
	raw, ok := response[field]
	if !ok {
		return "", invalidRuntimeKeyServicePayload(fmt.Sprintf("runtime key-service response missing %s", field), nil)
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil || value == "" {
		return "", invalidRuntimeKeyServicePayload(fmt.Sprintf("runtime key-service response field %s is not a non-empty string", field), err)
	}
	return value, nil
}

func writeRuntimeKeyServiceBytes(writer io.Writer, value []byte) error {
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

func runtimeKeyServiceRejected(kind, message string) error {
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
	rejection := &runtimeKeyServiceRejection{kind: kind, message: message}
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

func rejectRuntimeKeyServiceCustodyFields(encoded []byte) error {
	var value any
	if err := decodeRuntimeKeyServiceJSON(encoded, &value, false); err != nil {
		return invalidRuntimeKeyServicePayload("decode runtime key-service response", err)
	}
	return walkRuntimeKeyServiceResponse(value)
}

func walkRuntimeKeyServiceResponse(value any) error {
	switch value := value.(type) {
	case map[string]any:
		for field, child := range value {
			lower := strings.ToLower(field)
			for _, forbidden := range runtimeKeyServiceForbiddenCustodyFields {
				if strings.Contains(lower, forbidden) {
					return invalidRuntimeKeyServicePayload(
						fmt.Sprintf("runtime key-service response contains forbidden custody field %q", field),
						nil,
					)
				}
			}
			if err := walkRuntimeKeyServiceResponse(child); err != nil {
				return err
			}
		}
	case []any:
		for _, child := range value {
			if err := walkRuntimeKeyServiceResponse(child); err != nil {
				return err
			}
		}
	}
	return nil
}

func decodeRuntimeKeyServiceJSON(encoded []byte, target any, disallowUnknownFields bool) error {
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	if disallowUnknownFields {
		decoder.DisallowUnknownFields()
	}
	if err := decoder.Decode(target); err != nil {
		return err
	}
	if decoder.Decode(&struct{}{}) != io.EOF {
		return errors.New("runtime key-service JSON contains trailing data")
	}
	return nil
}

func invalidRuntimeKeyServiceInput(message string) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "key_service",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}

func invalidRuntimeKeyServicePayload(message string, cause error) error {
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     "key_service",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}

func runtimeKeyServiceTransportError(message string, cause error) error {
	return &SDKError{
		Code:      ErrTransport,
		Stage:     "key_service",
		Retry:     RetrySafe,
		Retryable: true,
		Message:   fmt.Sprintf("%s: %v", message, cause),
		Cause:     cause,
	}
}
