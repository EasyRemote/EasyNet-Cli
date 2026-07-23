package run.runtime.sdk;

import java.util.List;
import java.util.Map;

final class InvocationAuthorityBindingValidator {
  private final InvocationTuple tuple;
  private final AbilityView ability;
  private final Map<String, Object> details;

  private InvocationAuthorityBindingValidator(InvocationTuple tuple) {
    this.tuple = tuple;
    this.ability = AbilityView.fromTuple(tuple);
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
        sessionAuthorityAdmitsSubject(authority, tuple.subject()),
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

  private static boolean scopesAdmit(List<String> patterns, AbilityView ability) {
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

  private static boolean sessionAuthorityAdmitsSubject(
      SessionAuthority authority, String subjectURA) {
    String subject = subjectURA.trim();
    if (authority.subjectURA().trim().equals(subject)) {
      return true;
    }
    String owner = resourceOwnerID(subject);
    if (owner.isBlank()) {
      return false;
    }
    String ownerUserID = authority.sessionOwnerUserID().trim();
    if (ownerUserID.isBlank()) {
      return false;
    }
    if (owner.equals("user." + ownerUserID)) {
      return true;
    }
    if (!owner.startsWith("agent.")) {
      return false;
    }
    String rest = owner.substring("agent.".length());
    int dot = rest.indexOf('.');
    return dot > 0 && rest.substring(0, dot).equals(ownerUserID);
  }

  private static String resourceOwnerID(String ura) {
    String marker = "/resource/";
    int index = ura.indexOf(marker);
    if (index < 0) {
      return "";
    }
    String rest = ura.substring(index + marker.length());
    int slash = rest.indexOf('/');
    return (slash < 0 ? rest : rest.substring(0, slash)).trim();
  }

  private record AbilityView(String wire, String abilityURA, String publicName) {
    static AbilityView fromTuple(InvocationTuple tuple) {
      String abilityURA = descriptorAbilityURA(tuple.descriptor());
      String wire = descriptorWireAbility(abilityURA);
      String publicName =
          publicAbilityName(tuple.callee(), abilityURA.isBlank() ? wire : abilityURA);
      return new AbilityView(wire, abilityURA, publicName);
    }

    private static String descriptorAbilityURA(String descriptorRef) {
      String clean = descriptorRef == null ? "" : descriptorRef.trim();
      int hash = clean.indexOf('#');
      int bang = clean.indexOf('!');
      int limit = clean.length();
      if (hash >= 0) {
        limit = Math.min(limit, hash);
      }
      if (bang >= 0) {
        limit = Math.min(limit, bang);
      }
      String withoutMode = clean.substring(0, limit);
      int version = withoutMode.lastIndexOf('@');
      return (version >= 0 ? withoutMode.substring(0, version) : withoutMode).trim();
    }

    private static String descriptorWireAbility(String abilityURA) {
      String marker = "/ability/";
      int index = abilityURA.indexOf(marker);
      return (index >= 0 ? abilityURA.substring(index + marker.length()) : abilityURA).trim();
    }

    private static String publicAbilityName(String calleeURA, String ability) {
      String clean = ability == null ? "" : ability.trim();
      String owner = abilityOwnerPrefix(calleeURA);
      if (!owner.isBlank() && clean.startsWith(owner + ".")) {
        return clean.substring(owner.length() + 1);
      }
      String marker = "/ability/";
      int index = clean.indexOf(marker);
      if (index >= 0) {
        return publicAbilityName(calleeURA, clean.substring(index + marker.length()));
      }
      return clean;
    }

    private static String abilityOwnerPrefix(String calleeURA) {
      String clean = calleeURA == null ? "" : calleeURA.trim();
      String device = "/device/";
      int deviceIndex = clean.indexOf(device);
      if (deviceIndex >= 0) {
        String rest = clean.substring(deviceIndex + device.length());
        return "device." + rest.split("[/?#]", 2)[0];
      }
      if (clean.endsWith("/authority")) {
        String realmMarker = "easynet:///r/";
        if (clean.startsWith(realmMarker)) {
          return "hub."
              + clean.substring(realmMarker.length(), clean.length() - "/authority".length());
        }
      }
      return "";
    }
  }
}
