package run.easynet.daemon;

import java.util.Map;

public record AbilityQuery(
    DirectoryQueryBase base, String scope, String ownerURA, String abilityURA) {
  public AbilityQuery {
    if (base == null) {
      throw SDKError.validation("directory", "base query is required");
    }
    scope = DirectoryIdentitySupport.optionalClean(scope, "scope");
    ownerURA = DirectoryIdentitySupport.optionalClean(ownerURA, "owner_ura");
    abilityURA = DirectoryIdentitySupport.optionalClean(abilityURA, "ability_ura");
  }

  byte[] toJSON() {
    Map<String, Object> out = base.toObject();
    if (scope != null) {
      out.put("scope", scope);
    }
    if (ownerURA != null) {
      out.put("owner_ura", ownerURA);
    }
    if (abilityURA != null) {
      out.put("ability_ura", abilityURA);
    }
    return JsonValueWriter.object(out);
  }
}
