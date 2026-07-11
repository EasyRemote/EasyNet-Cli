package easynet

import (
	"context"
	"strings"
)

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
	AbilityURA  string
	Name        string
	OwnerURA    string
	Source      string
	Description string
	Hints       AbilityDescriptorHints
	InputSchema map[string]any
	Metadata    map[string]any
}

// AbilityDescriptorRef is the SDK DTO projection of a descriptor identity.
// DescriptorRef grammar and canonicalization are owned by Axon behind the
// daemon/Identity profile boundary; this value object only carries the
// projected fields.
type AbilityDescriptorRef struct {
	Raw        string
	AbilityURA string
	Version    string
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
		AbilityURA:  descriptorString(values["ability_ura"]),
		Name:        descriptorString(values["name"]),
		OwnerURA:    descriptorString(values["owner_ura"]),
		Source:      descriptorString(values["source"]),
		Description: descriptorString(values["description"]),
		Metadata:    descriptorMap(values["metadata"]),
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
	return nil
}
