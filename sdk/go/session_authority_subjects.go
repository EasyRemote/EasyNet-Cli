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

// runtimeSessionAuthorityAdmitsSubject is the Go SDK's canonical
// session-authority subject admission predicate. Runtime ability calls and
// invocation-history queries both consume this helper; neither path owns a
// private subject expansion rule.
func runtimeSessionAuthorityAdmitsSubject(
	authority *SessionAuthority,
	subjectURA string,
) bool {
	if authority == nil {
		return false
	}
	if strings.TrimSpace(authority.SubjectURA) == strings.TrimSpace(subjectURA) {
		return true
	}
	parts, err := ParseURAParts(strings.TrimSpace(subjectURA))
	if err != nil || parts.Kind != URAKindResource {
		return false
	}
	ownerID := strings.TrimSpace(parts.OwnerID)
	ownerUserID := strings.TrimSpace(authority.SessionOwnerUserID)
	if ownerUserID == "" {
		return false
	}
	if strings.HasPrefix(ownerID, "user.") && strings.TrimPrefix(ownerID, "user.") == ownerUserID {
		return true
	}
	if !strings.HasPrefix(ownerID, "agent.") {
		return false
	}
	agentOwner := strings.TrimPrefix(ownerID, "agent.")
	userID, _, found := strings.Cut(agentOwner, ".")
	return found && userID == ownerUserID
}
