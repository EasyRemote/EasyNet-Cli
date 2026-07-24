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
	CallerURA       string
	CalleeURA       string
	AbilityURA      string
	SubjectURA      string
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
	invocation, err := frame.projectInvocation()
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
	Type       string          `json:"type"`
	CallID     string          `json:"call_id"`
	Invocation json.RawMessage `json:"invocation"`
}

type sidecarInvocationFrame struct {
	CallerURA       string         `json:"caller_ura"`
	CalleeURA       string         `json:"callee_ura"`
	AbilityURA      string         `json:"ability_ura"`
	SubjectURA      string         `json:"subject_ura"`
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

func (f requestFrame) projectInvocation() (SidecarInvocation, error) {
	if len(f.Invocation) == 0 {
		return SidecarInvocation{}, protocolError("sidecar frame field \"invocation\" must be an object")
	}
	if err := rejectLegacyTupleAliases(f.Invocation); err != nil {
		return SidecarInvocation{}, err
	}
	var invocation sidecarInvocationFrame
	if err := json.Unmarshal(f.Invocation, &invocation); err != nil {
		return SidecarInvocation{}, protocolError("sidecar frame field \"invocation\" must be an object")
	}
	return invocation.project(f.Type, f.CallID)
}

func rejectLegacyTupleAliases(raw json.RawMessage) error {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err != nil {
		return protocolError("sidecar frame field \"invocation\" must be an object")
	}
	for legacy, canonical := range map[string]string{
		"caller":  "caller_ura",
		"callee":  "callee_ura",
		"ability": "ability_ura",
		"subject": "subject_ura",
	} {
		if _, ok := object[legacy]; ok {
			return protocolError("sidecar frame field %q is retired; use %q", legacy, canonical)
		}
	}
	return nil
}

func (f sidecarInvocationFrame) project(frameType string, callID string) (SidecarInvocation, error) {
	if frameType != "invoke" {
		return SidecarInvocation{}, protocolError("exec sidecar expected invoke frame, got %q", frameType)
	}
	for field, value := range map[string]string{
		"caller_ura":  f.CallerURA,
		"callee_ura":  f.CalleeURA,
		"ability_ura": f.AbilityURA,
		"subject_ura": f.SubjectURA,
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
		CallerURA:       f.CallerURA,
		CalleeURA:       f.CalleeURA,
		AbilityURA:      f.AbilityURA,
		SubjectURA:      f.SubjectURA,
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
