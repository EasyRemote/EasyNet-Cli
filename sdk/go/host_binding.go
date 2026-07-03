package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const hostStreamFrameSchema = "host-stream-frame.schema.json"
const hostStreamHashAlgorithm = "sha256(prev_hash || seq_be || canonical_json(value))"

// HostStreamBindingRequest declares a daemon-to-host execution binding.
type HostStreamBindingRequest struct {
	BindingID     string         `json:"binding_id"`
	DescriptorRef string         `json:"descriptor_ref"`
	Endpoint      string         `json:"endpoint"`
	FrameSchema   string         `json:"frame_schema"`
	Cleanup       map[string]any `json:"cleanup,omitempty"`
	TimeoutMS     *int64         `json:"timeout_ms,omitempty"`
	Readiness     map[string]any `json:"readiness,omitempty"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

// HostStreamBinding is the schema-backed host-stream binding projection.
type HostStreamBinding struct {
	BindingID     string         `json:"binding_id"`
	DescriptorRef string         `json:"descriptor_ref"`
	Endpoint      string         `json:"endpoint"`
	FrameSchema   string         `json:"frame_schema"`
	Cleanup       map[string]any `json:"cleanup"`
	TimeoutMS     *int64         `json:"timeout_ms"`
	Readiness     map[string]any `json:"readiness"`
	Lifecycle     map[string]any `json:"lifecycle"`
	Metadata      map[string]any `json:"metadata"`
}

// HostStreamEnvelope is the daemon envelope delivered to a product host.
type HostStreamEnvelope struct {
	Request HostStreamEnvelopeRequest `json:"request"`
}

type HostStreamEnvelopeRequest struct {
	Fn     string `json:"fn"`
	Args   any    `json:"args"`
	CallID string `json:"call_id"`
	Caller string `json:"caller"`
}

// HostStreamRequest is the decoded host request projection.
type HostStreamRequest struct {
	Function string         `json:"function"`
	Args     any            `json:"args"`
	CallID   string         `json:"call_id"`
	Caller   string         `json:"caller"`
	Metadata map[string]any `json:"metadata"`
}

// HostStreamTerminalSummary is the terminal frame payload.
type HostStreamTerminalSummary struct {
	OutputHash string         `json:"output_hash"`
	Frames     int64          `json:"frames"`
	Metadata   map[string]any `json:"metadata,omitempty"`
}

// HostStreamFrame is one item/error/terminal host-stream frame.
type HostStreamFrame struct {
	FrameType  string                     `json:"frame_type"`
	Seq        *uint64                    `json:"seq"`
	Value      any                        `json:"value"`
	Error      *SDKError                  `json:"error"`
	Terminal   *HostStreamTerminalSummary `json:"terminal"`
	OutputHash *string                    `json:"output_hash"`
}

// HostStreamHashState is the output-hash folding state.
type HostStreamHashState struct {
	Algorithm     string  `json:"algorithm"`
	OutputHash    string  `json:"output_hash"`
	Frames        int64   `json:"frames"`
	LastSeq       *uint64 `json:"last_seq"`
	CanonicalJSON string  `json:"canonical_json,omitempty"`
}

// HostBindingTransport supplies host binding codec/hash operations.
type HostBindingTransport interface {
	BuildHostStreamBinding(ctx context.Context, requestJSON []byte) ([]byte, error)
	DecodeRequest(ctx context.Context, envelopeJSON []byte) ([]byte, error)
	EncodeItem(ctx context.Context, requestJSON []byte) ([]byte, error)
	EncodeError(ctx context.Context, requestJSON []byte) ([]byte, error)
	EncodeTerminal(ctx context.Context, requestJSON []byte) ([]byte, error)
	FoldOutputHash(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// HostBindingTransportFunc adapts functions into a HostBindingTransport.
type HostBindingTransportFunc struct {
	BuildHostStreamBindingFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	DecodeRequestFunc          func(ctx context.Context, envelopeJSON []byte) ([]byte, error)
	EncodeItemFunc             func(ctx context.Context, requestJSON []byte) ([]byte, error)
	EncodeErrorFunc            func(ctx context.Context, requestJSON []byte) ([]byte, error)
	EncodeTerminalFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
	FoldOutputHashFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f HostBindingTransportFunc) BuildHostStreamBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildHostStreamBindingFunc == nil {
		return nil, invalidRuntimeClient("host binding build transport function is required")
	}
	return f.BuildHostStreamBindingFunc(ctx, requestJSON)
}

func (f HostBindingTransportFunc) DecodeRequest(ctx context.Context, envelopeJSON []byte) ([]byte, error) {
	if f.DecodeRequestFunc == nil {
		return nil, invalidRuntimeClient("host binding decode transport function is required")
	}
	return f.DecodeRequestFunc(ctx, envelopeJSON)
}

func (f HostBindingTransportFunc) EncodeItem(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EncodeItemFunc == nil {
		return nil, invalidRuntimeClient("host binding encode item transport function is required")
	}
	return f.EncodeItemFunc(ctx, requestJSON)
}

func (f HostBindingTransportFunc) EncodeError(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EncodeErrorFunc == nil {
		return nil, invalidRuntimeClient("host binding encode error transport function is required")
	}
	return f.EncodeErrorFunc(ctx, requestJSON)
}

func (f HostBindingTransportFunc) EncodeTerminal(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EncodeTerminalFunc == nil {
		return nil, invalidRuntimeClient("host binding encode terminal transport function is required")
	}
	return f.EncodeTerminalFunc(ctx, requestJSON)
}

func (f HostBindingTransportFunc) FoldOutputHash(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.FoldOutputHashFunc == nil {
		return nil, invalidRuntimeClient("host binding hash transport function is required")
	}
	return f.FoldOutputHashFunc(ctx, requestJSON)
}

// HostBindingClient is the Host Binding profile facade.
type HostBindingClient struct {
	transport HostBindingTransport
}

func NewHostBindingClient(transport HostBindingTransport) (*HostBindingClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("host binding transport is required")
	}
	return &HostBindingClient{transport: transport}, nil
}

func (c *HostBindingClient) BuildHostStreamBinding(ctx context.Context, req HostStreamBindingRequest) (HostStreamBinding, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamBinding{}, err
	}
	if err := validateHostStreamBindingRequest(req); err != nil {
		return HostStreamBinding{}, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return HostStreamBinding{}, invalidRuntimePayload(fmt.Sprintf("encode host binding request: %v", err), err)
	}
	raw, err := c.transport.BuildHostStreamBinding(ctx, requestJSON)
	if err != nil {
		return HostStreamBinding{}, wrapHostBindingTransportError("host binding build failed", err)
	}
	return NewHostStreamBindingFromJSON(raw)
}

func (c *HostBindingClient) DecodeRequest(ctx context.Context, envelope HostStreamEnvelope) (HostStreamRequest, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamRequest{}, err
	}
	if envelope.Request.Fn == "" || envelope.Request.CallID == "" || envelope.Request.Caller == "" {
		return HostStreamRequest{}, invalidRuntimePayload("host stream envelope request is incomplete", nil)
	}
	requestJSON, err := json.Marshal(envelope)
	if err != nil {
		return HostStreamRequest{}, invalidRuntimePayload(fmt.Sprintf("encode host stream envelope: %v", err), err)
	}
	raw, err := c.transport.DecodeRequest(ctx, requestJSON)
	if err != nil {
		return HostStreamRequest{}, wrapHostBindingTransportError("host binding decode request failed", err)
	}
	return NewHostStreamRequestFromJSON(raw)
}

func (c *HostBindingClient) EncodeItem(ctx context.Context, seq uint64, value any) (HostStreamFrame, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamFrame{}, err
	}
	requestJSON, err := json.Marshal(map[string]any{"seq": seq, "value": value})
	if err != nil {
		return HostStreamFrame{}, invalidRuntimePayload(fmt.Sprintf("encode host stream item request: %v", err), err)
	}
	raw, err := c.transport.EncodeItem(ctx, requestJSON)
	if err != nil {
		return HostStreamFrame{}, wrapHostBindingTransportError("host binding encode item failed", err)
	}
	return NewHostStreamFrameFromJSON(raw)
}

func (c *HostBindingClient) EncodeError(ctx context.Context, errValue error) (HostStreamFrame, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamFrame{}, err
	}
	if errValue == nil {
		return HostStreamFrame{}, invalidRuntimePayload("error is required", nil)
	}
	requestJSON, err := json.Marshal(map[string]any{"error": hostBindingErrorDTO(errValue)})
	if err != nil {
		return HostStreamFrame{}, invalidRuntimePayload(fmt.Sprintf("encode host stream error request: %v", err), err)
	}
	raw, err := c.transport.EncodeError(ctx, requestJSON)
	if err != nil {
		return HostStreamFrame{}, wrapHostBindingTransportError("host binding encode error failed", err)
	}
	return NewHostStreamFrameFromJSON(raw)
}

func (c *HostBindingClient) EncodeTerminal(ctx context.Context, summary HostStreamTerminalSummary) (HostStreamFrame, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamFrame{}, err
	}
	if summary.OutputHash == "" || summary.Frames < 0 {
		return HostStreamFrame{}, invalidRuntimePayload("terminal output_hash and frames are required", nil)
	}
	requestJSON, err := json.Marshal(map[string]any{"summary": summary})
	if err != nil {
		return HostStreamFrame{}, invalidRuntimePayload(fmt.Sprintf("encode host stream terminal request: %v", err), err)
	}
	raw, err := c.transport.EncodeTerminal(ctx, requestJSON)
	if err != nil {
		return HostStreamFrame{}, wrapHostBindingTransportError("host binding encode terminal failed", err)
	}
	return NewHostStreamFrameFromJSON(raw)
}

func (c *HostBindingClient) FoldOutputHash(ctx context.Context, state HostStreamHashState, seq uint64, value any) (HostStreamHashState, error) {
	if err := c.requireReady(ctx); err != nil {
		return HostStreamHashState{}, err
	}
	if err := validateHostStreamHashFold(state, seq); err != nil {
		return HostStreamHashState{}, err
	}
	requestJSON, err := json.Marshal(map[string]any{"state": state, "seq": seq, "value": value})
	if err != nil {
		return HostStreamHashState{}, invalidRuntimePayload(fmt.Sprintf("encode host stream hash request: %v", err), err)
	}
	raw, err := c.transport.FoldOutputHash(ctx, requestJSON)
	if err != nil {
		return HostStreamHashState{}, wrapHostBindingTransportError("host binding hash fold failed", err)
	}
	return NewHostStreamHashStateFromJSON(raw)
}

func (c *HostBindingClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("host binding client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewHostStreamBindingFromJSON(raw []byte) (HostStreamBinding, error) {
	var binding HostStreamBinding
	if err := json.Unmarshal(raw, &binding); err != nil {
		return HostStreamBinding{}, invalidRuntimePayload(fmt.Sprintf("decode host binding JSON: %v", err), err)
	}
	if binding.BindingID == "" || binding.DescriptorRef == "" || binding.Endpoint == "" ||
		binding.FrameSchema != hostStreamFrameSchema || binding.Cleanup == nil ||
		binding.Readiness == nil || binding.Lifecycle == nil || binding.Metadata == nil {
		return HostStreamBinding{}, invalidRuntimePayload("invalid host stream binding projection", nil)
	}
	if !isAbsoluteHostEndpoint(binding.Endpoint) {
		return HostStreamBinding{}, invalidRuntimePayload("host stream endpoint must be absolute", nil)
	}
	return binding, nil
}

func NewHostStreamRequestFromJSON(raw []byte) (HostStreamRequest, error) {
	var request HostStreamRequest
	if err := json.Unmarshal(raw, &request); err != nil {
		return HostStreamRequest{}, invalidRuntimePayload(fmt.Sprintf("decode host stream request JSON: %v", err), err)
	}
	if request.Function == "" || request.CallID == "" || request.Caller == "" || request.Metadata == nil {
		return HostStreamRequest{}, invalidRuntimePayload("invalid host stream request projection", nil)
	}
	return request, nil
}

func NewHostStreamFrameFromJSON(raw []byte) (HostStreamFrame, error) {
	var dto struct {
		FrameType  string                     `json:"frame_type"`
		Seq        *uint64                    `json:"seq"`
		Value      any                        `json:"value"`
		Error      json.RawMessage            `json:"error"`
		Terminal   *HostStreamTerminalSummary `json:"terminal"`
		OutputHash *string                    `json:"output_hash"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return HostStreamFrame{}, invalidRuntimePayload(fmt.Sprintf("decode host stream frame JSON: %v", err), err)
	}
	var sdkErr *SDKError
	if len(dto.Error) > 0 && string(dto.Error) != "null" {
		decoded, err := DecodeDaemonErrorJSON(dto.Error)
		if err != nil {
			return HostStreamFrame{}, err
		}
		sdkErr = decoded
	}
	frame := HostStreamFrame{
		FrameType:  dto.FrameType,
		Seq:        dto.Seq,
		Value:      dto.Value,
		Error:      sdkErr,
		Terminal:   dto.Terminal,
		OutputHash: dto.OutputHash,
	}
	if err := validateHostStreamFrame(frame); err != nil {
		return HostStreamFrame{}, err
	}
	return frame, nil
}

func NewHostStreamHashStateFromJSON(raw []byte) (HostStreamHashState, error) {
	var state HostStreamHashState
	if err := json.Unmarshal(raw, &state); err != nil {
		return HostStreamHashState{}, invalidRuntimePayload(fmt.Sprintf("decode host stream hash state JSON: %v", err), err)
	}
	if state.Algorithm != hostStreamHashAlgorithm || state.OutputHash == "" || state.Frames < 0 {
		return HostStreamHashState{}, invalidRuntimePayload("invalid host stream hash state projection", nil)
	}
	return state, nil
}

func validateHostStreamBindingRequest(req HostStreamBindingRequest) error {
	if req.BindingID == "" || req.DescriptorRef == "" || req.Endpoint == "" {
		return invalidRuntimePayload("binding_id, descriptor_ref, and endpoint are required", nil)
	}
	if req.FrameSchema != hostStreamFrameSchema {
		return invalidRuntimePayload("frame_schema must be host-stream-frame.schema.json", nil)
	}
	if !isAbsoluteHostEndpoint(req.Endpoint) {
		return invalidRuntimePayload("host stream endpoint must be absolute", nil)
	}
	return nil
}

func isAbsoluteHostEndpoint(endpoint string) bool {
	return strings.HasPrefix(endpoint, "/") || strings.HasPrefix(endpoint, "unix:///")
}

func validateHostStreamFrame(frame HostStreamFrame) error {
	switch frame.FrameType {
	case "item":
		if frame.Seq == nil || frame.Error != nil || frame.Terminal != nil || frame.OutputHash != nil {
			return invalidRuntimePayload("invalid item host stream frame", nil)
		}
	case "error":
		if frame.Seq != nil || frame.Value != nil || frame.Error == nil || frame.Terminal != nil || frame.OutputHash != nil {
			return invalidRuntimePayload("invalid error host stream frame", nil)
		}
	case "terminal":
		if frame.Seq == nil || frame.Value != nil || frame.Error != nil || frame.Terminal == nil || frame.OutputHash == nil {
			return invalidRuntimePayload("invalid terminal host stream frame", nil)
		}
		if frame.Terminal.OutputHash == "" || frame.Terminal.Frames < 0 || *frame.OutputHash != frame.Terminal.OutputHash {
			return invalidRuntimePayload("invalid terminal host stream summary", nil)
		}
	default:
		return invalidRuntimePayload("unknown host stream frame type", nil)
	}
	return nil
}

func validateHostStreamHashFold(state HostStreamHashState, seq uint64) error {
	if state.Algorithm != hostStreamHashAlgorithm || state.OutputHash == "" || state.Frames < 0 {
		return invalidRuntimePayload("valid hash state is required", nil)
	}
	if state.LastSeq == nil {
		if state.Frames != 0 || seq != 0 {
			return invalidRuntimePayload("host stream hash sequence gap", nil)
		}
		return nil
	}
	if seq != *state.LastSeq+1 {
		return invalidRuntimePayload("host stream hash sequence gap", nil)
	}
	return nil
}

func hostBindingErrorDTO(errValue error) map[string]any {
	var sdkErr *SDKError
	if errors.As(errValue, &sdkErr) {
		code := sdkErr.Code
		if code == "" {
			code = ErrGeneric
		}
		stage := sdkErr.Stage
		if stage == "" {
			stage = "host_binding"
		}
		retry := sdkErr.Retry
		if retry == "" {
			retry = RetryNever
		}
		details := sdkErr.Details
		if details == nil {
			details = map[string]any{}
		}
		return map[string]any{
			"code":          string(code),
			"stage":         stage,
			"message":       sdkErr.Message,
			"retry":         string(retry),
			"source":        emptyStringAsNil(sdkErr.Source),
			"invocation_id": emptyStringAsNil(sdkErr.InvocationID),
			"receipt_ura":   emptyStringAsNil(sdkErr.ReceiptURA),
			"details":       details,
		}
	}
	return map[string]any{
		"code":    string(ErrGeneric),
		"stage":   "host_binding",
		"message": errValue.Error(),
		"retry":   string(RetryNever),
		"details": map[string]any{},
	}
}

func emptyStringAsNil(value string) any {
	if value == "" {
		return nil
	}
	return value
}

func wrapHostBindingTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
