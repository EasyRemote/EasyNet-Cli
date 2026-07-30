package easynet

import "strings"

const runtimeStateReadSubjectPath = "runtime-state/read"

// RuntimeStateReadSubjectURA builds the canonical subject for runtime-state
// read projections owned by one authenticated user.
//
// Products pass realm/user identity into this helper instead of defaulting
// read-only invocations to the target device. Admission still validates the
// resulting tuple; this helper only centralizes the product-neutral subject
// projection used by history, catalogue, and status reads.
func RuntimeStateReadSubjectURA(realm string, userID string) (string, error) {
	realm = strings.TrimSpace(realm)
	userID = strings.TrimSpace(userID)
	if realm == "" {
		return "", invalidInvocation("runtime-state read subject realm is required", nil)
	}
	if userID == "" {
		return "", invalidInvocation("runtime-state read subject user_id is required", nil)
	}
	if containsAllZeroPrincipal(userID) {
		return "", invalidInvocation("runtime-state read subject user_id must not be all-zero", nil)
	}
	subject := ResourceDotURA(realm, "user."+userID, runtimeStateReadSubjectPath)
	if _, err := ParseURA(subject); err != nil {
		return "", invalidInvocation("runtime-state read subject_ura must be canonical", err)
	}
	return subject, nil
}

func isRuntimeStateReadSubjectURA(subjectURA string) bool {
	parts, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil || parts.Kind != URAKindResource {
		return false
	}
	ownerID := strings.TrimSpace(parts.OwnerID)
	userID := strings.TrimPrefix(ownerID, "user.")
	return strings.HasPrefix(ownerID, "user.") &&
		strings.TrimSpace(userID) != "" &&
		!containsAllZeroPrincipal(userID) &&
		strings.TrimSpace(parts.Path) == runtimeStateReadSubjectPath
}

func isRuntimeGovernanceReadSubjectURA(subjectURA string, calleeURA string) bool {
	subjectURA = strings.TrimSpace(subjectURA)
	calleeURA = strings.TrimSpace(calleeURA)
	if isRuntimeStateReadSubjectURA(subjectURA) {
		return true
	}
	subject, subjectErr := ParseURAParts(subjectURA)
	callee, calleeErr := ParseURAParts(calleeURA)
	if subjectErr != nil || calleeErr != nil {
		return false
	}
	return (subject.Kind == URAKindAuthority || subject.Kind == URAKindDevice) &&
		subject.Kind == callee.Kind &&
		subject.Realm == callee.Realm &&
		subjectURA == calleeURA
}

// RuntimeGovernanceReadSubjectURA projects a business subject into the
// canonical runtime governance-read subject accepted by provider-backed
// descriptor and receipt-history reads.
//
// Runtime governance reads are not product actions against the target device.
// User-, agent-, ability-, and user-owned resource subjects are projected to
// the user's runtime-state/read resource; a runtime owner subject is admitted
// only when it exactly matches the selected callee runtime owner.
func RuntimeGovernanceReadSubjectURA(subjectURA string, calleeURA string) (string, error) {
	subjectURA = strings.TrimSpace(subjectURA)
	calleeURA = strings.TrimSpace(calleeURA)
	if subjectURA == "" {
		return "", invalidInvocation("runtime governance read subject_ura is required", nil)
	}
	if isRuntimeGovernanceReadSubjectURA(subjectURA, calleeURA) {
		return subjectURA, nil
	}
	parts, err := ParseURAParts(subjectURA)
	if err != nil {
		return "", invalidInvocation("runtime governance read subject_ura must be canonical", err)
	}
	switch parts.Kind {
	case URAKindUser:
		return RuntimeStateReadSubjectURA(parts.Realm, parts.UserID)
	case URAKindAgent:
		if strings.TrimSpace(parts.UserID) == "" {
			break
		}
		return RuntimeStateReadSubjectURA(parts.Realm, parts.UserID)
	case URAKindAbility:
		userID := strings.TrimSpace(parts.AbilityOwner.UserID)
		if userID == "" {
			userID = strings.TrimSpace(parts.UserID)
		}
		if userID != "" {
			return RuntimeStateReadSubjectURA(parts.Realm, userID)
		}
	case URAKindResource:
		ownerID := strings.TrimSpace(parts.OwnerID)
		if strings.HasPrefix(ownerID, "user.") {
			userID := strings.TrimPrefix(ownerID, "user.")
			if strings.TrimSpace(userID) != "" && !strings.Contains(userID, ".") && !strings.Contains(userID, "/") {
				return RuntimeStateReadSubjectURA(parts.Realm, userID)
			}
		}
	}
	return "", invalidInvocation("runtime governance read subject_ura must be a runtime owner or user-owned runtime-state read subject", nil)
}

func runtimeGovernanceReadSubjectURA(subjectURA string, calleeURA string) (string, error) {
	return RuntimeGovernanceReadSubjectURA(subjectURA, calleeURA)
}
