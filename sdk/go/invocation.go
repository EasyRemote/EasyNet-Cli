package easynet

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
)

// InvocationSignature carries caller signature material when present.
type InvocationSignature struct {
	Algorithm             string `json:"algorithm"`
	SignatureBase64       string `json:"signature_base64"`
	KeyIDHint             string `json:"key_id_hint,omitempty"`
	SignerPublicKeyBase64 string `json:"signer_public_key_base64,omitempty"`
}

// InvocationDraft is the immutable complete Invocation tuple accepted by
// Runtime Core prepare/submit paths.
type InvocationDraft struct {
	callerURA       string
	calleeURA       string
	descriptorRef   string
	subjectURA      string
	nonceBase64     string
	causalContext   map[string]any
	args            any
	argumentsBase64 string
	contentType     string
	metadata        map[string]any
	callerSignature *InvocationSignature
	hasArgs         bool
}

// NewInvocationNonceBase64 returns a fresh 16-byte Invocation nonce encoded
// for the shared Invocation DTO. It is a Runtime Core construction default:
// callers may use it before Build, but the filled nonce remains inspectable.
func NewInvocationNonceBase64() (string, error) {
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return "", fmt.Errorf("generate invocation nonce: %w", err)
	}
	return base64.StdEncoding.EncodeToString(nonce[:]), nil
}

// NewInvocationDraftFromJSON decodes and validates the shared Invocation JSON DTO.
func NewInvocationDraftFromJSON(raw []byte) (InvocationDraft, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil {
		return InvocationDraft{}, invalidInvocation(fmt.Sprintf("decode invocation JSON: %v", err), err)
	}
	if err := rejectUnknownInvocationFields(fields); err != nil {
		return InvocationDraft{}, err
	}
	builder := NewInvocationBuilder()
	if err := setRequiredString(fields, "caller_ura", builder.WithCallerURA); err != nil {
		return InvocationDraft{}, err
	}
	if err := setRequiredString(fields, "callee_ura", builder.WithCalleeURA); err != nil {
		return InvocationDraft{}, err
	}
	if err := setRequiredString(fields, "descriptor_ref", builder.WithDescriptorRef); err != nil {
		return InvocationDraft{}, err
	}
	if err := setRequiredString(fields, "subject_ura", builder.WithSubjectURA); err != nil {
		return InvocationDraft{}, err
	}
	if err := setRequiredString(fields, "nonce_base64", builder.WithNonceBase64); err != nil {
		return InvocationDraft{}, err
	}
	if err := setRequiredString(fields, "content_type", builder.WithContentType); err != nil {
		return InvocationDraft{}, err
	}
	causalContext, err := requiredObject(fields, "causal_context")
	if err != nil {
		return InvocationDraft{}, err
	}
	builder.WithCausalContext(causalContext)
	if rawArgs, hasArgs := fields["args"]; hasArgs {
		var args any
		if err := json.Unmarshal(rawArgs, &args); err != nil {
			return InvocationDraft{}, invalidInvocation(fmt.Sprintf("args must be valid JSON: %v", err), err)
		}
		builder.WithJSONArgs(args)
	}
	if rawArguments, hasArguments := fields["arguments_base64"]; hasArguments {
		var arguments string
		if err := json.Unmarshal(rawArguments, &arguments); err != nil {
			return InvocationDraft{}, invalidInvocation("arguments_base64 must be a string", err)
		}
		builder.WithArgumentsBase64(arguments)
	}
	if rawMetadata, ok := fields["metadata"]; ok {
		var metadata map[string]any
		if err := json.Unmarshal(rawMetadata, &metadata); err != nil {
			return InvocationDraft{}, invalidInvocation("metadata must be an object", err)
		}
		builder.WithMetadata(metadata)
	}
	if rawSignature, ok := fields["caller_signature"]; ok {
		var signature InvocationSignature
		if err := json.Unmarshal(rawSignature, &signature); err != nil {
			return InvocationDraft{}, invalidInvocation("caller_signature must be an object", err)
		}
		builder.WithCallerSignature(signature)
	}
	return builder.Build()
}

// MarshalJSON emits the shared sdk/schemas/invocation.schema.json shape.
func (d InvocationDraft) MarshalJSON() ([]byte, error) {
	obj := map[string]any{
		"caller_ura":     d.callerURA,
		"callee_ura":     d.calleeURA,
		"descriptor_ref": d.descriptorRef,
		"subject_ura":    d.subjectURA,
		"nonce_base64":   d.nonceBase64,
		"causal_context": d.causalContext,
		"content_type":   d.contentType,
	}
	if d.hasArgs {
		obj["args"] = d.args
	} else {
		obj["arguments_base64"] = d.argumentsBase64
	}
	if d.metadata != nil {
		obj["metadata"] = d.metadata
	}
	if d.callerSignature != nil {
		obj["caller_signature"] = d.callerSignature
	}
	return json.Marshal(obj)
}

func (d InvocationDraft) CallerURA() string {
	return d.callerURA
}

func (d InvocationDraft) CalleeURA() string {
	return d.calleeURA
}

func (d InvocationDraft) DescriptorRef() string {
	return d.descriptorRef
}

func (d InvocationDraft) SubjectURA() string {
	return d.subjectURA
}

func (d InvocationDraft) NonceBase64() string {
	return d.nonceBase64
}

func (d InvocationDraft) CausalContext() map[string]any {
	return copyMap(d.causalContext)
}

func (d InvocationDraft) HasJSONArgs() bool {
	return d.hasArgs
}

func (d InvocationDraft) JSONArgs() any {
	return d.args
}

func (d InvocationDraft) ArgumentsBase64() string {
	return d.argumentsBase64
}

func (d InvocationDraft) ContentType() string {
	return d.contentType
}

func (d InvocationDraft) Metadata() map[string]any {
	return copyMap(d.metadata)
}

func (d InvocationDraft) CallerSignature() *InvocationSignature {
	return copySignature(d.callerSignature)
}

// InvocationBuilder is the mutable complete Invocation tuple builder.
type InvocationBuilder struct {
	callerURA       string
	calleeURA       string
	descriptorRef   string
	subjectURA      string
	nonceBase64     string
	causalContext   map[string]any
	args            any
	argumentsBase64 string
	contentType     string
	metadata        map[string]any
	callerSignature *InvocationSignature
	authorityErr    error
	hasArgs         bool
	hasArguments    bool
	consumed        bool
}

// NewInvocationBuilder creates an empty Invocation builder.
func NewInvocationBuilder() *InvocationBuilder {
	return &InvocationBuilder{}
}

func (b *InvocationBuilder) WithCallerURA(value string) *InvocationBuilder {
	b.callerURA = value
	return b
}

func (b *InvocationBuilder) WithCalleeURA(value string) *InvocationBuilder {
	b.calleeURA = value
	return b
}

func (b *InvocationBuilder) WithDescriptorRef(value string) *InvocationBuilder {
	b.descriptorRef = value
	return b
}

func (b *InvocationBuilder) WithSubjectURA(value string) *InvocationBuilder {
	b.subjectURA = value
	return b
}

func (b *InvocationBuilder) WithNonceBase64(value string) *InvocationBuilder {
	b.nonceBase64 = value
	return b
}

func (b *InvocationBuilder) WithCausalContext(value map[string]any) *InvocationBuilder {
	b.causalContext = value
	return b
}

func (b *InvocationBuilder) WithJSONArgs(value any) *InvocationBuilder {
	b.args = value
	b.hasArgs = true
	return b
}

func (b *InvocationBuilder) WithArgumentsBase64(value string) *InvocationBuilder {
	b.argumentsBase64 = value
	b.hasArguments = true
	return b
}

func (b *InvocationBuilder) WithContentType(value string) *InvocationBuilder {
	b.contentType = value
	return b
}

func (b *InvocationBuilder) WithMetadata(value map[string]any) *InvocationBuilder {
	b.metadata = value
	return b
}

func (b *InvocationBuilder) WithAuthorityMetadata(value AuthorityMetadata) *InvocationBuilder {
	metadata, err := value.MergeInto(b.metadata)
	if err != nil {
		b.authorityErr = err
		return b
	}
	b.metadata = metadata
	return b
}

func (b *InvocationBuilder) WithCallerSignature(value InvocationSignature) *InvocationBuilder {
	value = normalizeInvocationSignatureMaterial(value)
	b.callerSignature = &value
	return b
}

// Build validates tuple completeness and returns an immutable draft.
func (b *InvocationBuilder) Build() (InvocationDraft, error) {
	draft, err := b.inspectDraft()
	if err != nil {
		return InvocationDraft{}, err
	}
	b.consumed = true
	return draft, nil
}

// Inspect validates tuple completeness without consuming the builder handle.
func (b *InvocationBuilder) Inspect() (InvocationDraft, error) {
	return b.inspectDraft()
}

func (b *InvocationBuilder) consume() error {
	if b == nil {
		return invalidInvocation("invocation builder is not initialized", nil)
	}
	if b.consumed {
		return invalidInvocationHandle("invocation builder handle is consumed")
	}
	b.consumed = true
	return nil
}

func (b *InvocationBuilder) inspectDraft() (InvocationDraft, error) {
	if b == nil {
		return InvocationDraft{}, invalidInvocation("invocation builder is not initialized", nil)
	}
	if b.consumed {
		return InvocationDraft{}, invalidInvocationHandle("invocation builder handle is consumed")
	}
	if b.authorityErr != nil {
		return InvocationDraft{}, b.authorityErr
	}
	for _, field := range []struct {
		name  string
		value string
	}{
		{"caller_ura", b.callerURA},
		{"callee_ura", b.calleeURA},
		{"descriptor_ref", b.descriptorRef},
		{"subject_ura", b.subjectURA},
		{"nonce_base64", b.nonceBase64},
		{"content_type", b.contentType},
	} {
		if strings.TrimSpace(field.value) == "" {
			return InvocationDraft{}, invalidInvocation(fmt.Sprintf("%s is required", field.name), nil)
		}
	}
	// DescriptorRef canonical validation belongs to the Addressing provider
	// and runtime identity boundary. Runtime Core validates tuple completeness here.
	if b.causalContext == nil {
		return InvocationDraft{}, invalidInvocation("causal_context is required", nil)
	}
	if err := validateInvocationNonceBase64(b.nonceBase64); err != nil {
		return InvocationDraft{}, err
	}
	if b.hasArgs == b.hasArguments {
		return InvocationDraft{}, invalidInvocation("exactly one of args or arguments_base64 is required", nil)
	}
	if b.hasArguments {
		if strings.TrimSpace(b.argumentsBase64) == "" {
			return InvocationDraft{}, invalidInvocation("arguments_base64 must be non-empty", nil)
		}
		if err := validateInvocationArgumentsBase64(b.argumentsBase64); err != nil {
			return InvocationDraft{}, err
		}
	}
	if err := validateAuthorityMetadata(b.metadata); err != nil {
		return InvocationDraft{}, err
	}
	if b.callerSignature != nil {
		signature := normalizeInvocationSignatureMaterial(*b.callerSignature)
		if strings.TrimSpace(signature.Algorithm) == "" {
			return InvocationDraft{}, invalidInvocation("caller_signature.algorithm is required", nil)
		}
		if strings.TrimSpace(signature.SignatureBase64) == "" {
			return InvocationDraft{}, invalidInvocation("caller_signature.signature_base64 is required", nil)
		}
		b.callerSignature = &signature
	}
	return InvocationDraft{
		callerURA:       b.callerURA,
		calleeURA:       b.calleeURA,
		descriptorRef:   b.descriptorRef,
		subjectURA:      b.subjectURA,
		nonceBase64:     b.nonceBase64,
		causalContext:   copyMap(b.causalContext),
		args:            b.args,
		argumentsBase64: b.argumentsBase64,
		contentType:     b.contentType,
		metadata:        copyMap(b.metadata),
		callerSignature: copySignature(b.callerSignature),
		hasArgs:         b.hasArgs,
	}, nil
}

func normalizeInvocationSignatureMaterial(signature InvocationSignature) InvocationSignature {
	if strings.TrimSpace(signature.KeyIDHint) == "" {
		signature.KeyIDHint = strings.TrimSpace(signature.SignerPublicKeyBase64)
	}
	return signature
}

func validateInvocationNonceBase64(value string) error {
	raw, err := decodeBase64Field(value, "nonce_base64")
	if err != nil {
		return err
	}
	if len(raw) != 16 {
		return invalidInvocation("nonce_base64 must decode to 16 bytes", nil)
	}
	return nil
}

func validateInvocationArgumentsBase64(value string) error {
	if _, err := decodeBase64Field(value, "arguments_base64"); err != nil {
		return err
	}
	return nil
}

func setRequiredString(
	fields map[string]json.RawMessage,
	name string,
	setter func(string) *InvocationBuilder,
) error {
	raw, ok := fields[name]
	if !ok {
		return invalidInvocation(fmt.Sprintf("%s is required", name), nil)
	}
	var value string
	if err := json.Unmarshal(raw, &value); err != nil {
		return invalidInvocation(fmt.Sprintf("%s must be a string", name), err)
	}
	setter(value)
	return nil
}

func requiredObject(fields map[string]json.RawMessage, name string) (map[string]any, error) {
	raw, ok := fields[name]
	if !ok {
		return nil, invalidInvocation(fmt.Sprintf("%s is required", name), nil)
	}
	var value map[string]any
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil, invalidInvocation(fmt.Sprintf("%s must be an object", name), err)
	}
	return value, nil
}

func rejectUnknownInvocationFields(fields map[string]json.RawMessage) error {
	allowed := map[string]struct{}{
		"caller_ura":       {},
		"callee_ura":       {},
		"descriptor_ref":   {},
		"subject_ura":      {},
		"nonce_base64":     {},
		"causal_context":   {},
		"args":             {},
		"arguments_base64": {},
		"content_type":     {},
		"metadata":         {},
		"caller_signature": {},
	}
	for name := range fields {
		if _, ok := allowed[name]; !ok {
			return invalidInvocation(fmt.Sprintf("%s is not an invocation field", name), nil)
		}
	}
	return nil
}

func invalidInvocation(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "build",
		Retry:     RetryNever,
		Retryable: RetryableForHint(RetryNever),
		Message:   message,
		Cause:     cause,
	}
}

func invalidInvocationHandle(message string) error {
	return &SDKError{
		Code:      ErrInvalidHandle,
		Stage:     "build",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}

func copyMap(source map[string]any) map[string]any {
	if source == nil {
		return nil
	}
	result := make(map[string]any, len(source))
	for key, value := range source {
		result[key] = value
	}
	return result
}

func copySignature(source *InvocationSignature) *InvocationSignature {
	if source == nil {
		return nil
	}
	value := *source
	return &value
}
