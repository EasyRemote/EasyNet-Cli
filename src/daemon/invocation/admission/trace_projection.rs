// EasyNet CLI — RFC-014 trace redaction helpers
// ==============================================

use super::decision::{AbilityCallTrace, ChildFailureClass, RedactionReason};

pub struct TraceProjector;

impl TraceProjector {
    #[must_use]
    pub fn redact_child_edge(
        mut trace: AbilityCallTrace,
        reason: RedactionReason,
    ) -> AbilityCallTrace {
        trace.caller_ura.clear();
        trace.callee_ura.clear();
        trace.subject_ura.clear();
        trace.ability_ura.clear();
        trace.route_ref = None;
        trace.execution_host_ura = None;
        trace.rejector_ura = None;
        trace.signature_decision = None;
        trace.policy_decision = None;
        trace.authority_proof_id = None;
        trace.redacted = true;
        trace.child_failure_class = Some(ChildFailureClass::DownstreamDependencyDenied);
        trace.redaction_reason = Some(reason);
        trace.children = trace
            .children
            .into_iter()
            .map(|child| Self::redact_child_edge(child, reason))
            .collect();
        trace
    }
}
