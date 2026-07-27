package easynet

import "strings"

const runtimeStateReadSubjectPath = "runtime-state/read"
const retiredInvocationHistorySubjectPath = "session/invocation_history"

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

func isRetiredInvocationHistorySubjectURA(subjectURA string) bool {
	parts, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil || parts.Kind != URAKindResource {
		return false
	}
	ownerID := strings.TrimSpace(parts.OwnerID)
	userID := strings.TrimPrefix(ownerID, "user.")
	return strings.HasPrefix(ownerID, "user.") &&
		strings.TrimSpace(userID) != "" &&
		!strings.Contains(userID, ".") &&
		!containsAllZeroPrincipal(userID) &&
		strings.TrimSpace(parts.Path) == retiredInvocationHistorySubjectPath
}
