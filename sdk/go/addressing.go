package easynet

import (
	"context"
	"fmt"
	"strings"
)

const addressingProfile = "addressing"

// Addressing is the product-neutral SDK seam for canonical URA and
// AbilityDescriptorRef operations. Runtime transports depend on this narrow
// contract; signing-key lifecycle is supplied by a separate provider.
type Addressing interface {
	ProjectDescriptorRef(context.Context, DescriptorRefRequest) (AddressingProjection, error)
	ProjectIdentity(context.Context, URAProjectionRequest) (AddressingProjection, error)
	BuildURA(context.Context, URABuildRequest) (AddressingProjection, error)
	BuildDescriptorRef(context.Context, DescriptorRefBuildRequest) (AddressingProjection, error)
	OwnerAbilityURA(context.Context, string, string) (string, error)
	ResourceURA(context.Context, string, string) (string, error)
	OwnerURAForAbility(context.Context, string) (string, error)
	OwnerAbilityDescriptorRef(context.Context, string, string, string) (string, error)
	CanonicalAbilityDescriptorRef(context.Context, string, string) (string, error)
	AbilityURAFromDescriptorRef(context.Context, string) (string, error)
	DescriptorBoundResourceSubjectURA(context.Context, string, string) (string, error)
}

type DescriptorRefRequest struct {
	DescriptorRef string         `json:"descriptor_ref"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

type URAProjectionRequest struct {
	URA      string         `json:"ura,omitempty"`
	Kind     string         `json:"kind,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

type URABuildRequest struct {
	Kind        string         `json:"kind"`
	Realm       string         `json:"realm,omitempty"`
	UserID      string         `json:"user_id,omitempty"`
	DeviceID    string         `json:"device_id,omitempty"`
	AgentID     string         `json:"agent_id,omitempty"`
	OwnerKind   string         `json:"owner_kind,omitempty"`
	OwnerURA    string         `json:"owner_ura,omitempty"`
	AbilityName string         `json:"ability_name,omitempty"`
	Path        string         `json:"path,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type DescriptorRefBuildRequest struct {
	AbilityURA        string         `json:"ability_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

type AddressingProjection struct {
	Kind              string         `json:"kind"`
	Valid             bool           `json:"valid"`
	URA               string         `json:"ura,omitempty"`
	Realm             string         `json:"realm,omitempty"`
	DisplayID         string         `json:"display_id,omitempty"`
	DescriptorRef     string         `json:"descriptor_ref,omitempty"`
	AbilityURA        string         `json:"ability_ura,omitempty"`
	DescriptorVersion string         `json:"descriptor_version,omitempty"`
	Profile           string         `json:"profile"`
	Components        map[string]any `json:"components"`
	Metadata          map[string]any `json:"metadata"`
}

// CanonicalAddressing delegates protocol grammar to the Axon Go SDK through
// the package-level typed URA and descriptor helpers. It has no daemon,
// product-profile, service-locator, or signing-key dependency.
type CanonicalAddressing struct{}

// NewCanonicalAddressing creates the process-local canonical Addressing
// provider used by native Runtime handles and product integrations.
func NewCanonicalAddressing() *CanonicalAddressing {
	return &CanonicalAddressing{}
}

func (a *CanonicalAddressing) ProjectDescriptorRef(ctx context.Context, req DescriptorRefRequest) (AddressingProjection, error) {
	if err := requireAddressing(ctx, a); err != nil {
		return AddressingProjection{}, err
	}
	ref, err := ParseAbilityDescriptorRef(req.DescriptorRef)
	if err != nil {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("project descriptor_ref: %v", err), err)
	}
	return descriptorRefProjection(ref), nil
}

func (a *CanonicalAddressing) ProjectIdentity(ctx context.Context, req URAProjectionRequest) (AddressingProjection, error) {
	if err := requireAddressing(ctx, a); err != nil {
		return AddressingProjection{}, err
	}
	if strings.TrimSpace(req.URA) == "" {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, "ura is required for addressing projection", nil)
	}
	if req.Kind != "" {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, "kind is not an addressing projection selector; use BuildURA", nil)
	}
	parts, err := ParseURAParts(req.URA)
	if err != nil {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("project URA: %v", err), err)
	}
	projection := AddressingProjection{
		Kind:       string(parts.Kind),
		Valid:      true,
		URA:        parts.Raw,
		Realm:      parts.Realm,
		DisplayID:  DisplayID(parts.Raw),
		Profile:    UraProfileEasynetStrictV2,
		Components: addressingComponents(parts),
		Metadata:   addressingMetadata(),
	}
	if parts.Kind == URAKindAbility {
		projection.AbilityURA = parts.Raw
	}
	return projection, nil
}

func (a *CanonicalAddressing) BuildURA(ctx context.Context, req URABuildRequest) (AddressingProjection, error) {
	if err := requireAddressing(ctx, a); err != nil {
		return AddressingProjection{}, err
	}
	kind := strings.TrimSpace(req.Kind)
	var raw string
	switch kind {
	case "user":
		raw = UserURA(req.Realm, req.UserID)
	case "device":
		raw = DeviceURA(req.Realm, req.DeviceID)
	case "agent":
		ownerKind := strings.TrimSpace(req.OwnerKind)
		switch ownerKind {
		case "", "user":
			raw = AgentURA(req.Realm, req.UserID, req.AgentID)
		case "device":
			raw = DeviceAgentURA(req.Realm, req.DeviceID, req.AgentID)
		default:
			return AddressingProjection{}, invalidProfilePayload(
				addressingProfile,
				fmt.Sprintf("unsupported agent owner_kind %q", req.OwnerKind),
				nil,
			)
		}
	case "hub":
		raw = HubURA(req.Realm)
	case "ability":
		raw = OwnerAbilityURA(req.OwnerURA, req.AbilityName)
	case "resource":
		var err error
		raw, err = canonicalOwnerResourceURA(req.OwnerURA, req.Path)
		if err != nil {
			return AddressingProjection{}, err
		}
	default:
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("unsupported URA build kind %q", req.Kind), nil)
	}
	if strings.TrimSpace(raw) == "" {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("cannot build %s URA from supplied fields", kind), nil)
	}
	projection, err := a.ProjectIdentity(ctx, URAProjectionRequest{URA: raw})
	if err != nil {
		return AddressingProjection{}, err
	}
	if projection.Kind != kind {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("built URA kind %q does not match %q", projection.Kind, kind), nil)
	}
	return projection, nil
}

func (a *CanonicalAddressing) BuildDescriptorRef(ctx context.Context, req DescriptorRefBuildRequest) (AddressingProjection, error) {
	if err := requireAddressing(ctx, a); err != nil {
		return AddressingProjection{}, err
	}
	abilityURA := strings.TrimSpace(req.AbilityURA)
	version := strings.TrimSpace(req.DescriptorVersion)
	if abilityURA == "" || version == "" {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, "ability_ura and descriptor_version are required", nil)
	}
	ref, err := ParseAbilityDescriptorRef(abilityURA + "@" + version)
	if err != nil {
		return AddressingProjection{}, invalidProfilePayload(addressingProfile, fmt.Sprintf("build descriptor_ref: %v", err), err)
	}
	return descriptorRefProjection(ref), nil
}

func (a *CanonicalAddressing) OwnerAbilityURA(ctx context.Context, ownerURA string, abilityName string) (string, error) {
	projection, err := a.BuildURA(ctx, URABuildRequest{Kind: "ability", OwnerURA: ownerURA, AbilityName: abilityName})
	if err != nil {
		return "", err
	}
	return projection.URA, nil
}

func (a *CanonicalAddressing) ResourceURA(ctx context.Context, ownerURA string, path string) (string, error) {
	projection, err := a.BuildURA(ctx, URABuildRequest{Kind: "resource", OwnerURA: ownerURA, Path: path})
	if err != nil {
		return "", err
	}
	return projection.URA, nil
}

func (a *CanonicalAddressing) OwnerURAForAbility(ctx context.Context, abilityURA string) (string, error) {
	projection, err := a.ProjectIdentity(ctx, URAProjectionRequest{URA: abilityURA})
	if err != nil {
		return "", err
	}
	if projection.Kind != "ability" {
		return "", invalidProfilePayload(addressingProfile, "ability_ura must project to an ability", nil)
	}
	ownerURA, ok := projection.Components["owner_ura"].(string)
	if !ok || ownerURA == "" {
		return "", invalidProfilePayload(addressingProfile, "ability projection missing owner_ura", nil)
	}
	return ownerURA, nil
}

func (a *CanonicalAddressing) OwnerAbilityDescriptorRef(ctx context.Context, ownerURA string, abilityName string, descriptorVersion string) (string, error) {
	abilityURA, err := a.OwnerAbilityURA(ctx, ownerURA, abilityName)
	if err != nil {
		return "", err
	}
	return a.CanonicalAbilityDescriptorRef(ctx, abilityURA, descriptorVersion)
}

func (a *CanonicalAddressing) CanonicalAbilityDescriptorRef(ctx context.Context, value string, descriptorVersion string) (string, error) {
	var projection AddressingProjection
	var err error
	if version := strings.TrimSpace(descriptorVersion); version != "" {
		projection, err = a.BuildDescriptorRef(ctx, DescriptorRefBuildRequest{AbilityURA: value, DescriptorVersion: version})
	} else {
		projection, err = a.ProjectDescriptorRef(ctx, DescriptorRefRequest{DescriptorRef: value})
	}
	if err != nil {
		return "", err
	}
	return projection.DescriptorRef, nil
}

func (a *CanonicalAddressing) AbilityURAFromDescriptorRef(ctx context.Context, descriptorRef string) (string, error) {
	projection, err := a.ProjectDescriptorRef(ctx, DescriptorRefRequest{DescriptorRef: descriptorRef})
	if err != nil {
		return "", err
	}
	return projection.AbilityURA, nil
}

func (a *CanonicalAddressing) DescriptorBoundResourceSubjectURA(ctx context.Context, ownerURA string, path string) (string, error) {
	return a.ResourceURA(ctx, ownerURA, path)
}

func requireAddressing(ctx context.Context, addressing *CanonicalAddressing) error {
	if addressing == nil {
		return invalidProfileClient(addressingProfile, "addressing provider is not initialized")
	}
	if ctx == nil {
		return invalidProfileClient(addressingProfile, "context is required")
	}
	return nil
}

func descriptorRefProjection(ref AbilityDescriptorRef) AddressingProjection {
	parts, _ := ParseURAParts(ref.AbilityURA)
	publicName := parts.AbilityID
	return AddressingProjection{
		Kind:              "descriptor_ref",
		Valid:             true,
		URA:               ref.AbilityURA,
		DescriptorRef:     ref.Raw,
		AbilityURA:        ref.AbilityURA,
		DescriptorVersion: ref.Version,
		Profile:           UraProfileEasynetStrictV2,
		Components: map[string]any{
			"ability_ura":            ref.AbilityURA,
			"descriptor_version":     ref.Version,
			"owner_ura":              ownerURAFromAbilityParts(parts),
			"owner_kind":             string(parts.AbilityOwner.Kind),
			"public_name":            publicName,
			"local_registry_ability": publicName,
		},
		Metadata: addressingMetadata(),
	}
}

func addressingComponents(parts ParsedURA) map[string]any {
	components := map[string]any{"realm": parts.Realm}
	switch parts.Kind {
	case URAKindUser:
		components["user_id"] = parts.UserID
	case URAKindDevice:
		components["device_id"] = parts.DeviceID
	case URAKindAgent:
		if parts.DeviceID != "" {
			components["owner_kind"] = "device"
			components["device_id"] = parts.DeviceID
		} else {
			components["owner_kind"] = "user"
			components["user_id"] = parts.UserID
		}
		components["agent_id"] = parts.AgentID
	case URAKindAbility:
		publicName := parts.AbilityID
		components["owner_ura"] = ownerURAFromAbilityParts(parts)
		components["owner_kind"] = string(parts.AbilityOwner.Kind)
		components["ability_name"] = parts.AbilityID
		components["public_name"] = publicName
		components["local_registry_ability"] = publicName
		components["namespace"] = parts.AbilityNamespace
		components["local_name"] = parts.AbilityLocalName
	case URAKindResource:
		components["owner_id"] = parts.OwnerID
		components["path"] = parts.Path
	}
	return components
}

func ownerURAFromAbilityParts(parts ParsedURA) string {
	switch parts.AbilityOwner.Kind {
	case AbilityOwnerAgent:
		return AgentURA(parts.Realm, parts.AbilityOwner.UserID, parts.AbilityOwner.AgentID)
	case AbilityOwnerDevice:
		return DeviceURA(parts.Realm, parts.AbilityOwner.DeviceID)
	case AbilityOwnerHub:
		return HubURA(parts.Realm)
	default:
		return ""
	}
}

func canonicalOwnerResourceURA(ownerURA string, path string) (string, error) {
	parts, err := ParseURAParts(strings.TrimSpace(ownerURA))
	if err != nil {
		return "", invalidProfilePayload(addressingProfile, fmt.Sprintf("parse resource owner_ura: %v", err), err)
	}
	var ownerID string
	switch parts.Kind {
	case URAKindUser:
		ownerID = "user." + parts.UserID
	case URAKindDevice:
		ownerID = "device." + parts.DeviceID
	case URAKindAgent:
		if parts.DeviceID != "" {
			ownerID = "agent.device." + parts.DeviceID + "." + parts.AgentID
		} else {
			ownerID = "agent." + parts.UserID + "." + parts.AgentID
		}
	case URAKindHub:
		ownerID = "hub"
	default:
		return "", invalidProfilePayload(addressingProfile, fmt.Sprintf("owner_ura kind %q cannot own protocol resources", parts.Kind), nil)
	}
	raw := ResourceDotURA(parts.Realm, ownerID, strings.TrimPrefix(strings.TrimSpace(path), "/"))
	if _, err := ParseURA(raw); err != nil {
		return "", invalidProfilePayload(addressingProfile, fmt.Sprintf("build resource URA: %v", err), err)
	}
	return raw, nil
}

func addressingMetadata() map[string]any {
	return map[string]any{
		"grammar_owner": "axon",
		"source":        "axon_go_sdk",
	}
}

var _ Addressing = (*CanonicalAddressing)(nil)
