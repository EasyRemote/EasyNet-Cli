package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"sort"
	"strings"
	"time"
)

const missionProfile = "mission"

// MissionCarrierBase is the complete carrier context shared by Mission operations.
type MissionCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// MissionRunRequest submits EAL source to daemon-owned Mission execution.
type MissionRunRequest struct {
	MissionCarrierBase
	Source string `json:"source"`
	Label  string `json:"label,omitempty"`
}

// MissionRunFileRequest submits an EAL source file to daemon-owned Mission execution.
type MissionRunFileRequest struct {
	MissionCarrierBase
	Path  string `json:"path"`
	Label string `json:"label,omitempty"`
}

// MissionTrackRequest tracks one daemon Mission run.
type MissionTrackRequest struct {
	MissionCarrierBase
	MissionID string `json:"mission_id"`
}

// MissionCancelRequest cancels one daemon Mission run.
type MissionCancelRequest struct {
	MissionCarrierBase
	MissionID string `json:"mission_id"`
}

// MissionEventListRequest requests mission timeline events from daemon-owned storage.
type MissionEventListRequest struct {
	MissionCarrierBase
	MissionID      string `json:"mission_id"`
	CursorSequence int64  `json:"cursor_sequence"`
	Limit          int    `json:"limit,omitempty"`
}

type MissionID string

// MissionRun is a Mission run projection. The status carries the durable state.
type MissionRun struct {
	Status MissionStatus `json:"status"`
}

type MissionCancelResult = MissionStatus

// MissionStatus is the sdk/schemas/mission-status.schema.json projection.
type MissionStatus struct {
	Profile            string                   `json:"profile"`
	Kind               string                   `json:"kind"`
	MissionID          string                   `json:"mission_id"`
	State              string                   `json:"state"`
	Terminal           bool                     `json:"terminal"`
	PartialFailures    int                      `json:"partial_failures"`
	Cancelled          bool                     `json:"cancelled"`
	ParentInvocationID *string                  `json:"parent_invocation_id"`
	ParentReceiptURA   *string                  `json:"parent_receipt_ura"`
	ParentInvocation   map[string]any           `json:"parent_invocation"`
	ChildInvocations   []MissionChildInvocation `json:"child_invocations"`
	ChildReceipts      []MissionChildReceipt    `json:"child_receipts"`
	OutputRefs         []MissionOutputRef       `json:"output_refs"`
	Error              *SDKError                `json:"error"`
	Metadata           map[string]any           `json:"metadata"`
}

type MissionChildInvocation struct {
	StepID        *string        `json:"step_id"`
	RequestID     *string        `json:"request_id"`
	TraceID       *string        `json:"trace_id"`
	Ability       *string        `json:"ability"`
	InvocationURA *string        `json:"invocation_ura"`
	CallerURA     *string        `json:"caller_ura"`
	CalleeURA     *string        `json:"callee_ura"`
	SubjectURA    *string        `json:"subject_ura"`
	MetadataState *string        `json:"metadata_state"`
	LedgerState   any            `json:"ledger_state"`
	Receipt       map[string]any `json:"receipt"`
}

type MissionChildReceipt struct {
	StepID        *string `json:"step_id"`
	InvocationURA *string `json:"invocation_ura"`
	ReceiptURA    string  `json:"receipt_ura"`
	ReceiptHash   string  `json:"receipt_hash"`
}

type MissionOutputRef struct {
	Kind     string         `json:"kind"`
	Path     string         `json:"path,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

// MissionEvent is one daemon Mission timeline event projection.
type MissionEvent struct {
	Profile        string         `json:"profile"`
	Kind           string         `json:"kind"`
	MissionID      string         `json:"mission_id"`
	Sequence       int64          `json:"sequence"`
	EventType      string         `json:"event_type"`
	OccurredUnixMS int64          `json:"occurred_unix_ms"`
	Terminal       bool           `json:"terminal"`
	Payload        any            `json:"payload"`
	Receipt        map[string]any `json:"receipt"`
	Metadata       map[string]any `json:"metadata"`
}

// MissionEventPage is a replay page over daemon Mission timeline events.
type MissionEventPage struct {
	Profile            string         `json:"profile"`
	Kind               string         `json:"kind"`
	MissionID          string         `json:"mission_id"`
	CursorSequence     int64          `json:"cursor_sequence"`
	NextCursorSequence int64          `json:"next_cursor_sequence"`
	HasMore            bool           `json:"has_more"`
	DroppedCount       int64          `json:"dropped_count"`
	Events             []MissionEvent `json:"events"`
	Metadata           map[string]any `json:"metadata"`
}

// MissionEventTailSleep lets tests and integrations control bounded tail sleeps.
type MissionEventTailSleep func(ctx context.Context, delay time.Duration) error

// MissionEventTailOptions configures the SDK-owned Mission event tail state machine.
type MissionEventTailOptions struct {
	CursorSequence int64
	Limit          int
	MaxEmptyPages  int
	PollInterval   time.Duration
	Sleep          MissionEventTailSleep
}

// MissionEventTailer is a bounded facade state machine over daemon Mission
// event pages. The daemon owns event storage and projection; the SDK owns only
// cursor progress, terminal closure, and drop detection.
type MissionEventTailer struct {
	client         *MissionClient
	request        MissionEventListRequest
	options        MissionEventTailOptions
	cursorSequence int64
	buffer         []MissionEvent
	emptyPages     int
	closed         bool
	terminalSeen   bool
}

// MissionPlanStepOutput is a dataflow reference to one Mission plan step result.
type MissionPlanStepOutput struct {
	Alias string
}

func (o MissionPlanStepOutput) Render() string {
	return o.Alias + ".output"
}

// MissionPlanStep is one SDK-owned Mission/EAL plan step.
type MissionPlanStep struct {
	Alias     string
	Ref       string
	Args      map[string]any
	On        string
	Timeout   *int
	Retries   *int
	OnFailure string
	Optional  bool
}

func (s MissionPlanStep) Output() MissionPlanStepOutput {
	return MissionPlanStepOutput{Alias: s.Alias}
}

func (s MissionPlanStep) Render() (string, error) {
	if s.Alias == "" || s.Ref == "" {
		return "", invalidMissionPlan("mission plan step alias and ref are required", "invalid_step")
	}
	parts := []string{"let " + s.Alias + " = call " + missionEALString(s.Ref)}
	if s.On != "" {
		parts = append(parts, "on "+missionEALString(s.On))
	}
	if len(s.Args) > 0 {
		keys := make([]string, 0, len(s.Args))
		for key := range s.Args {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		fields := make([]string, 0, len(keys))
		for _, key := range keys {
			rendered, err := missionEALField(s.Args[key])
			if err != nil {
				return "", err
			}
			fields = append(fields, key+" = "+rendered)
		}
		parts = append(parts, "with { "+strings.Join(fields, ", ")+" }")
	}
	if s.Timeout != nil {
		parts = append(parts, fmt.Sprintf("timeout %d", *s.Timeout))
	}
	if s.Retries != nil {
		parts = append(parts, fmt.Sprintf("retries %d", *s.Retries))
	}
	if s.OnFailure != "" {
		parts = append(parts, "on_failure "+s.OnFailure)
	}
	if s.Optional {
		parts = append(parts, "optional")
	}
	return strings.Join(parts, " "), nil
}

// MissionPlanStepOptions configures one MissionPlan step.
type MissionPlanStepOptions struct {
	On             string
	TimeoutSeconds *float64
	Retries        *int
	OnFailure      string
	Optional       bool
	Args           map[string]any
}

// MissionChildInvocationIntent is the SDK projection of a child Invocation
// expected from a Mission plan step.
type MissionChildInvocationIntent struct {
	StepID    string
	Ability   string
	On        string
	Optional  bool
	OnFailure string
}

// MissionChildInvocationConformance matches daemon MissionStatus child facts
// against a Mission plan.
type MissionChildInvocationConformance struct {
	MissionID              string
	ExpectedSteps          []string
	ObservedSteps          []string
	MissingSteps           []string
	UnexpectedSteps        []string
	AbilityMismatchedSteps []string
	IncompleteFactSteps    []string
	ReceiptBackedSteps     []string
}

func (c MissionChildInvocationConformance) Passed() bool {
	return len(c.MissingSteps) == 0 &&
		len(c.UnexpectedSteps) == 0 &&
		len(c.AbilityMismatchedSteps) == 0 &&
		len(c.IncompleteFactSteps) == 0
}

func (c MissionChildInvocationConformance) RequirePassed() error {
	if c.Passed() {
		return nil
	}
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     missionProfile,
		Retry:     RetryNever,
		Retryable: false,
		Message:   "Mission plan child Invocation facts do not match planned steps",
		Details: profileErrorDetails(missionProfile, map[string]any{
			"reason":                   "mission_child_invocation_mismatch",
			"mission_id":               c.MissionID,
			"missing_steps":            append([]string(nil), c.MissingSteps...),
			"unexpected_steps":         append([]string(nil), c.UnexpectedSteps...),
			"ability_mismatched_steps": append([]string(nil), c.AbilityMismatchedSteps...),
			"incomplete_fact_steps":    append([]string(nil), c.IncompleteFactSteps...),
		}),
	}
}

// MissionPlan is the SDK-owned Mission/EAL plan rendering facade.
type MissionPlan struct {
	Name      string
	CreatedBy string
	Version   string
	steps     []MissionPlanStep
	aliases   map[string]struct{}
}

func NewMissionPlan(name string) (*MissionPlan, error) {
	return NewMissionPlanWithOptions(name, "", "")
}

func NewMissionPlanWithOptions(name string, createdBy string, version string) (*MissionPlan, error) {
	cleanName, err := requiredCleanMissionString(name, "mission plan name")
	if err != nil {
		return nil, err
	}
	return &MissionPlan{
		Name:      cleanName,
		CreatedBy: strings.TrimSpace(createdBy),
		Version:   strings.TrimSpace(version),
		steps:     []MissionPlanStep{},
		aliases:   map[string]struct{}{},
	}, nil
}

func (p *MissionPlan) Steps() []MissionPlanStep {
	if p == nil {
		return nil
	}
	return append([]MissionPlanStep(nil), p.steps...)
}

func (p *MissionPlan) Step(ref string, options MissionPlanStepOptions) (MissionPlanStep, error) {
	if p == nil {
		return MissionPlanStep{}, invalidMissionPlan("mission plan is not initialized", "invalid_plan")
	}
	targetRef, err := requiredCleanMissionString(ref, "mission step target")
	if err != nil {
		return MissionPlanStep{}, err
	}
	if options.OnFailure != "" && !missionFailurePolicyAllowed(options.OnFailure) {
		return MissionPlanStep{}, invalidMissionPlan(
			fmt.Sprintf("on_failure must be one of %v, got %q", missionFailurePolicies(), options.OnFailure),
			"invalid_failure_policy",
		)
	}
	args := make(map[string]any, len(options.Args))
	for key, value := range options.Args {
		if err := validateMissionPlanField(key, value, p.aliases); err != nil {
			return MissionPlanStep{}, err
		}
		args[key] = value
	}
	timeout, err := missionTimeoutSeconds(options.TimeoutSeconds)
	if err != nil {
		return MissionPlanStep{}, err
	}
	retries, err := missionRetriesCount(options.Retries)
	if err != nil {
		return MissionPlanStep{}, err
	}
	step := MissionPlanStep{
		Alias:     p.freshAlias(targetRef),
		Ref:       targetRef,
		Args:      args,
		On:        strings.TrimSpace(options.On),
		Timeout:   timeout,
		Retries:   retries,
		OnFailure: options.OnFailure,
		Optional:  options.Optional,
	}
	p.steps = append(p.steps, step)
	p.aliases[step.Alias] = struct{}{}
	return step, nil
}

func (p *MissionPlan) ToEAL() (string, error) {
	if p == nil {
		return "", invalidMissionPlan("mission plan is not initialized", "invalid_plan")
	}
	if len(p.steps) == 0 {
		return "", invalidMissionPlan(
			fmt.Sprintf("mission plan %q has no steps", p.Name),
			"empty_mission_plan",
		)
	}
	lines := []string{}
	if p.Version != "" {
		lines = append(lines, "// generated by easynet daemon sdk "+p.Version)
	} else {
		lines = append(lines, "// generated by easynet daemon sdk")
	}
	if p.CreatedBy != "" {
		lines = append(lines, "// created_by: "+p.CreatedBy)
	}
	lines = append(lines, "mission "+missionEALString(p.Name)+" {")
	for _, step := range p.steps {
		rendered, err := step.Render()
		if err != nil {
			return "", err
		}
		lines = append(lines, "  "+rendered)
	}
	lines = append(lines, "}")
	return strings.Join(lines, "\n") + "\n", nil
}

func (p *MissionPlan) ChildInvocationIntents() []MissionChildInvocationIntent {
	if p == nil {
		return nil
	}
	intents := make([]MissionChildInvocationIntent, 0, len(p.steps))
	for _, step := range p.steps {
		intents = append(intents, MissionChildInvocationIntent{
			StepID:    step.Alias,
			Ability:   step.Ref,
			On:        step.On,
			Optional:  step.Optional,
			OnFailure: step.OnFailure,
		})
	}
	return intents
}

func (p *MissionPlan) ValidateChildInvocations(status MissionStatus) (MissionChildInvocationConformance, error) {
	intents := p.ChildInvocationIntents()
	expectedByStep := map[string]MissionChildInvocationIntent{}
	expected := map[string]struct{}{}
	for _, intent := range intents {
		expectedByStep[intent.StepID] = intent
		expected[intent.StepID] = struct{}{}
	}

	observedByStep := map[string]MissionChildInvocation{}
	observed := map[string]struct{}{}
	receiptBacked := map[string]struct{}{}
	for _, child := range status.ChildInvocations {
		if child.StepID == nil || *child.StepID == "" {
			continue
		}
		stepID := *child.StepID
		observed[stepID] = struct{}{}
		observedByStep[stepID] = child
		if child.Receipt != nil {
			receiptBacked[stepID] = struct{}{}
		}
	}

	abilityMismatched := map[string]struct{}{}
	incompleteFacts := map[string]struct{}{}
	for stepID, intent := range expectedByStep {
		child, ok := observedByStep[stepID]
		if !ok {
			continue
		}
		if !missionChildInvocationFactComplete(child) {
			incompleteFacts[stepID] = struct{}{}
		}
		if child.Ability == nil || *child.Ability == "" {
			continue
		}
		if *child.Ability != intent.Ability {
			abilityMismatched[stepID] = struct{}{}
		}
	}

	conformance := MissionChildInvocationConformance{
		MissionID:              status.MissionID,
		ExpectedSteps:          sortedSet(expected),
		ObservedSteps:          sortedSet(observed),
		MissingSteps:           sortedSet(setDifference(expected, observed)),
		UnexpectedSteps:        sortedSet(setDifference(observed, expected)),
		AbilityMismatchedSteps: sortedSet(abilityMismatched),
		IncompleteFactSteps:    sortedSet(incompleteFacts),
		ReceiptBackedSteps:     sortedSet(setIntersection(receiptBacked, expected)),
	}
	return conformance, conformance.RequirePassed()
}

// MissionTransport supplies daemon Mission operations behind the facade.
type MissionTransport interface {
	BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunFile(ctx context.Context, requestJSON []byte) ([]byte, error)
	Track(ctx context.Context, requestJSON []byte) ([]byte, error)
	Cancel(ctx context.Context, requestJSON []byte) ([]byte, error)
	Events(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// MissionTransportFunc adapts functions into a MissionTransport.
type MissionTransportFunc struct {
	BuildRunEALInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildRunFileInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildTrackInvocationFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCancelInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunEALFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RunFileFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	TrackFunc                  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CancelFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	EventsFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f MissionTransportFunc) BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildRunEALInvocationFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission run invocation transport function is required")
	}
	return f.BuildRunEALInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildRunFileInvocationFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission run-file invocation transport function is required")
	}
	return f.BuildRunFileInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildTrackInvocationFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission track invocation transport function is required")
	}
	return f.BuildTrackInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildCancelInvocationFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission cancel invocation transport function is required")
	}
	return f.BuildCancelInvocationFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RunEALFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission run transport function is required")
	}
	return f.RunEALFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) RunFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RunFileFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission run-file transport function is required")
	}
	return f.RunFileFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Track(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.TrackFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission track transport function is required")
	}
	return f.TrackFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Cancel(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.CancelFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission cancel transport function is required")
	}
	return f.CancelFunc(ctx, requestJSON)
}

func (f MissionTransportFunc) Events(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EventsFunc == nil {
		return nil, invalidProfileClient(missionProfile, "mission events transport function is required")
	}
	return f.EventsFunc(ctx, requestJSON)
}

// MissionClient is the Mission profile facade.
type MissionClient struct {
	transport MissionTransport
	lifecycle profileClientLifecycle
}

func NewMissionClient(transport MissionTransport) (*MissionClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(missionProfile, "mission transport is required")
	}
	return &MissionClient{transport: transport}, nil
}

func (c *MissionClient) BuildRunEALInvocation(ctx context.Context, req MissionRunRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionRunRequest, c.transport.BuildRunEALInvocation, "mission run invocation failed")
}

func (c *MissionClient) BuildRunFileInvocation(ctx context.Context, req MissionRunFileRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionRunFileRequest, c.transport.BuildRunFileInvocation, "mission run-file invocation failed")
}

func (c *MissionClient) BuildTrackInvocation(ctx context.Context, req MissionTrackRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionTrackRequest, c.transport.BuildTrackInvocation, "mission track invocation failed")
}

func (c *MissionClient) BuildCancelInvocation(ctx context.Context, req MissionCancelRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateMissionCancelRequest, c.transport.BuildCancelInvocation, "mission cancel invocation failed")
}

func (c *MissionClient) RunEAL(ctx context.Context, req MissionRunRequest) (MissionRun, error) {
	status, err := c.statusOperation(ctx, req, validateMissionRunRequest, c.transport.RunEAL, "mission run failed")
	return MissionRun{Status: status}, err
}

func (c *MissionClient) RunFile(ctx context.Context, path string, opts MissionRunFileRequest) (MissionRun, error) {
	opts.Path = path
	status, err := c.statusOperation(ctx, opts, validateMissionRunFileRequest, c.transport.RunFile, "mission run-file failed")
	return MissionRun{Status: status}, err
}

func (c *MissionClient) Track(ctx context.Context, req MissionTrackRequest) (MissionStatus, error) {
	return c.statusOperation(ctx, req, validateMissionTrackRequest, c.transport.Track, "mission track failed")
}

func (c *MissionClient) Cancel(ctx context.Context, req MissionCancelRequest) (MissionCancelResult, error) {
	return c.statusOperation(ctx, req, validateMissionCancelRequest, c.transport.Cancel, "mission cancel failed")
}

func (c *MissionClient) Events(ctx context.Context, req MissionEventListRequest) (MissionEventPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return MissionEventPage{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validateMissionEventListRequest)
	if err != nil {
		return MissionEventPage{}, err
	}
	raw, err := c.transport.Events(ctx, requestJSON)
	if err != nil {
		return MissionEventPage{}, wrapMissionTransportError("mission events failed", err)
	}
	return NewMissionEventPageFromJSON(raw)
}

func (c *MissionClient) TailEvents(ctx context.Context, req MissionEventListRequest, opts MissionEventTailOptions) (*MissionEventTailer, error) {
	if err := c.requireReady(ctx); err != nil {
		return nil, err
	}
	if opts.CursorSequence == 0 {
		opts.CursorSequence = req.CursorSequence
	}
	if opts.Limit == 0 {
		opts.Limit = req.Limit
	}
	options, err := validateMissionEventTailOptions(opts)
	if err != nil {
		return nil, err
	}
	req.CursorSequence = options.CursorSequence
	req.Limit = options.Limit
	if err := validateMissionEventListRequest(req); err != nil {
		return nil, err
	}
	return &MissionEventTailer{
		client:         c,
		request:        req,
		options:        options,
		cursorSequence: options.CursorSequence,
	}, nil
}

func (c *MissionClient) buildInvocation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validate)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapMissionTransportError(label, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *MissionClient) statusOperation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (MissionStatus, error) {
	if err := c.requireReady(ctx); err != nil {
		return MissionStatus{}, err
	}
	requestJSON, err := marshalMissionRequest(req, validate)
	if err != nil {
		return MissionStatus{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return MissionStatus{}, wrapMissionTransportError(label, err)
	}
	return NewMissionStatusFromJSON(raw)
}

func (c *MissionClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(missionProfile, "mission client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "mission")
}

func (c *MissionClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(missionProfile, "mission client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "mission")
}

func (t *MissionEventTailer) CursorSequence() int64 {
	if t == nil {
		return 0
	}
	return t.cursorSequence
}

func (t *MissionEventTailer) Close() {
	if t == nil {
		return
	}
	t.closed = true
}

func (t *MissionEventTailer) Closed() bool {
	return t == nil || t.closed
}

func (t *MissionEventTailer) Next(ctx context.Context) (MissionEvent, bool, error) {
	if t == nil || t.client == nil {
		return MissionEvent{}, false, invalidProfileClient(missionProfile, "mission event tailer is not initialized")
	}
	if ctx == nil {
		return MissionEvent{}, false, invalidProfileClient(missionProfile, "context is required")
	}
	if len(t.buffer) == 0 {
		if err := t.fillBuffer(ctx); err != nil {
			return MissionEvent{}, false, err
		}
	}
	if len(t.buffer) == 0 {
		return MissionEvent{}, false, nil
	}
	event := t.buffer[0]
	t.buffer = t.buffer[1:]
	if event.Terminal {
		t.terminalSeen = true
		t.Close()
	}
	return event, true, nil
}

func (t *MissionEventTailer) fillBuffer(ctx context.Context) error {
	for !t.closed && !t.terminalSeen {
		request := t.request
		request.CursorSequence = t.cursorSequence
		request.Limit = t.options.Limit
		previousCursor := t.cursorSequence
		page, err := t.client.Events(ctx, request)
		if err != nil {
			return err
		}
		if page.DroppedCount > 0 {
			return missionEventTailDroppedError(page)
		}
		t.cursorSequence = page.NextCursorSequence
		for _, event := range page.Events {
			t.buffer = append(t.buffer, event)
			if event.Terminal {
				t.terminalSeen = true
				break
			}
		}
		if len(t.buffer) > 0 {
			return nil
		}
		if page.HasMore && t.cursorSequence == previousCursor {
			return invalidProfilePayload(missionProfile, "mission event tail made no cursor progress", nil)
		}
		if page.HasMore {
			continue
		}
		t.emptyPages++
		if t.emptyPages > t.options.MaxEmptyPages {
			t.Close()
			return nil
		}
		if err := t.sleep(ctx); err != nil {
			return err
		}
	}
	return nil
}

func (t *MissionEventTailer) sleep(ctx context.Context) error {
	delay := t.options.PollInterval
	if delay <= 0 {
		return nil
	}
	if t.options.Sleep != nil {
		return t.options.Sleep(ctx, delay)
	}
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

func NewMissionStatusFromJSON(raw []byte) (MissionStatus, error) {
	var dto struct {
		Profile            string                   `json:"profile"`
		Kind               string                   `json:"kind"`
		MissionID          string                   `json:"mission_id"`
		State              string                   `json:"state"`
		Terminal           bool                     `json:"terminal"`
		PartialFailures    int                      `json:"partial_failures"`
		Cancelled          bool                     `json:"cancelled"`
		ParentInvocationID *string                  `json:"parent_invocation_id"`
		ParentReceiptURA   *string                  `json:"parent_receipt_ura"`
		ParentInvocation   map[string]any           `json:"parent_invocation"`
		ChildInvocations   []MissionChildInvocation `json:"child_invocations"`
		ChildReceipts      []MissionChildReceipt    `json:"child_receipts"`
		OutputRefs         []MissionOutputRef       `json:"output_refs"`
		Error              json.RawMessage          `json:"error"`
		Metadata           map[string]any           `json:"metadata"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return MissionStatus{}, invalidProfilePayload(missionProfile, fmt.Sprintf("decode mission status JSON: %v", err), err)
	}
	if dto.Profile != missionProfile || dto.Kind != "mission_status" || dto.MissionID == "" ||
		dto.State == "" || dto.PartialFailures < 0 || dto.ChildInvocations == nil ||
		dto.ChildReceipts == nil || dto.OutputRefs == nil || dto.Metadata == nil {
		return MissionStatus{}, invalidProfilePayload(missionProfile, "invalid mission status projection", nil)
	}
	for _, invocation := range dto.ChildInvocations {
		if !missionChildInvocationFactComplete(invocation) {
			return MissionStatus{}, invalidProfilePayload(missionProfile, "incomplete mission child invocation fact", nil)
		}
		if invocation.Receipt != nil && !missionChildInvocationReceiptFactComplete(invocation.Receipt) {
			return MissionStatus{}, invalidProfilePayload(missionProfile, "incomplete mission child invocation receipt fact", nil)
		}
	}
	for _, receipt := range dto.ChildReceipts {
		if receipt.ReceiptURA == "" || receipt.ReceiptHash == "" {
			return MissionStatus{}, invalidProfilePayload(missionProfile, "invalid mission child receipt projection", nil)
		}
	}
	if err := validateMissionChildReceiptAnchors(dto.ParentReceiptURA, dto.ChildInvocations, dto.ChildReceipts); err != nil {
		return MissionStatus{}, err
	}
	for _, ref := range dto.OutputRefs {
		if ref.Kind == "" {
			return MissionStatus{}, invalidProfilePayload(missionProfile, "invalid mission output ref projection", nil)
		}
	}
	var sdkErr *SDKError
	if len(dto.Error) > 0 && string(dto.Error) != "null" {
		decoded, err := DecodeDaemonErrorJSON(dto.Error)
		if err != nil {
			return MissionStatus{}, err
		}
		sdkErr = decoded
	}
	return MissionStatus{
		Profile:            dto.Profile,
		Kind:               dto.Kind,
		MissionID:          dto.MissionID,
		State:              dto.State,
		Terminal:           dto.Terminal,
		PartialFailures:    dto.PartialFailures,
		Cancelled:          dto.Cancelled,
		ParentInvocationID: dto.ParentInvocationID,
		ParentReceiptURA:   dto.ParentReceiptURA,
		ParentInvocation:   dto.ParentInvocation,
		ChildInvocations:   dto.ChildInvocations,
		ChildReceipts:      dto.ChildReceipts,
		OutputRefs:         dto.OutputRefs,
		Error:              sdkErr,
		Metadata:           dto.Metadata,
	}, nil
}

func missionChildInvocationFactComplete(invocation MissionChildInvocation) bool {
	return stringPtrHasValue(invocation.StepID) &&
		stringPtrHasValue(invocation.RequestID) &&
		stringPtrHasValue(invocation.TraceID) &&
		stringPtrHasValue(invocation.Ability) &&
		stringPtrHasValue(invocation.InvocationURA) &&
		stringPtrHasValue(invocation.CallerURA) &&
		stringPtrHasValue(invocation.CalleeURA) &&
		stringPtrHasValue(invocation.SubjectURA) &&
		stringPtrHasValue(invocation.MetadataState) &&
		invocation.LedgerState != nil
}

func missionChildInvocationReceiptFactComplete(receipt map[string]any) bool {
	receiptURA, ok := receipt["receipt_ura"].(string)
	if !ok || strings.TrimSpace(receiptURA) == "" {
		return false
	}
	receiptHash, ok := receipt["receipt_hash"].(string)
	if !ok || strings.TrimSpace(receiptHash) == "" {
		return false
	}
	return true
}

func stringPtrHasValue(value *string) bool {
	return value != nil && strings.TrimSpace(*value) != ""
}

func validateMissionChildReceiptAnchors(parentReceiptURA *string, invocations []MissionChildInvocation, receipts []MissionChildReceipt) error {
	if len(receipts) == 0 {
		return nil
	}
	if parentReceiptURA == nil || *parentReceiptURA == "" {
		return invalidProfilePayload(missionProfile, "mission child receipts require parent receipt anchor", nil)
	}
	byInvocationURA := map[string]MissionChildInvocation{}
	for _, invocation := range invocations {
		if invocation.InvocationURA != nil && *invocation.InvocationURA != "" {
			byInvocationURA[*invocation.InvocationURA] = invocation
		}
	}
	for _, receipt := range receipts {
		if receipt.InvocationURA == nil || *receipt.InvocationURA == "" {
			return invalidProfilePayload(missionProfile, "mission child receipt requires invocation_ura", nil)
		}
		invocation, ok := byInvocationURA[*receipt.InvocationURA]
		if !ok || invocation.Receipt == nil {
			return invalidProfilePayload(missionProfile, "mission child receipt is not anchored to child invocation", nil)
		}
		if invocation.Receipt["receipt_ura"] != receipt.ReceiptURA || invocation.Receipt["receipt_hash"] != receipt.ReceiptHash {
			return invalidProfilePayload(missionProfile, "mission child receipt does not match child invocation receipt", nil)
		}
	}
	return nil
}

func NewMissionEventPageFromJSON(raw []byte) (MissionEventPage, error) {
	var dto struct {
		Profile            string         `json:"profile"`
		Kind               string         `json:"kind"`
		MissionID          string         `json:"mission_id"`
		CursorSequence     int64          `json:"cursor_sequence"`
		NextCursorSequence int64          `json:"next_cursor_sequence"`
		HasMore            bool           `json:"has_more"`
		DroppedCount       int64          `json:"dropped_count"`
		Events             []MissionEvent `json:"events"`
		Metadata           map[string]any `json:"metadata"`
	}
	if err := json.Unmarshal(raw, &dto); err != nil {
		return MissionEventPage{}, invalidProfilePayload(missionProfile, fmt.Sprintf("decode mission event page JSON: %v", err), err)
	}
	if dto.Profile != missionProfile || dto.Kind != "mission_event_page" || dto.MissionID == "" ||
		dto.CursorSequence < 0 || dto.NextCursorSequence < dto.CursorSequence || dto.DroppedCount < 0 ||
		dto.Events == nil || dto.Metadata == nil {
		return MissionEventPage{}, invalidProfilePayload(missionProfile, "invalid mission event page projection", nil)
	}
	var previousSequence int64
	hasPrevious := false
	for index := range dto.Events {
		event := &dto.Events[index]
		if event.Profile != missionProfile || event.Kind != "mission_event" || event.MissionID != dto.MissionID ||
			event.Sequence < 0 || event.EventType == "" || event.OccurredUnixMS < 0 || event.Metadata == nil {
			return MissionEventPage{}, invalidProfilePayload(missionProfile, "invalid mission event projection", nil)
		}
		if hasPrevious && event.Sequence <= previousSequence {
			return MissionEventPage{}, invalidProfilePayload(missionProfile, "mission events must be strictly ordered by sequence", nil)
		}
		if event.Terminal && !missionEventTypeIsTerminal(event.EventType) {
			return MissionEventPage{}, invalidProfilePayload(missionProfile, "terminal mission event has non-terminal event_type", nil)
		}
		previousSequence = event.Sequence
		hasPrevious = true
		if event.Receipt == nil {
			event.Receipt = map[string]any{}
		}
	}
	return MissionEventPage{
		Profile:            dto.Profile,
		Kind:               dto.Kind,
		MissionID:          dto.MissionID,
		CursorSequence:     dto.CursorSequence,
		NextCursorSequence: dto.NextCursorSequence,
		HasMore:            dto.HasMore,
		DroppedCount:       dto.DroppedCount,
		Events:             dto.Events,
		Metadata:           dto.Metadata,
	}, nil
}

func marshalMissionRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidProfilePayload(missionProfile, fmt.Sprintf("encode mission request: %v", err), err)
	}
	return requestJSON, nil
}

func validateMissionRunRequest(req any) error {
	value := req.(MissionRunRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if value.Source == "" {
		return invalidProfilePayload(missionProfile, "mission source is required", nil)
	}
	return nil
}

func validateMissionRunFileRequest(req any) error {
	value := req.(MissionRunFileRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if value.Path == "" || !strings.HasPrefix(value.Path, "/") {
		return invalidProfilePayload(missionProfile, "absolute mission file path is required", nil)
	}
	return nil
}

func validateMissionTrackRequest(req any) error {
	value := req.(MissionTrackRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	return validateMissionID(value.MissionID)
}

func validateMissionCancelRequest(req any) error {
	value := req.(MissionCancelRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	return validateMissionID(value.MissionID)
}

func validateMissionEventListRequest(req any) error {
	value := req.(MissionEventListRequest)
	if err := validateMissionCarrierBase(value.MissionCarrierBase); err != nil {
		return err
	}
	if err := validateMissionID(value.MissionID); err != nil {
		return err
	}
	if value.CursorSequence < 0 {
		return invalidProfilePayload(missionProfile, "mission event cursor_sequence must be non-negative", nil)
	}
	if value.Limit < 0 {
		return invalidProfilePayload(missionProfile, "mission event limit must be non-negative", nil)
	}
	if value.Limit > 1000 {
		return invalidProfilePayload(missionProfile, "mission event limit exceeds bounds", nil)
	}
	return nil
}

func validateMissionEventTailOptions(opts MissionEventTailOptions) (MissionEventTailOptions, error) {
	if opts.CursorSequence < 0 {
		return MissionEventTailOptions{}, invalidProfilePayload(missionProfile, "mission event cursor_sequence must be non-negative", nil)
	}
	if opts.Limit < 0 {
		return MissionEventTailOptions{}, invalidProfilePayload(missionProfile, "mission event limit must be non-negative", nil)
	}
	if opts.Limit > 1000 {
		return MissionEventTailOptions{}, invalidProfilePayload(missionProfile, "mission event limit exceeds bounds", nil)
	}
	if opts.MaxEmptyPages < 0 {
		return MissionEventTailOptions{}, invalidProfilePayload(missionProfile, "mission event max_empty_pages must be non-negative", nil)
	}
	if opts.PollInterval < 0 {
		return MissionEventTailOptions{}, invalidProfilePayload(missionProfile, "mission event poll_interval must be non-negative", nil)
	}
	return opts, nil
}

func validateMissionCarrierBase(base MissionCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidProfilePayload(missionProfile, "complete mission invocation carrier is required", nil)
	}
	return nil
}

func validateMissionID(missionID string) error {
	if missionID == "" {
		return invalidProfilePayload(missionProfile, "mission_id is required", nil)
	}
	if strings.Contains(missionID, "/") || strings.Contains(missionID, "\\") || strings.Contains(missionID, "://") {
		return invalidProfilePayload(missionProfile, "mission_id must not be path-like", nil)
	}
	return nil
}

func missionEventTailDroppedError(page MissionEventPage) error {
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     missionProfile,
		Retry:     RetrySafe,
		Retryable: true,
		Message:   "mission event tail dropped daemon events",
		Details: profileErrorDetails(missionProfile, map[string]any{
			"reason":          "mission_events_dropped",
			"mission_id":      page.MissionID,
			"cursor_sequence": page.CursorSequence,
			"dropped_count":   page.DroppedCount,
		}),
	}
}

func missionEventTypeIsTerminal(eventType string) bool {
	switch eventType {
	case "completed", "failed", "cancelled", "canceled":
		return true
	default:
		return false
	}
}

func requiredCleanMissionString(value string, field string) (string, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return "", invalidMissionPlan(field+" is required", "missing_mission_plan_field")
	}
	if strings.ContainsAny(trimmed, "\r\n\t") {
		return "", invalidMissionPlan(field+" must be a single-line string", "invalid_mission_plan_field")
	}
	return trimmed, nil
}

func invalidMissionPlan(message string, reason string) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     missionProfile,
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Details: profileErrorDetails(missionProfile, map[string]any{
			"reason": reason,
		}),
	}
}

func missionFailurePolicies() []string {
	return []string{"abort", "continue", "retry", "skip"}
}

func missionFailurePolicyAllowed(policy string) bool {
	for _, allowed := range missionFailurePolicies() {
		if policy == allowed {
			return true
		}
	}
	return false
}

func missionTimeoutSeconds(value *float64) (*int, error) {
	if value == nil {
		return nil, nil
	}
	if math.IsNaN(*value) || math.IsInf(*value, 0) || *value <= 0 {
		return nil, invalidMissionPlan("timeout must be a positive finite number", "invalid_timeout")
	}
	seconds := int(math.Ceil(*value))
	return &seconds, nil
}

func missionRetriesCount(value *int) (*int, error) {
	if value == nil {
		return nil, nil
	}
	if *value < 0 {
		return nil, invalidMissionPlan("retries must be non-negative", "invalid_retries")
	}
	return value, nil
}

func validateMissionPlanField(name string, value any, aliases map[string]struct{}) error {
	if _, err := requiredCleanMissionString(name, "mission plan field name"); err != nil {
		return err
	}
	switch typed := value.(type) {
	case MissionPlanStepOutput:
		return validateMissionPlanStepOutput(name, typed.Alias, aliases)
	case *MissionPlanStepOutput:
		if typed == nil {
			return invalidMissionPlan(
				fmt.Sprintf("argument %q is nil; EAL field values are scalars or step outputs", name),
				"non_scalar_field",
			)
		}
		return validateMissionPlanStepOutput(name, typed.Alias, aliases)
	case bool, string, int, int8, int16, int32, int64, uint, uint8, uint16, uint32, uint64:
		return nil
	case float32:
		if math.IsNaN(float64(typed)) || math.IsInf(float64(typed), 0) {
			return invalidMissionPlan(
				fmt.Sprintf("argument %q is non-finite; EAL numbers must be finite", name),
				"non_finite_field",
			)
		}
		return nil
	case float64:
		if math.IsNaN(typed) || math.IsInf(typed, 0) {
			return invalidMissionPlan(
				fmt.Sprintf("argument %q is non-finite; EAL numbers must be finite", name),
				"non_finite_field",
			)
		}
		return nil
	default:
		return invalidMissionPlan(
			fmt.Sprintf("argument %q is %T; EAL field values are scalars or step outputs", name, value),
			"non_scalar_field",
		)
	}
}

func validateMissionPlanStepOutput(name string, alias string, aliases map[string]struct{}) error {
	if _, ok := aliases[alias]; ok {
		return nil
	}
	return invalidMissionPlan(
		fmt.Sprintf("argument %q references step %q, which is not part of this mission plan", name, alias),
		"foreign_step_output",
	)
}

func missionEALString(value string) string {
	bytes, _ := json.Marshal(value)
	return string(bytes)
}

func missionEALField(value any) (string, error) {
	switch typed := value.(type) {
	case MissionPlanStepOutput:
		return typed.Render(), nil
	case *MissionPlanStepOutput:
		if typed == nil {
			return "", invalidMissionPlan("EAL field value must be scalar or step output, got nil", "non_scalar_field")
		}
		return typed.Render(), nil
	case string:
		return missionEALString(typed), nil
	case bool:
		if typed {
			return "true", nil
		}
		return "false", nil
	case nil:
		return "", invalidMissionPlan("EAL field value must be scalar or step output, got nil", "non_scalar_field")
	case int, int8, int16, int32, int64, uint, uint8, uint16, uint32, uint64:
		bytes, err := json.Marshal(typed)
		if err != nil {
			return "", invalidMissionPlan("mission plan numeric argument is invalid", "invalid_argument_value")
		}
		return string(bytes), nil
	case float32:
		if math.IsNaN(float64(typed)) || math.IsInf(float64(typed), 0) {
			return "", invalidMissionPlan("EAL number must be finite", "non_finite_field")
		}
		bytes, err := json.Marshal(typed)
		if err != nil {
			return "", invalidMissionPlan("mission plan numeric argument is invalid", "invalid_argument_value")
		}
		return string(bytes), nil
	case float64:
		if math.IsNaN(typed) || math.IsInf(typed, 0) {
			return "", invalidMissionPlan("EAL number must be finite", "non_finite_field")
		}
		bytes, err := json.Marshal(typed)
		if err != nil {
			return "", invalidMissionPlan("mission plan numeric argument is invalid", "invalid_argument_value")
		}
		return string(bytes), nil
	default:
		return "", invalidMissionPlan(
			fmt.Sprintf("EAL field value must be scalar or step output, got %T", value),
			"non_scalar_field",
		)
	}
}

func (p *MissionPlan) freshAlias(ref string) string {
	base := missionIdentifier(ref)
	alias := base
	for counter := 2; ; counter++ {
		if _, exists := p.aliases[alias]; !exists {
			return alias
		}
		alias = fmt.Sprintf("%s_%d", base, counter)
	}
}

func missionIdentifier(ref string) string {
	parts := strings.Split(ref, ".")
	base := parts[len(parts)-1]
	var builder strings.Builder
	for _, r := range base {
		if (r >= 'A' && r <= 'Z') || (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '_' {
			builder.WriteRune(r)
		} else {
			builder.WriteByte('_')
		}
	}
	if builder.Len() == 0 {
		return "step"
	}
	return builder.String()
}

func sortedSet(values map[string]struct{}) []string {
	result := make([]string, 0, len(values))
	for value := range values {
		result = append(result, value)
	}
	sort.Strings(result)
	return result
}

func setDifference(left map[string]struct{}, right map[string]struct{}) map[string]struct{} {
	result := map[string]struct{}{}
	for value := range left {
		if _, ok := right[value]; !ok {
			result[value] = struct{}{}
		}
	}
	return result
}

func setIntersection(left map[string]struct{}, right map[string]struct{}) map[string]struct{} {
	result := map[string]struct{}{}
	for value := range left {
		if _, ok := right[value]; ok {
			result[value] = struct{}{}
		}
	}
	return result
}

func wrapMissionTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return withProfileErrorDetails(sdkErr, missionProfile)
	}
	return transportProfileError(missionProfile, message, cause)
}
