package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// ReceiptFetchRequest preserves the complete carrier context for receipt fetch.
type ReceiptFetchRequest struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	InvocationURA     string         `json:"invocation_ura,omitempty"`
	RequestID         string         `json:"request_id,omitempty"`
	TraceID           string         `json:"trace_id,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// ReceiptSummary is the sdk/schemas/receipt.schema.json projection.
type ReceiptSummary struct {
	ReceiptURA   *string        `json:"receipt_ura"`
	InvocationID *string        `json:"invocation_id"`
	State        string         `json:"state"`
	Verified     bool           `json:"verified"`
	Output       any            `json:"output"`
	Error        *SDKError      `json:"error"`
	CausalRef    *string        `json:"causal_ref"`
	Metadata     map[string]any `json:"metadata"`
}

// ReceiptVerification is a daemon/Axon verification projection.
type ReceiptVerification struct {
	Verified     bool           `json:"verified"`
	ReceiptURA   *string        `json:"receipt_ura"`
	InvocationID *string        `json:"invocation_id"`
	Method       string         `json:"method"`
	Reason       string         `json:"reason,omitempty"`
	Metadata     map[string]any `json:"metadata"`
}

// CausalRef is a daemon/Axon-returned causal reference for child invocations.
type CausalRef struct {
	CausalRef    string         `json:"causal_ref"`
	ReceiptURA   *string        `json:"receipt_ura"`
	InvocationID *string        `json:"invocation_id"`
	Form         string         `json:"form,omitempty"`
	Metadata     map[string]any `json:"metadata"`
}

// ReceiptTransport supplies receipt operations behind the SDK facade.
type ReceiptTransport interface {
	Fetch(ctx context.Context, requestJSON []byte) ([]byte, error)
	Project(ctx context.Context, receiptJSON []byte) ([]byte, error)
	Verify(ctx context.Context, receiptJSON []byte) ([]byte, error)
	CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error)
}

// ReceiptTransportFunc adapts functions into a ReceiptTransport.
type ReceiptTransportFunc struct {
	FetchFunc     func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectFunc   func(ctx context.Context, receiptJSON []byte) ([]byte, error)
	VerifyFunc    func(ctx context.Context, receiptJSON []byte) ([]byte, error)
	CausalRefFunc func(ctx context.Context, receiptJSON []byte) ([]byte, error)
}

func (f ReceiptTransportFunc) Fetch(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.FetchFunc == nil {
		return nil, invalidRuntimeClient("receipt fetch transport function is required")
	}
	return f.FetchFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) Project(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.ProjectFunc == nil {
		return nil, invalidRuntimeClient("receipt project transport function is required")
	}
	return f.ProjectFunc(ctx, receiptJSON)
}

func (f ReceiptTransportFunc) Verify(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.VerifyFunc == nil {
		return nil, invalidRuntimeClient("receipt verify transport function is required")
	}
	return f.VerifyFunc(ctx, receiptJSON)
}

func (f ReceiptTransportFunc) CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.CausalRefFunc == nil {
		return nil, invalidRuntimeClient("receipt causal-ref transport function is required")
	}
	return f.CausalRefFunc(ctx, receiptJSON)
}

// ReceiptClient is the Receipt profile facade.
type ReceiptClient struct {
	transport ReceiptTransport
}

func NewReceiptClient(transport ReceiptTransport) (*ReceiptClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("receipt transport is required")
	}
	return &ReceiptClient{transport: transport}, nil
}

func (c *ReceiptClient) Fetch(ctx context.Context, req ReceiptFetchRequest) (ReceiptSummary, error) {
	if err := c.requireReady(ctx); err != nil {
		return ReceiptSummary{}, err
	}
	if err := validateReceiptFetchRequest(req); err != nil {
		return ReceiptSummary{}, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return ReceiptSummary{}, invalidRuntimePayload(fmt.Sprintf("encode receipt fetch request: %v", err), err)
	}
	raw, err := c.transport.Fetch(ctx, requestJSON)
	if err != nil {
		return ReceiptSummary{}, wrapReceiptTransportError("receipt fetch failed", err)
	}
	return NewReceiptSummaryFromJSON(raw)
}

func (c *ReceiptClient) Project(ctx context.Context, receiptJSON []byte) (ReceiptSummary, error) {
	if err := c.requireReady(ctx); err != nil {
		return ReceiptSummary{}, err
	}
	if len(receiptJSON) == 0 {
		return ReceiptSummary{}, invalidRuntimePayload("receipt JSON is required", nil)
	}
	raw, err := c.transport.Project(ctx, receiptJSON)
	if err != nil {
		return ReceiptSummary{}, wrapReceiptTransportError("receipt project failed", err)
	}
	return NewReceiptSummaryFromJSON(raw)
}

func (c *ReceiptClient) Verify(ctx context.Context, receiptJSON []byte) (ReceiptVerification, error) {
	if err := c.requireReady(ctx); err != nil {
		return ReceiptVerification{}, err
	}
	if len(receiptJSON) == 0 {
		return ReceiptVerification{}, invalidRuntimePayload("receipt JSON is required", nil)
	}
	raw, err := c.transport.Verify(ctx, receiptJSON)
	if err != nil {
		return ReceiptVerification{}, wrapReceiptTransportError("receipt verify failed", err)
	}
	return NewReceiptVerificationFromJSON(raw)
}

func (c *ReceiptClient) CausalRef(ctx context.Context, receiptJSON []byte) (CausalRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return CausalRef{}, err
	}
	if len(receiptJSON) == 0 {
		return CausalRef{}, invalidRuntimePayload("receipt JSON is required", nil)
	}
	raw, err := c.transport.CausalRef(ctx, receiptJSON)
	if err != nil {
		return CausalRef{}, wrapReceiptTransportError("receipt causal-ref failed", err)
	}
	return NewCausalRefFromJSON(raw)
}

func (c *ReceiptClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("receipt client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewReceiptSummaryFromJSON(raw []byte) (ReceiptSummary, error) {
	var dto struct {
		ReceiptURA   *string         `json:"receipt_ura"`
		InvocationID *string         `json:"invocation_id"`
		State        string          `json:"state"`
		Verified     bool            `json:"verified"`
		Output       json.RawMessage `json:"output"`
		Error        json.RawMessage `json:"error"`
		CausalRef    *string         `json:"causal_ref"`
		Metadata     map[string]any  `json:"metadata"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return ReceiptSummary{}, invalidRuntimePayload(fmt.Sprintf("decode receipt summary JSON: %v", err), err)
	}
	if dto.State == "" || dto.Output == nil {
		return ReceiptSummary{}, invalidRuntimePayload("invalid receipt summary", nil)
	}
	var output any
	if err := json.Unmarshal(dto.Output, &output); err != nil {
		return ReceiptSummary{}, invalidRuntimePayload(fmt.Sprintf("decode receipt output JSON: %v", err), err)
	}
	var sdkErr *SDKError
	if len(dto.Error) > 0 && string(dto.Error) != "null" {
		decoded, err := DecodeDaemonErrorJSON(dto.Error)
		if err != nil {
			return ReceiptSummary{}, err
		}
		sdkErr = decoded
	}
	if dto.Metadata == nil {
		dto.Metadata = map[string]any{}
	}
	return ReceiptSummary{
		ReceiptURA:   dto.ReceiptURA,
		InvocationID: dto.InvocationID,
		State:        dto.State,
		Verified:     dto.Verified,
		Output:       output,
		Error:        sdkErr,
		CausalRef:    dto.CausalRef,
		Metadata:     dto.Metadata,
	}, nil
}

func NewReceiptVerificationFromJSON(raw []byte) (ReceiptVerification, error) {
	var result ReceiptVerification
	if err := json.Unmarshal(raw, &result); err != nil {
		return ReceiptVerification{}, invalidRuntimePayload(fmt.Sprintf("decode receipt verification JSON: %v", err), err)
	}
	if result.Method == "" {
		return ReceiptVerification{}, invalidRuntimePayload("verification method is required", nil)
	}
	if result.Metadata == nil {
		result.Metadata = map[string]any{}
	}
	return result, nil
}

func NewCausalRefFromJSON(raw []byte) (CausalRef, error) {
	var ref CausalRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return CausalRef{}, invalidRuntimePayload(fmt.Sprintf("decode causal ref JSON: %v", err), err)
	}
	if ref.CausalRef == "" {
		return CausalRef{}, invalidRuntimePayload("causal_ref is required", nil)
	}
	if ref.Metadata == nil {
		ref.Metadata = map[string]any{}
	}
	return ref, nil
}

func validateReceiptFetchRequest(req ReceiptFetchRequest) error {
	if req.CallerURA == "" || req.CalleeURA == "" || req.SubjectURA == "" || req.DescriptorVersion == "" || req.NonceBase64 == "" {
		return invalidRuntimePayload("caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required", nil)
	}
	if req.CausalContext == nil {
		return invalidRuntimePayload("causal_context is required", nil)
	}
	keys := 0
	if req.InvocationURA != "" {
		keys++
	}
	if req.RequestID != "" {
		keys++
	}
	if req.TraceID != "" {
		keys++
	}
	if keys != 1 {
		return invalidRuntimePayload("exactly one receipt lookup key is required", nil)
	}
	return nil
}

func wrapReceiptTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
