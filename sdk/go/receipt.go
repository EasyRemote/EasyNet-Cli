package easynet

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"

	axonsdk "axon.run/sdk/go/axon"
	axoninv "axon.run/sdk/go/axon/invocation"
)

const (
	DefaultReceiptHistoryLimit uint32 = 50
	MaxReceiptHistoryLimit     uint32 = 500
	maxReceiptHistoryCursorLen        = 4096
)

type InvocationReceiptAnchor = axoninv.InvocationReceiptAnchor
type InvocationReceiptChainSummary = axoninv.InvocationReceiptChainSummary
type InvocationCausalLink = axoninv.InvocationCausalLink
type InvocationTraceEdge = axoninv.InvocationTraceEdge
type InvocationLedgerRecord = axoninv.InvocationLedgerRecord
type InvocationTraceGraph = axoninv.InvocationTraceGraph

// ReceiptReference is a daemon/Axon-issued locator plus the exact canonical
// receipt hash. The runtime SDK validates and projects it; it never fabricates
// a receipt URA.
type ReceiptReference struct {
	ReceiptURA  string   `json:"receipt_ura"`
	ReceiptHash [32]byte `json:"receipt_hash"`
}

func NewReceiptReference(receiptURA string, receiptHash []byte) (ReceiptReference, error) {
	reference := ReceiptReference{
		ReceiptURA: strings.TrimSpace(receiptURA),
	}
	if len(receiptHash) != len(reference.ReceiptHash) {
		return ReceiptReference{}, invalidReceipt("receipt_hash must contain exactly 32 bytes", nil)
	}
	copy(reference.ReceiptHash[:], receiptHash)
	if err := reference.Validate(); err != nil {
		return ReceiptReference{}, err
	}
	return reference, nil
}

func ReceiptReferenceFromAnchor(anchor InvocationReceiptAnchor) (ReceiptReference, error) {
	receiptHash, err := hex.DecodeString(strings.TrimSpace(anchor.ReceiptHash))
	if err != nil {
		return ReceiptReference{}, invalidReceipt("receipt anchor hash must be hexadecimal", err)
	}
	return NewReceiptReference(anchor.ReceiptURA, receiptHash)
}

// ReceiptReferenceFromRuntimeReceipt projects a daemon runtime receipt summary
// into the scalar causal reference used by Invocation builders. The SDK owns
// receipt URA and hash validation; products must not fabricate anchors from
// partial summaries.
func ReceiptReferenceFromRuntimeReceipt(receipt RuntimeReceipt) (ReceiptReference, error) {
	if strings.TrimSpace(receipt.ReceiptURA) == "" {
		return ReceiptReference{}, invalidReceipt("runtime receipt summary is missing receipt_ura", nil)
	}
	receiptHash, err := hex.DecodeString(strings.TrimSpace(receipt.SelfHashHex))
	if err != nil {
		return ReceiptReference{}, invalidReceipt("runtime receipt summary self_hash_hex must be hexadecimal", err)
	}
	return NewReceiptReference(receipt.ReceiptURA, receiptHash)
}

func (r ReceiptReference) Validate() error {
	if strings.TrimSpace(r.ReceiptURA) == "" {
		return invalidReceipt("receipt_ura is required", nil)
	}
	if _, err := axonsdk.ParseURA(strings.TrimSpace(r.ReceiptURA)); err != nil {
		return invalidReceipt("receipt_ura must be a canonical URA", err)
	}
	return nil
}

// CausalContext projects this reference through Axon's causal union and JSON
// codec. The runtime SDK does not own a second causal wire format.
func (r ReceiptReference) CausalContext() (map[string]any, error) {
	if err := r.Validate(); err != nil {
		return nil, err
	}
	projection := axoninv.CausalFromCtx(axoninv.CausalScalarCtx(axoninv.ReceiptRef{
		ReceiptHash: r.ReceiptHash,
		ReceiptURA:  strings.TrimSpace(r.ReceiptURA),
	}))
	raw, err := json.Marshal(projection)
	if err != nil {
		return nil, invalidReceipt("encode Axon causal context", err)
	}
	var result map[string]any
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, invalidReceipt("project Axon causal context", err)
	}
	return result, nil
}

// ReceiptLookup selects one ledger record or trace root. Exactly one selector
// must be set.
type ReceiptLookup struct {
	InvocationURA string `json:"invocation_ura,omitempty"`
	RequestID     string `json:"request_id,omitempty"`
	TraceID       string `json:"trace_id,omitempty"`
}

func (l ReceiptLookup) Validate() error {
	count := 0
	for _, value := range []string{l.InvocationURA, l.RequestID, l.TraceID} {
		if strings.TrimSpace(value) != "" {
			count++
		}
	}
	if count != 1 {
		return invalidReceipt("receipt lookup requires exactly one invocation_ura, request_id, or trace_id", nil)
	}
	if value := strings.TrimSpace(l.InvocationURA); value != "" {
		if _, err := axonsdk.ParseURA(value); err != nil {
			return invalidReceipt("invocation_ura must be a canonical URA", err)
		}
	}
	return nil
}

func (l ReceiptLookup) arguments() (map[string]any, error) {
	if err := l.Validate(); err != nil {
		return nil, err
	}
	key := map[string]any{}
	putReceiptString(key, "ura", l.InvocationURA)
	putReceiptString(key, "request_id", l.RequestID)
	putReceiptString(key, "trace_id", l.TraceID)
	return key, nil
}

type ReceiptFilter struct {
	CallerURA   string   `json:"caller_ura,omitempty"`
	CalleeURA   string   `json:"callee_ura,omitempty"`
	SubjectURAs []string `json:"subject_uras,omitempty"`
	AbilityURAs []string `json:"ability_uras,omitempty"`
	State       string   `json:"state,omitempty"`
	TraceID     string   `json:"trace_id,omitempty"`
}

func (f ReceiptFilter) arguments() (map[string]any, error) {
	filter := map[string]any{}
	for key, value := range map[string]string{
		"caller_ura": f.CallerURA,
		"callee_ura": f.CalleeURA,
	} {
		if err := putReceiptURA(filter, key, value); err != nil {
			return nil, err
		}
	}
	subjects, err := receiptURAList(f.SubjectURAs, "subject_uras")
	if err != nil {
		return nil, err
	}
	if len(subjects) != 0 {
		filter["subject_uras"] = subjects
	}
	abilities, err := receiptURAList(f.AbilityURAs, "ability_uras")
	if err != nil {
		return nil, err
	}
	switch len(abilities) {
	case 1:
		filter["ability_ura"] = abilities[0]
	case 0:
	default:
		filter["ability_uras"] = abilities
	}
	putReceiptString(filter, "state", f.State)
	putReceiptString(filter, "trace_id", f.TraceID)
	return filter, nil
}

type ReceiptListRequest struct {
	Call               RuntimeCallContext `json:"call"`
	Lookup             *ReceiptLookup     `json:"lookup,omitempty"`
	Filter             ReceiptFilter      `json:"filter,omitempty"`
	Limit              uint32             `json:"limit,omitempty"`
	Cursor             string             `json:"cursor,omitempty"`
	ExcludeAbilityURAs []string           `json:"exclude_ability_uras,omitempty"`
}

type ReceiptGetRequest struct {
	Call   RuntimeCallContext `json:"call"`
	Lookup ReceiptLookup      `json:"lookup"`
	Filter ReceiptFilter      `json:"filter,omitempty"`
}

type ReceiptTraceRequest struct {
	Call   RuntimeCallContext `json:"call"`
	Lookup ReceiptLookup      `json:"lookup"`
	Filter ReceiptFilter      `json:"filter,omitempty"`
}

type ReceiptLedgerSource struct {
	LedgerURA string `json:"ledger_ura,omitempty"`
}

type ReceiptHistoryPage struct {
	Source     ReceiptLedgerSource      `json:"source"`
	Records    []InvocationLedgerRecord `json:"records"`
	Limit      uint32                   `json:"limit"`
	NextCursor string                   `json:"next_cursor,omitempty"`
}

type ReceiptGetResult struct {
	Source ReceiptLedgerSource     `json:"source"`
	Record *InvocationLedgerRecord `json:"record,omitempty"`
}

type ReceiptTraceResult struct {
	Source ReceiptLedgerSource  `json:"source"`
	Graph  InvocationTraceGraph `json:"graph"`
}

type ReceiptProvider interface {
	List(context.Context, ReceiptListRequest) (ReceiptHistoryPage, error)
	Get(context.Context, ReceiptGetRequest) (ReceiptGetResult, error)
	Trace(context.Context, ReceiptTraceRequest) (ReceiptTraceResult, error)
}

type ReceiptClient struct {
	provider ReceiptProvider
}

func NewReceiptClient(provider ReceiptProvider) (*ReceiptClient, error) {
	if provider == nil {
		return nil, invalidReceipt("Receipt provider is required", nil)
	}
	return &ReceiptClient{provider: provider}, nil
}

func (c *ReceiptClient) List(ctx context.Context, request ReceiptListRequest) (ReceiptHistoryPage, error) {
	if c == nil || c.provider == nil {
		return ReceiptHistoryPage{}, invalidReceipt("Receipt client is not initialized", nil)
	}
	return c.provider.List(ctx, request)
}

func (c *ReceiptClient) Get(ctx context.Context, request ReceiptGetRequest) (ReceiptGetResult, error) {
	if c == nil || c.provider == nil {
		return ReceiptGetResult{}, invalidReceipt("Receipt client is not initialized", nil)
	}
	return c.provider.Get(ctx, request)
}

func (c *ReceiptClient) Trace(ctx context.Context, request ReceiptTraceRequest) (ReceiptTraceResult, error) {
	if c == nil || c.provider == nil {
		return ReceiptTraceResult{}, invalidReceipt("Receipt client is not initialized", nil)
	}
	return c.provider.Trace(ctx, request)
}

func (c *ReceiptClient) Verify(receipt *axoninv.SignedInvocationReceipt, resolver axoninv.KeyResolver) (axoninv.VerifiedReceipt, error) {
	if receipt == nil || resolver == nil {
		return axoninv.VerifiedReceipt{}, invalidReceipt("receipt and key resolver are required", nil)
	}
	return receipt.Verify(resolver)
}

func (c *ReceiptClient) VerifyChain(receipts []*axoninv.SignedInvocationReceipt) axoninv.ChainCheckResult {
	verified := make([]axoninv.SignedInvocationReceipt, 0, len(receipts))
	for index, receipt := range receipts {
		if receipt == nil {
			return axoninv.ChainCheckResult{
				OK:          false,
				BrokenIndex: int64(index),
				Detail:      "nil signed receipt",
			}
		}
		verified = append(verified, *receipt)
	}
	return axoninv.VerifyReceiptChain(verified)
}

type RuntimeReceiptProvider struct {
	ability *RuntimeAbilityClient
}

func NewRuntimeReceiptProvider(ability *RuntimeAbilityClient) (*RuntimeReceiptProvider, error) {
	if ability == nil {
		return nil, invalidReceipt("runtime ability client is required", nil)
	}
	return &RuntimeReceiptProvider{ability: ability}, nil
}

func (p *RuntimeReceiptProvider) List(ctx context.Context, request ReceiptListRequest) (ReceiptHistoryPage, error) {
	if err := p.requireReady(); err != nil {
		return ReceiptHistoryPage{}, err
	}
	limit, err := receiptHistoryLimit(request.Limit)
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	cursor, err := receiptHistoryCursor(request.Cursor)
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	args, err := receiptQueryArguments(request.Lookup, request.Filter)
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	args["limit"] = limit
	if cursor != "" {
		args["cursor"] = cursor
	}
	excluded, err := receiptURAList(request.ExcludeAbilityURAs, "exclude_ability_uras")
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	if len(excluded) != 0 {
		args["exclude_ability_uras"] = excluded
	}
	output, err := p.ability.Invoke(ctx, request.Call, receiptHistoryListAbility, args)
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	records, err := parseReceiptRecords(output["records"])
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	if uint32(len(records)) > limit {
		return ReceiptHistoryPage{}, invalidReceipt("runtime Receipt history exceeds the bounded page and has no stable cursor", nil)
	}
	nextCursor, err := optionalReceiptString(output, "next_cursor")
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	if nextCursor != "" && nextCursor == cursor {
		return ReceiptHistoryPage{}, invalidReceipt("runtime Receipt history returned a repeated cursor", nil)
	}
	source, err := receiptLedgerSource(output)
	if err != nil {
		return ReceiptHistoryPage{}, err
	}
	return ReceiptHistoryPage{
		Source:     source,
		Records:    records,
		Limit:      limit,
		NextCursor: nextCursor,
	}, nil
}

func (p *RuntimeReceiptProvider) Get(ctx context.Context, request ReceiptGetRequest) (ReceiptGetResult, error) {
	if err := p.requireReady(); err != nil {
		return ReceiptGetResult{}, err
	}
	args, err := receiptQueryArguments(&request.Lookup, request.Filter)
	if err != nil {
		return ReceiptGetResult{}, err
	}
	output, err := p.ability.Invoke(ctx, request.Call, receiptHistoryGetAbility, args)
	if err != nil {
		return ReceiptGetResult{}, err
	}
	source, err := receiptLedgerSource(output)
	if err != nil {
		return ReceiptGetResult{}, err
	}
	result := ReceiptGetResult{Source: source}
	value, present := output["record"]
	if !present {
		return ReceiptGetResult{}, invalidReceipt("Receipt get result must include record", nil)
	}
	if value == nil {
		return result, nil
	}
	record, err := parseReceiptRecord(value)
	if err != nil {
		return ReceiptGetResult{}, err
	}
	result.Record = &record
	return result, nil
}

func (p *RuntimeReceiptProvider) Trace(ctx context.Context, request ReceiptTraceRequest) (ReceiptTraceResult, error) {
	if err := p.requireReady(); err != nil {
		return ReceiptTraceResult{}, err
	}
	args, err := receiptQueryArguments(&request.Lookup, request.Filter)
	if err != nil {
		return ReceiptTraceResult{}, err
	}
	output, err := p.ability.Invoke(ctx, request.Call, receiptTraceGetAbility, args)
	if err != nil {
		return ReceiptTraceResult{}, err
	}
	nodes, err := requiredReceiptArray(output["nodes"], "Receipt trace nodes")
	if err != nil {
		return ReceiptTraceResult{}, err
	}
	edges, err := requiredReceiptArray(output["edges"], "Receipt trace edges")
	if err != nil {
		return ReceiptTraceResult{}, err
	}
	graphJSON, err := json.Marshal(map[string]any{
		"trace_id": receiptString(output, "trace_id"),
		"records":  nodes,
		"edges":    edges,
	})
	if err != nil {
		return ReceiptTraceResult{}, invalidReceipt("encode Axon trace graph projection", err)
	}
	graph, err := axoninv.ParseInvocationTraceGraphJSON(graphJSON)
	if err != nil {
		return ReceiptTraceResult{}, invalidReceipt("decode Axon trace graph projection", err)
	}
	source, err := receiptLedgerSource(output)
	if err != nil {
		return ReceiptTraceResult{}, err
	}
	return ReceiptTraceResult{
		Source: source,
		Graph:  graph,
	}, nil
}

func (p *RuntimeReceiptProvider) requireReady() error {
	if p == nil || p.ability == nil {
		return invalidReceipt("runtime Receipt provider is not initialized", nil)
	}
	return nil
}

func receiptQueryArguments(lookup *ReceiptLookup, filter ReceiptFilter) (map[string]any, error) {
	args := map[string]any{}
	if lookup != nil {
		key, err := lookup.arguments()
		if err != nil {
			return nil, err
		}
		args["key"] = key
	}
	filterArgs, err := filter.arguments()
	if err != nil {
		return nil, err
	}
	if len(filterArgs) != 0 {
		args["filter"] = filterArgs
	}
	return args, nil
}

func parseReceiptRecords(value any) ([]InvocationLedgerRecord, error) {
	rows, ok := value.([]any)
	if !ok {
		return nil, invalidReceipt("Receipt history records must be an array", nil)
	}
	records := make([]InvocationLedgerRecord, 0, len(rows))
	for index, row := range rows {
		record, err := parseReceiptRecord(row)
		if err != nil {
			return nil, invalidReceipt(fmt.Sprintf("decode Receipt history record %d", index), err)
		}
		records = append(records, record)
	}
	return records, nil
}

func parseReceiptRecord(value any) (InvocationLedgerRecord, error) {
	value = unwrapReceiptRecordValue(value)
	raw, err := json.Marshal(value)
	if err != nil {
		return InvocationLedgerRecord{}, invalidReceipt("encode Axon invocation ledger record", err)
	}
	record, err := axoninv.ParseInvocationLedgerRecordJSON(raw)
	if err != nil {
		return InvocationLedgerRecord{}, invalidReceipt("decode Axon invocation ledger record", err)
	}
	return record, nil
}

func unwrapReceiptRecordValue(value any) any {
	row, ok := value.(map[string]any)
	if !ok {
		return value
	}
	for _, key := range []string{"record", "ledger_record"} {
		nested, ok := row[key]
		if !ok || nested == nil {
			continue
		}
		if _, ok := nested.(map[string]any); ok {
			return nested
		}
	}
	return value
}

func receiptLedgerSource(output map[string]any) (ReceiptLedgerSource, error) {
	ledgerURA := receiptString(output, "ledger_ura")
	if ledgerURA == "" {
		return ReceiptLedgerSource{}, invalidReceipt("Receipt ledger_ura is required", nil)
	}
	if _, err := axonsdk.ParseURA(ledgerURA); err != nil {
		return ReceiptLedgerSource{}, invalidReceipt("Receipt ledger_ura must be a canonical URA", err)
	}
	return ReceiptLedgerSource{LedgerURA: ledgerURA}, nil
}

func receiptHistoryLimit(limit uint32) (uint32, error) {
	if limit == 0 {
		return DefaultReceiptHistoryLimit, nil
	}
	if limit > MaxReceiptHistoryLimit {
		return 0, invalidReceipt("Receipt history limit exceeds the maximum page bound", nil)
	}
	return limit, nil
}

func receiptHistoryCursor(value string) (string, error) {
	cursor := strings.TrimSpace(value)
	if len(cursor) > maxReceiptHistoryCursorLen {
		return "", invalidReceipt("Receipt history cursor exceeds the maximum bound", nil)
	}
	return cursor, nil
}

func receiptString(value map[string]any, key string) string {
	text, _ := value[key].(string)
	return strings.TrimSpace(text)
}

func optionalReceiptString(output map[string]any, key string) (string, error) {
	value, present := output[key]
	if !present || value == nil {
		return "", nil
	}
	text, ok := value.(string)
	if !ok {
		return "", invalidReceipt(key+" must be a string", nil)
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return "", invalidReceipt(key+" must be non-empty when present", nil)
	}
	if key == "next_cursor" && len(text) > maxReceiptHistoryCursorLen {
		return "", invalidReceipt("Receipt history next_cursor exceeds the maximum bound", nil)
	}
	return text, nil
}

func requiredReceiptArray(value any, field string) ([]any, error) {
	rows, ok := value.([]any)
	if !ok {
		return nil, invalidReceipt(field+" must be an array", nil)
	}
	return rows, nil
}

func putReceiptString(output map[string]any, key, value string) {
	if value = strings.TrimSpace(value); value != "" {
		output[key] = value
	}
}

func receiptURAList(values []string, field string) ([]string, error) {
	result := make([]string, 0, len(values))
	seen := map[string]struct{}{}
	for index, value := range values {
		value = strings.TrimSpace(value)
		if value == "" {
			return nil, invalidReceipt(fmt.Sprintf("%s item %d is required", field, index), nil)
		}
		if _, err := axonsdk.ParseURA(value); err != nil {
			return nil, invalidReceipt(fmt.Sprintf("%s item %d must be a canonical URA", field, index), err)
		}
		if _, ok := seen[value]; ok {
			return nil, invalidReceipt(field+" must not contain duplicates", nil)
		}
		seen[value] = struct{}{}
		result = append(result, value)
	}
	return result, nil
}

func putReceiptURA(output map[string]any, key, value string) error {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	if _, err := axonsdk.ParseURA(value); err != nil {
		return invalidReceipt(key+" must be a canonical URA", err)
	}
	output[key] = value
	return nil
}

func invalidReceipt(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "receipt",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}
