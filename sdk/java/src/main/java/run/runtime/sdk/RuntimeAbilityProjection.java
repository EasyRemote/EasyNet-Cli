package run.runtime.sdk;

final class RuntimeAbilityProjection {
  private static final String ABILITY_PATH_MARKER = "/ability/";
  private static final String REALM_PREFIX = "easynet:///r/";
  private static final String[] RUNTIME_GOVERNANCE_READ_ABILITIES = {
    "meta.list_abilities",
    "invocation.history.list",
    "invocation.history.get",
    "invocation.history.path",
    "invocation.record.get",
    "invocation.trace.get"
  };

  private final String abilityURA;
  private final String publicName;

  private RuntimeAbilityProjection(String abilityURA, String publicName) {
    this.abilityURA = abilityURA;
    this.publicName = publicName;
  }

  static RuntimeAbilityProjection fromTuple(InvocationTuple tuple) {
    String abilityURA = descriptorAbilityURA(tuple.descriptor());
    String publicName = publicAbilityName(tuple.callee(), abilityURA);
    return new RuntimeAbilityProjection(abilityURA, publicName);
  }

  String abilityURA() {
    return abilityURA;
  }

  String publicName() {
    return publicName;
  }

  static String runtimeGovernanceReadAbility(String calleeURA, String descriptorRef) {
    String abilityURA = descriptorAbilityURA(descriptorRef);
    String publicName = publicAbilityName(calleeURA, abilityURA);
    String wireName = descriptorWireAbility(abilityURA);
    String matched = runtimeGovernanceReadAbility(publicName);
    if (!matched.isBlank()) {
      return matched;
    }
    return runtimeGovernanceReadAbility(wireName);
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
    String ability = (version >= 0 ? withoutMode.substring(0, version) : withoutMode).trim();
    if (!ability.startsWith(REALM_PREFIX) || !ability.contains(ABILITY_PATH_MARKER)) {
      throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA");
    }
    return ability;
  }

  private static String runtimeGovernanceReadAbility(String value) {
    String clean = value == null ? "" : value.trim();
    for (String ability : RUNTIME_GOVERNANCE_READ_ABILITIES) {
      if (clean.equals(ability) || clean.endsWith("." + ability)) {
        return ability;
      }
    }
    return "";
  }

  private static String descriptorWireAbility(String abilityURA) {
    int index = abilityURA.indexOf(ABILITY_PATH_MARKER);
    return abilityURA.substring(index + ABILITY_PATH_MARKER.length()).trim();
  }

  private static String publicAbilityName(String calleeURA, String abilityURA) {
    String clean = descriptorWireAbility(abilityURA);
    String owner = abilityOwnerPrefix(calleeURA);
    if (!owner.isBlank() && clean.startsWith(owner + ".")) {
      return clean.substring(owner.length() + 1);
    }
    return "";
  }

  private static String abilityOwnerPrefix(String calleeURA) {
    String path = canonicalTopLevelPath(calleeURA);
    if (path.startsWith("device/")) {
      String deviceID = path.substring("device/".length()).trim();
      if (!deviceID.isBlank() && !deviceID.contains("/")) {
        return "device." + deviceID;
      }
    }
    if ("authority".equals(path)) {
      return "authority";
    }
    return "";
  }

  private static String canonicalTopLevelPath(String ura) {
    String clean = ura == null ? "" : ura.trim();
    if (!clean.startsWith(REALM_PREFIX)) {
      return "";
    }
    String rest = clean.substring(REALM_PREFIX.length());
    int slash = rest.indexOf('/');
    if (slash <= 0 || slash == rest.length() - 1) {
      return "";
    }
    return rest.substring(slash + 1).trim();
  }
}
