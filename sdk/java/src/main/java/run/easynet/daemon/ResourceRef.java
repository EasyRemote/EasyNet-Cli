package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record ResourceRef(
    String resourceURA,
    String ownerURA,
    String namespace,
    String displayPath,
    String capability,
    long expiresUnixMS,
    String revision) {
  public ResourceRef {
    resourceURA = PublicationSupport.required(resourceURA, "resource_ura");
    ownerURA = PublicationSupport.required(ownerURA, "owner_ura");
    namespace = PublicationSupport.required(namespace, "namespace");
    if (!namespace.equals("fs")) {
      throw PublicationSupport.invalid("resource_ref namespace is unsupported");
    }
    displayPath = PublicationSupport.optional(displayPath, "display_path");
    capability = PublicationSupport.capability(capability);
    if (expiresUnixMS < 0) {
      throw PublicationSupport.invalid("expires_unix_ms must be non-negative");
    }
    revision = PublicationSupport.required(revision, "revision");
  }

  public static ResourceRef fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "resource ref JSON"));
  }

  static ResourceRef fromObject(Map<String, Object> fields) {
    return new ResourceRef(
        PublicationSupport.requiredString(fields, "resource_ura"),
        PublicationSupport.requiredString(fields, "owner_ura"),
        PublicationSupport.requiredString(fields, "namespace"),
        PublicationSupport.optionalString(fields.get("display_path"), "display_path"),
        PublicationSupport.requiredString(fields, "capability"),
        PublicationSupport.requiredLong(fields, "expires_unix_ms"),
        PublicationSupport.requiredString(fields, "revision"));
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("resource_ura", resourceURA);
    out.put("owner_ura", ownerURA);
    out.put("namespace", namespace);
    if (displayPath != null) {
      out.put("display_path", displayPath);
    }
    out.put("capability", capability);
    out.put("expires_unix_ms", expiresUnixMS);
    out.put("revision", revision);
    return out;
  }
}
