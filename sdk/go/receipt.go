package easynet

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const receiptProfile = "receipt"
const receiptFetchAbility = "invocation.history.get"

const (
	receiptHistoryListAbility = "invocation.history.list"
	receiptHistoryGetAbility  = "invocation.history.get"
	receiptTraceGetAbility    = "invocation.trace.get"
)

// ReceiptCarrierBase is the complete carrier context shared by daemon receipt
// and invocation-ledger read-model operations.
type ReceiptCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	TimeoutMS         int            `json:"timeout_ms,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// ReceiptHistoryReadRequest preserves the daemon-owned invocation-ledger query
// arguments while keeping Invocation carrier fields out of backend handlers.
type ReceiptHistoryReadRequest struct {
	ReceiptCarrierBase
	Arguments map[string]any `json:"arguments,omitempty"`
}

// ReceiptFetchRequest preserves the complete carrier context for receipt fetch.
type ReceiptFetchRequest struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	DescriptorRef     string         `json:"descriptor_ref"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	InvocationURA     string         `json:"invocation_ura,omitempty"`
	RequestID         string         `json:"request_id,omitempty"`
	TraceID           string         `json:"trace_id,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// ReceiptChainVerificationRequest preserves ordered receipt bodies for daemon/Axon continuity checks.
type ReceiptChainVerificationRequest struct {
	Receipts []json.RawMessage `json:"receipts"`
	Metadata map[string]any    `json:"metadata,omitempty"`
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

// ReceiptChainItemVerification describes one daemon-projected chain edge.
type ReceiptChainItemVerification struct {
	Index              int            `json:"index"`
	ReceiptURA         string         `json:"receipt_ura"`
	InvocationID       *string        `json:"invocation_id"`
	ReceiptHashHex     string         `json:"receipt_hash_hex"`
	PrevReceiptHashHex *string        `json:"prev_receipt_hash_hex"`
	Continuous         bool           `json:"continuous"`
	Reason             string         `json:"reason,omitempty"`
	Metadata           map[string]any `json:"metadata"`
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

// ReceiptChainVerification is a daemon/Axon receipt-chain continuity projection.
type ReceiptChainVerification struct {
	Verified            bool                           `json:"verified"`
	Continuous          bool                           `json:"continuous"`
	Method              string                         `json:"method"`
	Reason              string                         `json:"reason,omitempty"`
	RequiresFullReceipt bool                           `json:"requires_full_receipt"`
	RootReceiptURA      *string                        `json:"root_receipt_ura"`
	TerminalReceiptURA  *string                        `json:"terminal_receipt_ura"`
	ReceiptCount        int                            `json:"receipt_count"`
	Items               []ReceiptChainItemVerification `json:"items"`
	Metadata            map[string]any                 `json:"metadata"`
}

// CausalRef is a daemon/Axon-returned causal reference for child invocations.
type CausalRef struct {
	CausalRef      string         `json:"causal_ref"`
	ReceiptURA     *string        `json:"receipt_ura"`
	ReceiptHashHex string         `json:"receipt_hash_hex"`
	CausalContext  map[string]any `json:"causal_context"`
	InvocationID   *string        `json:"invocation_id"`
	Verified       bool           `json:"verified"`
	Form           string         `json:"form,omitempty"`
	Metadata       map[string]any `json:"metadata"`
}

// ToCausalContext returns the child-Invocation causal_context projection.
func (r CausalRef) ToCausalContext() map[string]any {
	return copyMap(r.CausalContext)
}

// ReceiptTransport supplies receipt operations behind the SDK facade.
type ReceiptTransport interface {
	Fetch(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildListHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildGetHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTraceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListHistory(ctx context.Context, requestJSON []byte) ([]byte, error)
	GetHistory(ctx context.Context, requestJSON []byte) ([]byte, error)
	GetTrace(ctx context.Context, requestJSON []byte) ([]byte, error)
	Project(ctx context.Context, receiptJSON []byte) ([]byte, error)
	Verify(ctx context.Context, receiptJSON []byte) ([]byte, error)
	VerifyChain(ctx context.Context, requestJSON []byte) ([]byte, error)
	CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error)
}

// ReceiptTransportFunc adapts functions into a ReceiptTransport.
type ReceiptTransportFunc struct {
	FetchFunc                      func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildListHistoryInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildGetHistoryInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTraceInvocationFunc       func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListHistoryFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	GetHistoryFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	GetTraceFunc                   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectFunc                    func(ctx context.Context, receiptJSON []byte) ([]byte, error)
	VerifyFunc                     func(ctx context.Context, receiptJSON []byte) ([]byte, error)
	VerifyChainFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CausalRefFunc                  func(ctx context.Context, receiptJSON []byte) ([]byte, error)
}

func (f ReceiptTransportFunc) Fetch(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.FetchFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt fetch transport function is required")
	}
	return f.FetchFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) BuildListHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildListHistoryInvocationFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt list-history invocation transport function is required")
	}
	return f.BuildListHistoryInvocationFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) BuildGetHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildGetHistoryInvocationFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt get-history invocation transport function is required")
	}
	return f.BuildGetHistoryInvocationFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) BuildTraceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildTraceInvocationFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt trace invocation transport function is required")
	}
	return f.BuildTraceInvocationFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) ListHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListHistoryFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt list-history transport function is required")
	}
	return f.ListHistoryFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) GetHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.GetHistoryFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt get-history transport function is required")
	}
	return f.GetHistoryFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) GetTrace(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.GetTraceFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt trace transport function is required")
	}
	return f.GetTraceFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) Project(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.ProjectFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt project transport function is required")
	}
	return f.ProjectFunc(ctx, receiptJSON)
}

func (f ReceiptTransportFunc) Verify(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.VerifyFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt verify transport function is required")
	}
	return f.VerifyFunc(ctx, receiptJSON)
}

func (f ReceiptTransportFunc) VerifyChain(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.VerifyChainFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt verify-chain transport function is required")
	}
	return f.VerifyChainFunc(ctx, requestJSON)
}

func (f ReceiptTransportFunc) CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	if f.CausalRefFunc == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt causal-ref transport function is required")
	}
	return f.CausalRefFunc(ctx, receiptJSON)
}

// ReceiptClient is the Receipt profile facade.
type ReceiptClient struct {
	lifecycle profileClientLifecycle
	transport ReceiptTransport
}

func NewReceiptClient(transport ReceiptTransport) (*ReceiptClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(receiptProfile, "receipt transport is required")
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
		return ReceiptSummary{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("encode receipt fetch request: %v", err), err)
	}
	raw, err := c.transport.Fetch(ctx, requestJSON)
	if err != nil {
		return ReceiptSummary{}, wrapReceiptTransportError("receipt fetch failed", err)
	}
	return NewReceiptSummaryFromJSON(raw)
}

// BuildFetchInvocation projects a receipt fetch request into the daemon-owned
// invocation.history.get carrier without opening the receipt ledger.
func (c *ReceiptClient) BuildFetchInvocation(ctx context.Context, req ReceiptFetchRequest) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	return BuildReceiptFetchInvocation(req)
}

func (c *ReceiptClient) BuildListHistoryInvocation(ctx context.Context, req ReceiptHistoryReadRequest) (InvocationDraft, error) {
	return c.buildHistoryInvocation(ctx, req, c.transport.BuildListHistoryInvocation, "receipt list-history invocation failed")
}

func (c *ReceiptClient) BuildGetHistoryInvocation(ctx context.Context, req ReceiptHistoryReadRequest) (InvocationDraft, error) {
	return c.buildHistoryInvocation(ctx, req, c.transport.BuildGetHistoryInvocation, "receipt get-history invocation failed")
}

func (c *ReceiptClient) BuildTraceInvocation(ctx context.Context, req ReceiptHistoryReadRequest) (InvocationDraft, error) {
	return c.buildHistoryInvocation(ctx, req, c.transport.BuildTraceInvocation, "receipt trace invocation failed")
}

func (c *ReceiptClient) ListHistory(ctx context.Context, req ReceiptHistoryReadRequest) (map[string]any, error) {
	return c.readHistory(ctx, req, c.transport.ListHistory, "receipt list-history failed")
}

func (c *ReceiptClient) GetHistory(ctx context.Context, req ReceiptHistoryReadRequest) (map[string]any, error) {
	return c.readHistory(ctx, req, c.transport.GetHistory, "receipt get-history failed")
}

func (c *ReceiptClient) GetTrace(ctx context.Context, req ReceiptHistoryReadRequest) (map[string]any, error) {
	return c.readHistory(ctx, req, c.transport.GetTrace, "receipt trace failed")
}

func (c *ReceiptClient) buildHistoryInvocation(
	ctx context.Context,
	req ReceiptHistoryReadRequest,
	build func(context.Context, []byte) ([]byte, error),
	message string,
) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalReceiptHistoryReadRequest(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := build(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapReceiptTransportError(message, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *ReceiptClient) readHistory(
	ctx context.Context,
	req ReceiptHistoryReadRequest,
	read func(context.Context, []byte) ([]byte, error),
	message string,
) (map[string]any, error) {
	if err := c.requireReady(ctx); err != nil {
		return nil, err
	}
	requestJSON, err := marshalReceiptHistoryReadRequest(req)
	if err != nil {
		return nil, err
	}
	raw, err := read(ctx, requestJSON)
	if err != nil {
		return nil, wrapReceiptTransportError(message, err)
	}
	return NewReceiptReadModelFromJSON(raw)
}

// BuildReceiptFetchInvocation projects a receipt fetch request into a complete
// Runtime Core InvocationDraft carrier.
func BuildReceiptFetchInvocation(req ReceiptFetchRequest) (InvocationDraft, error) {
	if err := validateReceiptFetchRequest(req); err != nil {
		return InvocationDraft{}, err
	}
	args, err := receiptFetchArgs(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	metadata := copyMap(req.Metadata)
	if metadata == nil {
		metadata = map[string]any{}
	}
	metadata["profile"] = receiptProfile
	metadata["system_ability"] = receiptFetchAbility
	metadata["carrier_owner"] = "daemon_sdk"

	return NewInvocationBuilder().
		WithCallerURA(req.CallerURA).
		WithCalleeURA(req.CalleeURA).
		WithDescriptorRef(req.DescriptorRef).
		WithSubjectURA(req.SubjectURA).
		WithNonceBase64(req.NonceBase64).
		WithCausalContext(req.CausalContext).
		WithJSONArgs(args).
		WithContentType("application/json").
		WithMetadata(metadata).
		Build()
}

func (c *ReceiptClient) Project(ctx context.Context, receiptJSON []byte) (ReceiptSummary, error) {
	if err := c.requireReady(ctx); err != nil {
		return ReceiptSummary{}, err
	}
	if len(receiptJSON) == 0 {
		return ReceiptSummary{}, invalidProfilePayload(receiptProfile, "receipt JSON is required", nil)
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
		return ReceiptVerification{}, invalidProfilePayload(receiptProfile, "receipt JSON is required", nil)
	}
	raw, err := c.transport.Verify(ctx, receiptJSON)
	if err != nil {
		return ReceiptVerification{}, wrapReceiptTransportError("receipt verify failed", err)
	}
	return NewReceiptVerificationFromJSON(raw)
}

func (c *ReceiptClient) VerifyChain(ctx context.Context, req ReceiptChainVerificationRequest) (ReceiptChainVerification, error) {
	if err := c.requireReady(ctx); err != nil {
		return ReceiptChainVerification{}, err
	}
	if err := validateReceiptChainVerificationRequest(req); err != nil {
		return ReceiptChainVerification{}, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("encode receipt chain verification request: %v", err), err)
	}
	raw, err := c.transport.VerifyChain(ctx, requestJSON)
	if err != nil {
		return ReceiptChainVerification{}, wrapReceiptTransportError("receipt verify-chain failed", err)
	}
	return NewReceiptChainVerificationFromJSON(raw)
}

func (c *ReceiptClient) CausalRef(ctx context.Context, receiptJSON []byte) (CausalRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return CausalRef{}, err
	}
	if len(receiptJSON) == 0 {
		return CausalRef{}, invalidProfilePayload(receiptProfile, "receipt JSON is required", nil)
	}
	raw, err := c.transport.CausalRef(ctx, receiptJSON)
	if err != nil {
		return CausalRef{}, wrapReceiptTransportError("receipt causal-ref failed", err)
	}
	return NewCausalRefFromJSON(raw)
}

func (c *ReceiptClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(receiptProfile, "receipt client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "receipt")
}

func (c *ReceiptClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(receiptProfile, "receipt client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "receipt")
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
		return ReceiptSummary{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt summary JSON: %v", err), err)
	}
	if dto.State == "" || dto.Output == nil {
		return ReceiptSummary{}, invalidProfilePayload(receiptProfile, "invalid receipt summary", nil)
	}
	var output any
	if err := json.Unmarshal(dto.Output, &output); err != nil {
		return ReceiptSummary{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt output JSON: %v", err), err)
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

func NewReceiptReadModelFromJSON(raw []byte) (map[string]any, error) {
	if len(raw) == 0 {
		return nil, invalidProfilePayload(receiptProfile, "receipt read-model JSON is required", nil)
	}
	var result map[string]any
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt read-model JSON: %v", err), err)
	}
	if result == nil {
		return nil, invalidProfilePayload(receiptProfile, "receipt read-model must be an object", nil)
	}
	return result, nil
}

func NewReceiptVerificationFromJSON(raw []byte) (ReceiptVerification, error) {
	var result ReceiptVerification
	if err := json.Unmarshal(raw, &result); err != nil {
		return ReceiptVerification{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt verification JSON: %v", err), err)
	}
	if result.Method == "" {
		return ReceiptVerification{}, invalidProfilePayload(receiptProfile, "verification method is required", nil)
	}
	if result.Metadata == nil {
		result.Metadata = map[string]any{}
	}
	return result, nil
}

func NewReceiptChainVerificationFromJSON(raw []byte) (ReceiptChainVerification, error) {
	var result ReceiptChainVerification
	if err := json.Unmarshal(raw, &result); err != nil {
		return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt chain verification JSON: %v", err), err)
	}
	if result.Method == "" {
		return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, "chain verification method is required", nil)
	}
	if result.ReceiptCount <= 0 || len(result.Items) == 0 {
		return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, "chain verification items are required", nil)
	}
	if result.ReceiptCount != len(result.Items) {
		return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, "receipt_count must match items length", nil)
	}
	for index := range result.Items {
		item := &result.Items[index]
		if item.Index != index {
			return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, "chain item index must match position", nil)
		}
		if item.ReceiptURA == "" || item.ReceiptHashHex == "" {
			return ReceiptChainVerification{}, invalidProfilePayload(receiptProfile, "chain item receipt_ura and receipt_hash_hex are required", nil)
		}
		if item.Metadata == nil {
			item.Metadata = map[string]any{}
		}
	}
	if result.Metadata == nil {
		result.Metadata = map[string]any{}
	}
	return result, nil
}

func NewCausalRefFromJSON(raw []byte) (CausalRef, error) {
	var ref CausalRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return CausalRef{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode causal ref JSON: %v", err), err)
	}
	var obj map[string]any
	if err := json.Unmarshal(raw, &obj); err != nil {
		return CausalRef{}, invalidProfilePayload(receiptProfile, fmt.Sprintf("decode causal ref JSON: %v", err), err)
	}
	causalContext, err := causalContextFromReceiptProjection(obj)
	if err != nil {
		return CausalRef{}, err
	}
	receiptURA, _ := receiptStringField(causalContext, "receipt_ura")
	receiptHash, _ := receiptHashField(causalContext)
	form, _ := receiptStringField(causalContext, "form")
	if form == "" {
		form = "scalar"
	}
	if ref.Metadata == nil {
		ref.Metadata = map[string]any{}
	}
	ref.ReceiptURA = &receiptURA
	ref.ReceiptHashHex = receiptHash
	ref.CausalContext = causalContext
	ref.Form = form
	return ref, nil
}

func validateReceiptFetchRequest(req ReceiptFetchRequest) error {
	if req.CallerURA == "" || req.CalleeURA == "" || req.DescriptorRef == "" || req.SubjectURA == "" || req.DescriptorVersion == "" || req.NonceBase64 == "" {
		return invalidProfilePayload(receiptProfile, "caller_ura, callee_ura, descriptor_ref, subject_ura, descriptor_version, and nonce_base64 are required", nil)
	}
	if req.CausalContext == nil {
		return invalidProfilePayload(receiptProfile, "causal_context is required", nil)
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
		return invalidProfilePayload(receiptProfile, "exactly one receipt lookup key is required", nil)
	}
	return nil
}

func validateReceiptCarrierBase(base ReceiptCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" || base.DescriptorVersion == "" || base.NonceBase64 == "" {
		return invalidProfilePayload(receiptProfile, "caller_ura, callee_ura, subject_ura, descriptor_version, and nonce_base64 are required", nil)
	}
	if base.CausalContext == nil {
		return invalidProfilePayload(receiptProfile, "causal_context is required", nil)
	}
	return nil
}

func marshalReceiptHistoryReadRequest(req ReceiptHistoryReadRequest) ([]byte, error) {
	if err := validateReceiptCarrierBase(req.ReceiptCarrierBase); err != nil {
		return nil, err
	}
	if req.Arguments == nil {
		req.Arguments = map[string]any{}
	}
	raw, err := json.Marshal(req)
	if err != nil {
		return nil, invalidProfilePayload(receiptProfile, fmt.Sprintf("encode receipt history request: %v", err), err)
	}
	return raw, nil
}

func receiptFetchArgs(req ReceiptFetchRequest) (map[string]any, error) {
	key := map[string]any{}
	switch {
	case req.InvocationURA != "":
		key["invocation_ura"] = req.InvocationURA
	case req.RequestID != "":
		key["request_id"] = req.RequestID
	case req.TraceID != "":
		key["trace_id"] = req.TraceID
	default:
		return nil, invalidProfilePayload(receiptProfile, "exactly one receipt lookup key is required", nil)
	}
	return map[string]any{"key": key}, nil
}

func validateReceiptChainVerificationRequest(req ReceiptChainVerificationRequest) error {
	if len(req.Receipts) == 0 {
		return invalidProfilePayload(receiptProfile, "at least one receipt is required", nil)
	}
	seenURAs := map[string]struct{}{}
	seenHashes := map[string]struct{}{}
	for index, raw := range req.Receipts {
		if len(raw) == 0 {
			return invalidProfilePayload(receiptProfile, fmt.Sprintf("receipt[%d] JSON is required", index), nil)
		}
		var obj map[string]any
		if err := json.Unmarshal(raw, &obj); err != nil {
			return invalidProfilePayload(receiptProfile, fmt.Sprintf("decode receipt[%d] JSON: %v", index, err), err)
		}
		if ura, ok := receiptStringField(obj, "receipt_ura"); ok {
			if _, exists := seenURAs[ura]; exists {
				return invalidProfilePayload(receiptProfile, "duplicate receipt_ura in chain request", nil)
			}
			seenURAs[ura] = struct{}{}
		}
		if hash, ok := receiptHashField(obj); ok {
			if _, err := hex.DecodeString(hash); err != nil || len(hash) != 64 {
				return invalidProfilePayload(receiptProfile, "receipt hash must decode to exactly 32 bytes", err)
			}
			if _, exists := seenHashes[hash]; exists {
				return invalidProfilePayload(receiptProfile, "duplicate receipt hash in chain request", nil)
			}
			seenHashes[hash] = struct{}{}
		}
	}
	return nil
}

func causalContextFromReceiptProjection(obj map[string]any) (map[string]any, error) {
	context := map[string]any{}
	hasContext := false
	if rawContext, ok := obj["causal_context"]; ok && rawContext != nil {
		hasContext = true
		decoded, ok := rawContext.(map[string]any)
		if !ok {
			return nil, invalidProfilePayload(receiptProfile, "causal_context must be an object", nil)
		}
		for key, value := range decoded {
			context[key] = value
		}
		if form, ok := receiptStringField(context, "form"); !ok || form == "" {
			return nil, invalidProfilePayload(receiptProfile, "causal_context.form is required", nil)
		}
	} else {
		receiptURA, ok := receiptStringField(obj, "receipt_ura")
		if !ok {
			return nil, invalidProfilePayload(receiptProfile, "receipt_ura is required", nil)
		}
		form, _ := receiptStringField(obj, "form")
		if form == "" {
			form = "scalar"
		}
		context["form"] = form
		context["receipt_ura"] = receiptURA
	}
	receiptURA, ok := receiptStringField(context, "receipt_ura")
	if !ok {
		return nil, invalidProfilePayload(receiptProfile, "receipt_ura is required", nil)
	}
	receiptHash, ok := receiptHashField(context)
	if !ok && !hasContext {
		receiptHash, ok = receiptHashField(obj)
		if ok {
			context["receipt_hash_hex"] = receiptHash
		}
	}
	if !ok {
		return nil, invalidProfilePayload(receiptProfile, "receipt_hash_hex is required", nil)
	}
	if _, err := hex.DecodeString(receiptHash); err != nil || len(receiptHash) != 64 {
		return nil, invalidProfilePayload(receiptProfile, "receipt hash must decode to exactly 32 bytes", err)
	}
	context["receipt_ura"] = receiptURA
	context["receipt_hash_hex"] = receiptHash
	return context, nil
}

func receiptStringField(obj map[string]any, key string) (string, bool) {
	value, ok := obj[key].(string)
	if !ok {
		return "", false
	}
	value = strings.TrimSpace(value)
	return value, value != ""
}

func receiptHashField(obj map[string]any) (string, bool) {
	for _, key := range []string{"self_hash_hex", "receipt_hash_hex", "receipt_hash"} {
		value, ok := receiptStringField(obj, key)
		if !ok {
			continue
		}
		value = strings.TrimPrefix(value, "sha256:")
		value = strings.ToLower(strings.TrimSpace(value))
		if value != "" {
			return value, true
		}
	}
	return "", false
}

func wrapReceiptTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return withProfileErrorDetails(sdkErr, receiptProfile)
	}
	return transportProfileError(receiptProfile, message, cause)
}
