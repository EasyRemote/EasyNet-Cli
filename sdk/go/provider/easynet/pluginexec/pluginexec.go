// Package pluginexec owns the EasyNet provider sidecar frame facade.
//
// It is intentionally provider-scoped: the canonical SDK root must not expose
// EasyNet-Cli daemon sidecar execution concepts. Process-backed plugins should
// implement handlers over SidecarInvocation instead of hand-writing stdin/stdout
// JSON frames.
package pluginexec

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
)

// SidecarInvocation is the handler-facing view of one daemon-admitted sidecar call.
type SidecarInvocation struct {
	CallID          string
	Caller          string
	Callee          string
	Ability         string
	Subject         string
	InvocationNonce []int
	CausalContext   map[string]any
	Args            map[string]any
	FrameType       string
}

// Handler implements one sidecar invocation.
type Handler func(context.Context, SidecarInvocation) (any, error)

// ProtocolError reports malformed daemon sidecar frames.
type ProtocolError struct {
	message string
}

func (e *ProtocolError) Error() string {
	if e == nil {
		return ""
	}
	return e.message
}

// Serve handles one stdin/stdout sidecar invocation using process defaults.
func Serve(ctx context.Context, handler Handler) error {
	return ServeIO(ctx, os.Stdin, os.Stdout, handler)
}

// MustServe is a process main helper for generated plugin templates.
func MustServe(ctx context.Context, handler Handler) {
	if err := Serve(ctx, handler); err != nil {
		os.Exit(1)
	}
}

// ServeIO handles one sidecar invocation over explicit streams.
func ServeIO(ctx context.Context, input io.Reader, output io.Writer, handler Handler) error {
	callID := ""
	frame, err := readRequestFrame(input)
	if err == nil {
		callID = frame.CallID
	}
	if err != nil {
		return writeResponseFrame(output, responseFrame{
			Type:    "error",
			CallID:  callID,
			Message: err.Error(),
		})
	}
	invocation, err := frame.Invocation.project(frame.Type, frame.CallID)
	if err != nil {
		return writeResponseFrame(output, responseFrame{
			Type:    "error",
			CallID:  callID,
			Message: err.Error(),
		})
	}
	value, err := handler(ctx, invocation)
	if err != nil {
		return writeResponseFrame(output, responseFrame{
			Type:    "error",
			CallID:  invocation.CallID,
			Message: err.Error(),
		})
	}
	return writeResponseFrame(output, responseFrame{
		Type:   "result",
		CallID: invocation.CallID,
		Value:  value,
	})
}

func IsProtocolError(err error) bool {
	var target *ProtocolError
	return errors.As(err, &target)
}

type requestFrame struct {
	Type       string                 `json:"type"`
	CallID     string                 `json:"call_id"`
	Invocation sidecarInvocationFrame `json:"invocation"`
}

type sidecarInvocationFrame struct {
	Caller          string         `json:"caller"`
	Callee          string         `json:"callee"`
	Ability         string         `json:"ability"`
	Subject         string         `json:"subject"`
	InvocationNonce []int          `json:"invocation_nonce"`
	CausalContext   map[string]any `json:"causal_context"`
	Args            map[string]any `json:"args"`
}

type responseFrame struct {
	Type    string `json:"type"`
	CallID  string `json:"call_id"`
	Value   any    `json:"value,omitempty"`
	Message string `json:"message,omitempty"`
}

func readRequestFrame(input io.Reader) (requestFrame, error) {
	line, err := bufio.NewReader(input).ReadBytes('\n')
	if err != nil && !errors.Is(err, io.EOF) {
		return requestFrame{}, protocolError("read sidecar request frame: %v", err)
	}
	if len(line) == 0 {
		return requestFrame{}, protocolError("missing sidecar request frame")
	}
	var frame requestFrame
	if err := json.Unmarshal(line, &frame); err != nil {
		return requestFrame{}, protocolError("invalid sidecar request JSON: %v", err)
	}
	if frame.CallID == "" {
		return requestFrame{}, protocolError("sidecar frame field \"call_id\" must be a string")
	}
	return frame, nil
}

func (f sidecarInvocationFrame) project(frameType string, callID string) (SidecarInvocation, error) {
	if frameType != "invoke" {
		return SidecarInvocation{}, protocolError("exec sidecar expected invoke frame, got %q", frameType)
	}
	for field, value := range map[string]string{
		"caller":  f.Caller,
		"callee":  f.Callee,
		"ability": f.Ability,
		"subject": f.Subject,
	} {
		if value == "" {
			return SidecarInvocation{}, protocolError("sidecar frame field %q must be a string", field)
		}
	}
	if len(f.InvocationNonce) == 0 {
		return SidecarInvocation{}, protocolError("sidecar frame field \"invocation_nonce\" must be a byte array")
	}
	for _, item := range f.InvocationNonce {
		if item < 0 || item > 255 {
			return SidecarInvocation{}, protocolError("sidecar frame field \"invocation_nonce\" must contain bytes")
		}
	}
	args := f.Args
	if args == nil {
		args = map[string]any{}
	}
	causalContext := f.CausalContext
	if causalContext == nil {
		causalContext = map[string]any{}
	}
	return SidecarInvocation{
		CallID:          callID,
		Caller:          f.Caller,
		Callee:          f.Callee,
		Ability:         f.Ability,
		Subject:         f.Subject,
		InvocationNonce: append([]int(nil), f.InvocationNonce...),
		CausalContext:   causalContext,
		Args:            args,
		FrameType:       frameType,
	}, nil
}

func writeResponseFrame(output io.Writer, frame responseFrame) error {
	encoded, err := json.Marshal(frame)
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	_, err = output.Write(encoded)
	return err
}

func protocolError(format string, args ...any) error {
	return &ProtocolError{message: fmt.Sprintf(format, args...)}
}
