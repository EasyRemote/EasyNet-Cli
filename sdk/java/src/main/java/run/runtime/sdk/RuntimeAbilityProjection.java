package run.runtime.sdk;

final class RuntimeAbilityProjection {
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
  private final String intrinsicName;

  private RuntimeAbilityProjection(String abilityURA, String publicName, String intrinsicName) {
    this.abilityURA = abilityURA;
    this.publicName = publicName;
    this.intrinsicName = intrinsicName;
  }

  static RuntimeAbilityProjection fromTuple(InvocationTuple tuple) {
    return fromDescriptorRef(tuple.callee(), tuple.descriptor());
  }

  String abilityURA() {
    return abilityURA;
  }

  String publicName() {
    return publicName;
  }

  String intrinsicName() {
    return intrinsicName;
  }

  static String runtimeGovernanceReadAbility(String calleeURA, String descriptorRef) {
    RuntimeAbilityProjection ability = fromDescriptorRef(calleeURA, descriptorRef);
    String matched = runtimeGovernanceReadAbility(ability.publicName());
    if (!matched.isBlank()) {
      return matched;
    }
    return runtimeGovernanceReadAbility(ability.intrinsicName());
  }

  private static RuntimeAbilityProjection fromDescriptorRef(String calleeURA, String descriptorRef) {
    AbilityDescriptorProjection projection = descriptorAbilityProjection(descriptorRef);
    String publicName = publicAbilityName(calleeURA, projection.intrinsicName());
    return new RuntimeAbilityProjection(projection.abilityURA(), publicName, projection.intrinsicName());
  }

  private static AbilityDescriptorProjection descriptorAbilityProjection(String descriptorRef) {
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
    String path = canonicalTopLevelPath(ability);
    String abilityPrefix = "ability/";
    if (!path.startsWith(abilityPrefix)) {
      throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA");
    }
    String intrinsicName = path.substring(abilityPrefix.length()).trim();
    if (intrinsicName.isBlank() || intrinsicName.contains("/")) {
      throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA");
    }
    return new AbilityDescriptorProjection(ability, intrinsicName);
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

  private static String publicAbilityName(String calleeURA, String intrinsicName) {
    String clean = intrinsicName == null ? "" : intrinsicName.trim();
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

  private record AbilityDescriptorProjection(String abilityURA, String intrinsicName) {}
}
