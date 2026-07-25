package run.runtime.sdk;

final class RuntimeAbilityProjection {
  private static final String ABILITY_PATH_MARKER = "/ability/";
  private static final String REALM_PREFIX = "easynet:///r/";

  private final String wire;
  private final String abilityURA;
  private final String publicName;

  private RuntimeAbilityProjection(String wire, String abilityURA, String publicName) {
    this.wire = wire;
    this.abilityURA = abilityURA;
    this.publicName = publicName;
  }

  static RuntimeAbilityProjection fromTuple(InvocationTuple tuple) {
    String abilityURA = descriptorAbilityURA(tuple.descriptor());
    String wire = descriptorWireAbility(abilityURA);
    String publicName = publicAbilityName(tuple.callee(), abilityURA);
    return new RuntimeAbilityProjection(wire, abilityURA, publicName);
  }

  String wire() {
    return wire;
  }

  String abilityURA() {
    return abilityURA;
  }

  String publicName() {
    return publicName;
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
    if (clean.endsWith("/authority") && clean.startsWith(REALM_PREFIX)) {
      return "hub."
          + clean.substring(REALM_PREFIX.length(), clean.length() - "/authority".length());
    }
    return "";
  }
}
