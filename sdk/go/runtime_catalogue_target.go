package easynet

import "strings"

type runtimeCatalogueReadTarget struct {
	calleeURA  string
	subjectURA string
}

func newRuntimeCatalogueReadTarget(calleeURA string, subjectURA string, abilityName string, provider string) (runtimeCatalogueReadTarget, error) {
	calleeURA = strings.TrimSpace(calleeURA)
	subjectURA = strings.TrimSpace(subjectURA)
	if strings.TrimSpace(provider) != runtimeAbilityDescriptorProvider ||
		runtimeGovernanceDescriptorProviderForAbility(abilityName) != runtimeAbilityDescriptorProvider {
		return runtimeCatalogueReadTarget{calleeURA: calleeURA, subjectURA: subjectURA}, nil
	}

	projectedCallee, err := runtimeCatalogueReadCalleeURA(calleeURA)
	if err != nil {
		return runtimeCatalogueReadTarget{}, err
	}
	projectedSubject, err := runtimeCatalogueReadSubjectURA(subjectURA, projectedCallee)
	if err != nil {
		return runtimeCatalogueReadTarget{}, err
	}
	return runtimeCatalogueReadTarget{
		calleeURA:  projectedCallee,
		subjectURA: projectedSubject,
	}, nil
}

func runtimeCatalogueReadCalleeURA(calleeURA string) (string, error) {
	calleeURA = strings.TrimSpace(calleeURA)
	parts, err := ParseURAParts(calleeURA)
	if err != nil {
		return "", invalidRuntimeClient("descriptor_ref provider callee_ura must be canonical: " + err.Error())
	}
	switch parts.Kind {
	case URAKindDevice:
		if strings.TrimSpace(parts.DeviceID) != "" {
			return DeviceAgentURA(parts.Realm, parts.DeviceID, "runtime-introspection"), nil
		}
	case URAKindAgent:
		if strings.TrimSpace(parts.DeviceID) != "" {
			return DeviceAgentURA(parts.Realm, parts.DeviceID, "runtime-introspection"), nil
		}
	}
	return calleeURA, nil
}

func runtimeCatalogueReadSubjectURA(subjectURA string, calleeURA string) (string, error) {
	subjectURA = strings.TrimSpace(subjectURA)
	calleeURA = strings.TrimSpace(calleeURA)
	if subjectURA == "" {
		return "", invalidInvocation("runtime governance read subject_ura is required", nil)
	}
	if subject, err := RuntimeGovernanceReadSubjectURA(subjectURA, calleeURA); err == nil {
		return subject, nil
	} else if subject, ok := runtimeCatalogueResourceOwnerSubjectURA(subjectURA, calleeURA); ok {
		return subject, nil
	} else {
		return "", err
	}
}

func runtimeOwnerReadSubjectURA(calleeURA string) (string, error) {
	parts, err := ParseURAParts(strings.TrimSpace(calleeURA))
	if err != nil {
		return "", invalidInvocation("runtime governance read callee_ura must be canonical", err)
	}
	switch parts.Kind {
	case URAKindAuthority:
		return AuthorityURA(parts.Realm), nil
	case URAKindDevice:
		if strings.TrimSpace(parts.DeviceID) != "" {
			return DeviceURA(parts.Realm, parts.DeviceID), nil
		}
	case URAKindAgent:
		if strings.TrimSpace(parts.DeviceID) != "" {
			return DeviceURA(parts.Realm, parts.DeviceID), nil
		}
	}
	return "", invalidInvocation("runtime governance read callee_ura has no runtime-owner subject", nil)
}

func runtimeCatalogueResourceOwnerSubjectURA(subjectURA string, calleeURA string) (string, bool) {
	subject, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil || subject.Kind != URAKindResource {
		return "", false
	}
	ownerID := strings.TrimSpace(subject.OwnerID)
	deviceID := strings.TrimPrefix(ownerID, "device.")
	if !strings.HasPrefix(ownerID, "device.") ||
		strings.TrimSpace(deviceID) == "" ||
		strings.Contains(deviceID, "/") ||
		strings.Contains(deviceID, ".") {
		return "", false
	}
	ownerSubject := DeviceURA(subject.Realm, deviceID)
	calleeOwner, err := runtimeOwnerReadSubjectURA(calleeURA)
	if err != nil {
		return "", false
	}
	if ownerSubject != calleeOwner {
		return "", false
	}
	return ownerSubject, true
}
