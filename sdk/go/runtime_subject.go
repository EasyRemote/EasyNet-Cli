package easynet

import (
	"context"
	"fmt"
	"strings"
)

func descriptorBoundSubjectURA(ctx context.Context, addressing Addressing, subjectURA string, abilityName string) (string, error) {
	subjectURA = strings.TrimSpace(subjectURA)
	if subjectURA == "" {
		return "", invalidProfilePayload(addressingProfile, "subject_ura is required", nil)
	}
	parts, err := ParseURAParts(subjectURA)
	if err != nil {
		return "", invalidProfilePayload(addressingProfile, fmt.Sprintf("subject_ura is not a valid URA: %v", err), err)
	}
	switch parts.Kind {
	case URAKindAgent, URAKindAbility, URAKindDevice, URAKindResource:
		return subjectURA, nil
	case URAKindUser, URAKindAuthority:
		if addressing == nil {
			return "", invalidProfileClient(addressingProfile, "addressing provider is required for descriptor-bound subject projection")
		}
		return addressing.DescriptorBoundResourceSubjectURA(ctx, subjectURA, "invoke/"+strings.TrimSpace(abilityName))
	default:
		return "", invalidProfilePayload(addressingProfile, fmt.Sprintf("subject_ura kind %q is not descriptor-bound", parts.Kind), nil)
	}
}
