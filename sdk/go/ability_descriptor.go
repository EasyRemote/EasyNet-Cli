package easynet

import (
	"context"
	"fmt"
	"strings"
)

const runtimeAbilityDescriptorListRoute = "meta.list_abilities"

// AbilityDescriptorHints mirrors descriptor tool-annotation booleans that
// consumers may render without parsing ability names.
type AbilityDescriptorHints struct {
	ReadOnly      bool `json:"read_only"`
	Destructive   bool `json:"destructive"`
	Idempotent    bool `json:"idempotent"`
	StreamingOnly bool `json:"streaming_only"`
	BidiOnly      bool `json:"bidi_only"`
}

// AbilityDescriptorProjection is the SDK read-model projection of one
// advertised AbilityDescriptor. Product-specific display conventions remain in
// Metadata; this type owns only generic runtime descriptor fields.
type AbilityDescriptorProjection struct {
	AbilityURA       string
	DescriptorRef    string
	Name             string
	OwnerURA         string
	Version          string
	SchemaHash       string
	DescriptorHash   string
	CallMode         string
	Class            string
	ReceiptSemantics map[string]any
	Visibility       string
	Source           string
	Description      string
	Hints            AbilityDescriptorHints
	SchemaSummary    map[string]any
	InputSchema      map[string]any
	Metadata         map[string]any
}

// AbilityDescriptorRef is the SDK DTO projection of a descriptor identity.
// DescriptorRef grammar and canonicalization are owned by Axon behind the
// runtime identity profile boundary; this value object only carries the
// projected fields.
type AbilityDescriptorRef struct {
	Raw            string
	AbilityURA     string
	Version        string
	DescriptorHash string
	Action         string
}

// AbilityDescriptorListRequest asks the runtime catalog for
// generic AbilityDescriptor rows. The SDK exposes product-neutral filter names
// and lowers them to the provider catalog argument fields.
type AbilityDescriptorListRequest struct {
	Call       RuntimeCallContext `json:"call"`
	Scope      string             `json:"scope,omitempty"`
	OwnerURA   string             `json:"owner_ura,omitempty"`
	AbilityURA string             `json:"ability_ura,omitempty"`
}

type AbilityDescriptorGetRequest struct {
	Call              RuntimeCallContext `json:"call"`
	AbilityURA        string             `json:"ability_ura"`
	DescriptorVersion string             `json:"descriptor_version,omitempty"`
	CallMode          string             `json:"call_mode,omitempty"`
	Scope             string             `json:"scope,omitempty"`
}

type AbilityDescriptorPage struct {
	Descriptors []AbilityDescriptorProjection `json:"descriptors"`
}

// AbilityDescriptorProvider is the product-neutral catalog provider seam.
// Implementations must use runtime facts instead of name-derived governance.
type AbilityDescriptorProvider interface {
	List(context.Context, AbilityDescriptorListRequest) (AbilityDescriptorPage, error)
	Get(context.Context, AbilityDescriptorGetRequest) (AbilityDescriptorProjection, error)
}

type AbilityDescriptorClient struct {
	provider AbilityDescriptorProvider
}

func NewAbilityDescriptorClient(provider AbilityDescriptorProvider) (*AbilityDescriptorClient, error) {
	if provider == nil {
		return nil, invalidAbilityDescriptor("AbilityDescriptor provider is required", nil)
	}
	return &AbilityDescriptorClient{provider: provider}, nil
}

func (c *AbilityDescriptorClient) List(ctx context.Context, request AbilityDescriptorListRequest) (AbilityDescriptorPage, error) {
	if c == nil || c.provider == nil {
		return AbilityDescriptorPage{}, invalidAbilityDescriptor("AbilityDescriptor client is not initialized", nil)
	}
	return c.provider.List(ctx, request)
}

func (c *AbilityDescriptorClient) Get(ctx context.Context, request AbilityDescriptorGetRequest) (AbilityDescriptorProjection, error) {
	if c == nil || c.provider == nil {
		return AbilityDescriptorProjection{}, invalidAbilityDescriptor("AbilityDescriptor client is not initialized", nil)
	}
	return c.provider.Get(ctx, request)
}

// RuntimeAbilityDescriptorProvider reads canonical runtime descriptor catalog
// facts through the generic RuntimeAbilityClient.
type RuntimeAbilityDescriptorProvider struct {
	ability *RuntimeAbilityClient
}

func NewRuntimeAbilityDescriptorProvider(ability *RuntimeAbilityClient) (*RuntimeAbilityDescriptorProvider, error) {
	if ability == nil {
		return nil, invalidAbilityDescriptor("runtime ability client is required", nil)
	}
	return &RuntimeAbilityDescriptorProvider{ability: ability}, nil
}

func (p *RuntimeAbilityDescriptorProvider) List(ctx context.Context, request AbilityDescriptorListRequest) (AbilityDescriptorPage, error) {
	if p == nil || p.ability == nil {
		return AbilityDescriptorPage{}, invalidAbilityDescriptor("runtime AbilityDescriptor provider is not initialized", nil)
	}
	args := map[string]any{}
	if scope := strings.TrimSpace(request.Scope); scope != "" {
		args["scope"] = scope
	}
	if ownerURA := strings.TrimSpace(request.OwnerURA); ownerURA != "" {
		args["owner_ura"] = ownerURA
	}
	if abilityURA := strings.TrimSpace(request.AbilityURA); abilityURA != "" {
		args["ability_ura"] = abilityURA
	}
	output, err := p.ability.invokeCatalogueRead(ctx, request.Call, runtimeAbilityDescriptorListRoute, args)
	if err != nil {
		return AbilityDescriptorPage{}, err
	}
	rawAbilities, ok := output["abilities"].([]any)
	if !ok {
		return AbilityDescriptorPage{}, invalidAbilityDescriptor("runtime descriptor catalog output must include descriptor rows", nil)
	}
	descriptors := make([]AbilityDescriptorProjection, 0, len(rawAbilities))
	for i, raw := range rawAbilities {
		row, ok := raw.(map[string]any)
		if !ok {
			return AbilityDescriptorPage{}, invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d must be an object", i), nil)
		}
		projection, err := projectRuntimeAbilityDescriptor(row, i)
		if err != nil {
			return AbilityDescriptorPage{}, err
		}
		descriptors = append(descriptors, projection)
	}
	return AbilityDescriptorPage{Descriptors: descriptors}, nil
}

func (p *RuntimeAbilityDescriptorProvider) Get(ctx context.Context, request AbilityDescriptorGetRequest) (AbilityDescriptorProjection, error) {
	abilityURA := strings.TrimSpace(request.AbilityURA)
	if abilityURA == "" {
		return AbilityDescriptorProjection{}, invalidAbilityDescriptor("ability_ura is required", nil)
	}
	page, err := p.List(ctx, AbilityDescriptorListRequest{
		Call:       request.Call,
		Scope:      request.Scope,
		AbilityURA: abilityURA,
	})
	if err != nil {
		return AbilityDescriptorProjection{}, err
	}
	version := strings.TrimSpace(request.DescriptorVersion)
	callMode := strings.TrimSpace(request.CallMode)
	matches := make([]AbilityDescriptorProjection, 0, len(page.Descriptors))
	for _, descriptor := range page.Descriptors {
		if descriptor.AbilityURA != abilityURA {
			return AbilityDescriptorProjection{}, invalidAbilityDescriptor("runtime returned descriptor outside requested ability_ura", nil)
		}
		if version != "" && descriptor.Version != version {
			continue
		}
		if callMode != "" && descriptor.CallMode != callMode {
			continue
		}
		matches = append(matches, descriptor)
	}
	switch len(matches) {
	case 0:
		return AbilityDescriptorProjection{}, abilityDescriptorNotFound(abilityURA)
	case 1:
		return matches[0], nil
	default:
		return AbilityDescriptorProjection{}, invalidAbilityDescriptor("ability descriptor selection is ambiguous; specify descriptor_version or call_mode", nil)
	}
}

// ProjectAbilityDescriptorRef projects a DescriptorRef through the
// product-neutral Axon-delegated Addressing seam.
func ProjectAbilityDescriptorRef(ctx context.Context, addressing Addressing, raw string) (AbilityDescriptorRef, error) {
	if addressing == nil {
		return AbilityDescriptorRef{}, invalidProfileClient(addressingProfile, "addressing provider is required for descriptor_ref projection")
	}
	projection, err := addressing.ProjectDescriptorRef(ctx, CanonicalDescriptorRefRequest{DescriptorRef: raw})
	if err != nil {
		return AbilityDescriptorRef{}, err
	}
	if projection.Kind != "descriptor_ref" ||
		strings.TrimSpace(projection.DescriptorRef) == "" ||
		strings.TrimSpace(projection.AbilityURA) == "" ||
		strings.TrimSpace(projection.DescriptorVersion) == "" {
		return AbilityDescriptorRef{}, invalidProfilePayload(addressingProfile, "descriptor_ref projection is incomplete", nil)
	}
	return AbilityDescriptorRef{
		Raw:            projection.DescriptorRef,
		AbilityURA:     projection.AbilityURA,
		Version:        projection.DescriptorVersion,
		DescriptorHash: projection.DescriptorHash,
		Action:         projection.Action,
	}, nil
}

func ProjectAbilityDescriptor(raw map[string]any) AbilityDescriptorProjection {
	projection := AbilityDescriptorProjection{
		AbilityURA:       descriptorString(raw["ability_ura"]),
		DescriptorRef:    descriptorString(raw["descriptor_ref"]),
		Name:             descriptorString(raw["name"]),
		OwnerURA:         descriptorString(raw["owner_ura"]),
		Version:          descriptorString(raw["descriptor_version"]),
		SchemaHash:       descriptorString(raw["schema_hash"]),
		DescriptorHash:   descriptorString(raw["descriptor_hash"]),
		CallMode:         descriptorString(raw["call_mode"]),
		Class:            descriptorString(raw["class"]),
		ReceiptSemantics: descriptorMap(raw["receipt_semantics"]),
		Visibility:       descriptorString(raw["visibility"]),
		Source:           descriptorString(raw["source"]),
		Description:      descriptorString(raw["description"]),
		Metadata:         descriptorMap(raw["metadata"]),
	}
	if hints := descriptorMap(raw["hints"]); hints != nil {
		projection.Hints = AbilityDescriptorHints{
			ReadOnly:      descriptorBool(hints["read_only"]),
			Destructive:   descriptorBool(hints["destructive"]),
			Idempotent:    descriptorBool(hints["idempotent"]),
			StreamingOnly: descriptorBool(hints["streaming_only"]),
			BidiOnly:      descriptorBool(hints["bidi_only"]),
		}
	}
	if schema := descriptorMap(raw["schema_summary"]); schema != nil {
		projection.SchemaSummary = schema
	}
	projection.InputSchema = descriptorMap(raw["input_schema"])
	return projection
}

func projectRuntimeAbilityDescriptor(raw map[string]any, index int) (AbilityDescriptorProjection, error) {
	projection := AbilityDescriptorProjection{
		AbilityURA:       "",
		DescriptorRef:    "",
		Name:             "",
		OwnerURA:         "",
		Version:          "",
		SchemaHash:       "",
		DescriptorHash:   "",
		CallMode:         "",
		Class:            "",
		ReceiptSemantics: nil,
		Visibility:       "",
		Source:           "",
		Description:      "",
		Metadata:         nil,
	}
	var err error
	if projection.AbilityURA, err = requiredDescriptorString(raw, "ability_ura", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.DescriptorRef, err = requiredDescriptorString(raw, "descriptor_ref", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Name, err = requiredDescriptorString(raw, "name", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.OwnerURA, err = requiredDescriptorString(raw, "owner_ura", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Version, err = requiredDescriptorString(raw, "descriptor_version", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.SchemaHash, err = optionalDescriptorString(raw, "schema_hash", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.DescriptorHash, err = optionalDescriptorString(raw, "descriptor_hash", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.CallMode, err = optionalDescriptorString(raw, "call_mode", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Class, err = optionalDescriptorString(raw, "class", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Visibility, err = optionalDescriptorString(raw, "visibility", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Source, err = optionalDescriptorString(raw, "source", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Description, err = optionalDescriptorString(raw, "description", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.ReceiptSemantics, err = optionalDescriptorMapWithErr(raw, "receipt_semantics", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.Metadata, err = optionalDescriptorMapWithErr(raw, "metadata", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.AbilityURA == "" ||
		projection.DescriptorRef == "" ||
		projection.Name == "" ||
		projection.OwnerURA == "" ||
		projection.Version == "" {
		return AbilityDescriptorProjection{}, invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d is missing identity fields", index), nil)
	}
	if hints, err := optionalDescriptorMapWithErr(raw, "hints", index); err != nil {
		return AbilityDescriptorProjection{}, err
	} else if hints != nil {
		projection.Hints = AbilityDescriptorHints{
			ReadOnly:      false,
			Destructive:   false,
			Idempotent:    false,
			StreamingOnly: false,
			BidiOnly:      false,
		}
		if projection.Hints.ReadOnly, err = optionalDescriptorBool(hints, "hints.read_only", index); err != nil {
			return AbilityDescriptorProjection{}, err
		}
		if projection.Hints.Destructive, err = optionalDescriptorBool(hints, "hints.destructive", index); err != nil {
			return AbilityDescriptorProjection{}, err
		}
		if projection.Hints.Idempotent, err = optionalDescriptorBool(hints, "hints.idempotent", index); err != nil {
			return AbilityDescriptorProjection{}, err
		}
		if projection.Hints.StreamingOnly, err = optionalDescriptorBool(hints, "hints.streaming_only", index); err != nil {
			return AbilityDescriptorProjection{}, err
		}
		if projection.Hints.BidiOnly, err = optionalDescriptorBool(hints, "hints.bidi_only", index); err != nil {
			return AbilityDescriptorProjection{}, err
		}
	}
	if projection.SchemaSummary, err = optionalDescriptorMapWithErr(raw, "schema_summary", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	if projection.InputSchema, err = optionalDescriptorMapWithErr(raw, "input_schema", index); err != nil {
		return AbilityDescriptorProjection{}, err
	}
	return projection, nil
}

func descriptorString(value any) string {
	if typed, ok := value.(string); ok {
		return typed
	}
	return ""
}

func descriptorBool(value any) bool {
	typed, ok := value.(bool)
	return ok && typed
}

func descriptorMap(value any) map[string]any {
	typed, ok := value.(map[string]any)
	if ok {
		return typed
	}
	if typed, ok := value.(map[string]string); ok {
		mapped := make(map[string]any, len(typed))
		for key, raw := range typed {
			mapped[key] = raw
		}
		return mapped
	}
	return nil
}

func requiredDescriptorString(raw map[string]any, field string, index int) (string, error) {
	value, ok := raw[field]
	if !ok || value == nil {
		return "", nil
	}
	typed, ok := value.(string)
	if !ok {
		return "", invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d field %s must be a string", index, field), nil)
	}
	return strings.TrimSpace(typed), nil
}

func optionalDescriptorString(raw map[string]any, field string, index int) (string, error) {
	value, ok := raw[field]
	if !ok || value == nil {
		return "", nil
	}
	typed, ok := value.(string)
	if !ok {
		return "", invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d field %s must be a string", index, field), nil)
	}
	return strings.TrimSpace(typed), nil
}

func optionalDescriptorMapWithErr(raw map[string]any, field string, index int) (map[string]any, error) {
	value, ok := raw[field]
	if !ok || value == nil {
		return nil, nil
	}
	mapped := descriptorMap(value)
	if mapped == nil {
		return nil, invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d field %s must be an object", index, field), nil)
	}
	return mapped, nil
}

func optionalDescriptorBool(raw map[string]any, field string, index int) (bool, error) {
	key := field
	if dot := strings.LastIndex(field, "."); dot >= 0 {
		key = field[dot+1:]
	}
	value, ok := raw[key]
	if !ok || value == nil {
		return false, nil
	}
	typed, ok := value.(bool)
	if !ok {
		return false, invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d field %s must be a boolean", index, field), nil)
	}
	return typed, nil
}

func invalidAbilityDescriptor(message string, cause error) error {
	return &SDKError{
		Code:      ErrInvalidArgument,
		Stage:     "ability_descriptor",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
		Cause:     cause,
	}
}

func abilityDescriptorNotFound(abilityURA string) error {
	return &SDKError{
		Code:      ErrDescriptorNotFound,
		Stage:     "ability_descriptor",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "ability descriptor not found",
		Details:   map[string]any{"ability_ura": abilityURA},
	}
}
