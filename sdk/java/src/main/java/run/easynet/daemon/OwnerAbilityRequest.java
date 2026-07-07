package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record OwnerAbilityRequest(String ownerURA, String abilityName) {
  public OwnerAbilityRequest {
    ownerURA = DirectoryIdentitySupport.cleanRequired(ownerURA, "owner_ura");
    abilityName = DirectoryIdentitySupport.cleanRequired(abilityName, "ability_name");
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> value = new LinkedHashMap<>();
    value.put("owner_ura", ownerURA);
    value.put("ability_name", abilityName);
    return JsonValueWriter.object(value);
  }
}
