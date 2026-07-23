package easynet

import "strings"

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
