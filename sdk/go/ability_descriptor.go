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

type runtimeAbilityDescriptorRoute struct {
	listAbility string
	rowsField   string
}

func defaultRuntimeAbilityDescriptorRoute() runtimeAbilityDescriptorRoute {
	return runtimeAbilityDescriptorRoute{
		listAbility: runtimeAbilityDescriptorListRoute,
		rowsField:   "abilities",
	}
}

func newRuntimeAbilityDescriptorRoute(listAbility string) (runtimeAbilityDescriptorRoute, error) {
	route := runtimeAbilityDescriptorRoute{
		listAbility: strings.TrimSpace(listAbility),
		rowsField:   "abilities",
	}
	if route.listAbility == "" {
		return runtimeAbilityDescriptorRoute{}, invalidAbilityDescriptor("descriptor catalog route ability is required", nil)
	}
	return route, nil
}

func (r runtimeAbilityDescriptorRoute) list(ctx context.Context, ability *RuntimeAbilityClient, call RuntimeCallContext, args map[string]any) (map[string]any, error) {
	if strings.TrimSpace(r.listAbility) == "" {
		return nil, invalidAbilityDescriptor("descriptor catalog route ability is required", nil)
	}
	if strings.TrimSpace(r.rowsField) == "" {
		return nil, invalidAbilityDescriptor("descriptor catalog route rows field is required", nil)
	}
	return ability.invokeCatalogueRead(ctx, call, r.listAbility, args)
}

func (r runtimeAbilityDescriptorRoute) rows(output map[string]any) ([]any, error) {
	rawRows, ok := output[r.rowsField].([]any)
	if !ok {
		return nil, invalidAbilityDescriptor("runtime descriptor catalog output must include descriptor rows", nil)
	}
	return rawRows, nil
}

// RuntimeAbilityDescriptorProvider reads the runtime descriptor catalog through
// an explicit provider route and the generic RuntimeAbilityClient.
type RuntimeAbilityDescriptorProvider struct {
	ability *RuntimeAbilityClient
	route   runtimeAbilityDescriptorRoute
}

func NewRuntimeAbilityDescriptorProvider(ability *RuntimeAbilityClient) (*RuntimeAbilityDescriptorProvider, error) {
	return newRuntimeAbilityDescriptorProviderWithRoute(ability, defaultRuntimeAbilityDescriptorRoute())
}

func newRuntimeAbilityDescriptorProviderWithRoute(ability *RuntimeAbilityClient, route runtimeAbilityDescriptorRoute) (*RuntimeAbilityDescriptorProvider, error) {
	if ability == nil {
		return nil, invalidAbilityDescriptor("runtime ability client is required", nil)
	}
	if strings.TrimSpace(route.listAbility) == "" || strings.TrimSpace(route.rowsField) == "" {
		return nil, invalidAbilityDescriptor("descriptor catalog route is incomplete", nil)
	}
	return &RuntimeAbilityDescriptorProvider{ability: ability, route: route}, nil
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
	output, err := p.route.list(ctx, p.ability, request.Call, args)
	if err != nil {
		return AbilityDescriptorPage{}, err
	}
	rawAbilities, err := p.route.rows(output)
	if err != nil {
		return AbilityDescriptorPage{}, err
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
	values := mergeAbilityDescriptorMap(raw)
	projection := AbilityDescriptorProjection{
		AbilityURA:       descriptorString(values["ability_ura"]),
		DescriptorRef:    descriptorString(values["descriptor_ref"]),
		Name:             descriptorString(values["name"]),
		OwnerURA:         descriptorString(values["owner_ura"]),
		Version:          descriptorString(values["descriptor_version"]),
		SchemaHash:       descriptorString(values["schema_hash"]),
		DescriptorHash:   descriptorString(values["descriptor_hash"]),
		CallMode:         descriptorString(values["call_mode"]),
		Class:            descriptorString(values["class"]),
		ReceiptSemantics: descriptorMap(values["receipt_semantics"]),
		Visibility:       descriptorString(values["visibility"]),
		Source:           descriptorString(values["source"]),
		Description:      descriptorString(values["description"]),
		Metadata:         descriptorMap(values["metadata"]),
	}
	if projection.Name == "" {
		projection.Name = joinAbilityDescriptorName(
			descriptorString(values["namespace"]),
			descriptorString(values["local_name"]),
		)
	}
	if hints := descriptorMap(values["hints"]); hints != nil {
		projection.Hints = AbilityDescriptorHints{
			ReadOnly:      descriptorBool(hints["read_only"]),
			Destructive:   descriptorBool(hints["destructive"]),
			Idempotent:    descriptorBool(hints["idempotent"]),
			StreamingOnly: descriptorBool(hints["streaming_only"]),
			BidiOnly:      descriptorBool(hints["bidi_only"]),
		}
	}
	if schema := descriptorMap(values["schema_summary"]); schema != nil {
		projection.SchemaSummary = schema
		projection.InputSchema = descriptorMap(schema["input"])
	}
	return projection
}

func mergeAbilityDescriptorMap(raw map[string]any) map[string]any {
	if raw == nil {
		return map[string]any{}
	}
	nested := descriptorMap(raw["descriptor"])
	if nested == nil {
		return raw
	}
	merged := make(map[string]any, len(nested)+len(raw))
	for key, value := range nested {
		merged[key] = value
	}
	for key, value := range raw {
		if key != "descriptor" {
			merged[key] = value
		}
	}
	return merged
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
