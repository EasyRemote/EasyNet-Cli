package run.runtime.sdk;

public record AbilityDescriptorListRequest(
    RuntimeCallContext call, String scope, String ownerURA, String abilityURA) {
  public AbilityDescriptorListRequest(RuntimeCallContext call) {
    this(call, "", "", "");
  }

  public AbilityDescriptorListRequest {
    if (call == null) {
      throw SDKError.validation("ability_descriptor", "call is required");
    }
    scope = optional(scope);
    ownerURA = optional(ownerURA);
    abilityURA = optional(abilityURA);
  }

  private static String optional(String value) {
    return value == null ? "" : value.trim();
  }
}
