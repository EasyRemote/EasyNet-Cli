package easynet

import (
	"context"
	"encoding/json"
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
	Raw        string
	AbilityURA string
	Version    string
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
		projection := ProjectAbilityDescriptor(row)
		if strings.TrimSpace(projection.AbilityURA) == "" ||
			strings.TrimSpace(projection.OwnerURA) == "" ||
			strings.TrimSpace(projection.Name) == "" ||
			strings.TrimSpace(projection.Version) == "" {
			return AbilityDescriptorPage{}, invalidAbilityDescriptor(fmt.Sprintf("ability descriptor row %d is missing identity fields", i), nil)
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
		Raw:        projection.DescriptorRef,
		AbilityURA: projection.AbilityURA,
		Version:    projection.DescriptorVersion,
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
	if projection.Name == "" {
		projection.Name = joinAbilityDescriptorName(
			descriptorString(raw["namespace"]),
			descriptorString(raw["local_name"]),
		)
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
		projection.InputSchema = descriptorMap(schema["input"])
	}
	return projection
}

func joinAbilityDescriptorName(namespace string, localName string) string {
	namespace = strings.TrimSpace(namespace)
	localName = strings.TrimSpace(localName)
	switch {
	case namespace == "":
		return localName
	case localName == "":
		return namespace
	default:
		return namespace + "." + localName
	}
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
	if raw, err := json.Marshal(value); err == nil {
		var mapped map[string]any
		if json.Unmarshal(raw, &mapped) == nil {
			return mapped
		}
	}
	return nil
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
