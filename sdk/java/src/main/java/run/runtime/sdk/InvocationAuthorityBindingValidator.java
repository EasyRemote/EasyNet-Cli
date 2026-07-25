package run.runtime.sdk;

import java.util.List;
import java.util.Map;

final class InvocationAuthorityBindingValidator {
  private final InvocationTuple tuple;
  private final RuntimeAbilityProjection ability;
  private final Map<String, Object> details;

  private InvocationAuthorityBindingValidator(InvocationTuple tuple) {
    this.tuple = tuple;
    this.ability = RuntimeAbilityProjection.fromTuple(tuple);
    this.details =
        Map.of(
            "caller_ura",
            tuple.caller(),
            "callee_ura",
            tuple.callee(),
            "subject_ura",
            tuple.subject(),
            "descriptor_ref",
            tuple.descriptor());
  }

  static void validate(InvocationTuple tuple) {
    Object authority = authorityFromMetadata(tuple.metadata());
    if (authority == null) {
      return;
    }
    InvocationAuthorityBindingValidator validator =
        new InvocationAuthorityBindingValidator(tuple);
    if (authority instanceof DelegationProof delegation) {
      validator.validateDelegation(delegation);
      return;
    }
    validator.validateSession((SessionAuthority) authority);
  }

  private static Object authorityFromMetadata(Map<String, Object> metadata) {
    String delegation =
        AuthoritySupport.authorityMetadataValue(metadata, AuthoritySupport.DELEGATION_METADATA_KEY);
    if (!delegation.isBlank()) {
      return DelegationProof.fromMetadata(delegation);
    }
    String session =
        AuthoritySupport.authorityMetadataValue(
            metadata, AuthoritySupport.SESSION_AUTHORITY_METADATA_KEY);
    if (!session.isBlank()) {
      return SessionAuthority.fromMetadata(session);
    }
    return null;
  }

  private void validateDelegation(DelegationProof proof) {
    require(
        proof.callerURA().trim().equals(tuple.caller().trim()),
        ErrorCode.AUTHORITY_DENIED,
        "delegation authority caller does not match invocation caller_ura");
    require(
        proof.subjectURA().trim().equals(tuple.subject().trim()),
        ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
        "delegation authority subject does not match invocation subject_ura");
    require(
        audienceAdmits(proof.audience(), tuple.callee()),
        ErrorCode.AUTHORITY_DENIED,
        "delegation authority audience does not admit invocation callee_ura");
    require(
        scopesAdmit(proof.scopes(), ability),
        ErrorCode.AUTHORITY_DENIED,
        "delegation authority scopes do not admit invocation ability");
  }

  private void validateSession(SessionAuthority authority) {
    require(
        authority.issuerURA().trim().equals(tuple.caller().trim()),
        ErrorCode.AUTHORITY_DENIED,
        "session authority issuer does not match invocation caller_ura");
    require(
        authority.calleeURA().trim().equals(tuple.callee().trim()),
        ErrorCode.AUTHORITY_DENIED,
        "session authority callee does not match invocation callee_ura");
    require(
        AuthoritySupport.sessionAuthorityAdmitsSubject(authority, tuple.subject()),
        ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
        "session authority subject does not admit invocation subject_ura");
    require(
        audienceAdmits(authority.audience(), tuple.callee()),
        ErrorCode.AUTHORITY_DENIED,
        "session authority audience does not admit invocation callee_ura");
    require(
        listAdmits(authority.allowedActions(), "invoke"),
        ErrorCode.AUTHORITY_DENIED,
        "session authority allowed_actions do not admit invoke");
    require(
        scopesAdmit(authority.allowedFollowupAbilities(), ability),
        ErrorCode.AUTHORITY_DENIED,
        "session authority allowed_followup_abilities do not admit invocation ability");
    require(
        scopesAdmit(authority.scopes(), ability),
        ErrorCode.AUTHORITY_DENIED,
        "session authority scopes do not admit invocation ability");
  }

  private void require(boolean condition, ErrorCode code, String message) {
    if (!condition) {
      throw AuthoritySupport.authorityBindingError(code, message, details);
    }
  }

  private static boolean audienceAdmits(String audience, String calleeURA) {
    String pattern = audience.trim();
    String callee = calleeURA.trim();
    return "*".equals(pattern)
        || pattern.equals(callee)
        || (pattern.endsWith("/") && callee.startsWith(pattern));
  }

  private static boolean scopesAdmit(List<String> patterns, RuntimeAbilityProjection ability) {
    for (String pattern : patterns) {
      if (scopeMatches(pattern, ability.publicName())
          || scopeMatches(pattern, ability.abilityURA())
          || scopeMatches(pattern, ability.wire())) {
        return true;
      }
    }
    return false;
  }

  private static boolean listAdmits(List<String> patterns, String value) {
    for (String pattern : patterns) {
      if (scopeMatches(pattern, value)) {
        return true;
      }
    }
    return false;
  }

  private static boolean scopeMatches(String pattern, String value) {
    String cleanPattern = pattern == null ? "" : pattern.trim();
    String cleanValue = value == null ? "" : value.trim();
    if (cleanPattern.isBlank() || cleanValue.isBlank()) {
      return false;
    }
    if ("*".equals(cleanPattern)) {
      return true;
    }
    if (cleanPattern.endsWith("*")) {
      String prefix = cleanPattern.substring(0, cleanPattern.length() - 1);
      return !prefix.isBlank() && cleanValue.startsWith(prefix);
    }
    return cleanPattern.equals(cleanValue);
  }

}
