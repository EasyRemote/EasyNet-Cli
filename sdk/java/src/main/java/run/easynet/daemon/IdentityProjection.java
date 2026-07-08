package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public record IdentityProjection(
    String kind,
    boolean valid,
    String descriptorRef,
    String abilityURA,
    String resourceURA,
    String ura,
    String descriptorVersion,
    String profile,
    Map<String, Object> components,
    Map<String, Object> metadata) {
  public IdentityProjection {
    kind = DirectoryIdentitySupport.requiredString(kind, "kind");
    profile = DirectoryIdentitySupport.requiredString(profile, "profile");
    components = DirectoryIdentitySupport.copyObject(components);
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  public static IdentityProjection fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "identity projection JSON");
    return new IdentityProjection(
        DirectoryIdentitySupport.requiredString(fields, "kind"),
        DirectoryIdentitySupport.requiredBoolean(fields, "valid"),
        DirectoryIdentitySupport.optionalString(fields.get("descriptor_ref"), "descriptor_ref"),
        DirectoryIdentitySupport.optionalString(fields.get("ability_ura"), "ability_ura"),
        DirectoryIdentitySupport.optionalString(fields.get("resource_ura"), "resource_ura"),
        DirectoryIdentitySupport.optionalString(fields.get("ura"), "ura"),
        DirectoryIdentitySupport.optionalString(
            fields.get("descriptor_version"), "descriptor_version"),
        DirectoryIdentitySupport.requiredString(fields, "profile"),
        DirectoryIdentitySupport.requiredObject(fields, "components"),
        DirectoryIdentitySupport.requiredObject(fields, "metadata"));
  }

  public static SDKError invalid(String message) {
    return DirectoryIdentitySupport.invalidField("descriptor_ref", message);
  }
}
