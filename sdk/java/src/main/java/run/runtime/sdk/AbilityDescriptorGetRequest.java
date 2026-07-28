package run.runtime.sdk;

public record AbilityDescriptorGetRequest(
    RuntimeCallContext call,
    String abilityURA,
    String descriptorVersion,
    String callMode,
    String scope) {
  public AbilityDescriptorGetRequest(RuntimeCallContext call, String abilityURA) {
    this(call, abilityURA, "", "", "");
  }

  public AbilityDescriptorGetRequest {
    if (call == null) {
      throw SDKError.validation("ability_descriptor", "call is required");
    }
    abilityURA = required(abilityURA, "ability_ura");
    descriptorVersion = optional(descriptorVersion);
    callMode = optional(callMode);
    scope = optional(scope);
  }

  private static String required(String value, String field) {
    String clean = optional(value);
    if (clean.isBlank()) {
      throw SDKError.validation("ability_descriptor", field + " is required");
    }
    return clean;
  }

  private static String optional(String value) {
    return value == null ? "" : value.trim();
  }
}
