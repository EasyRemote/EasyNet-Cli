package run.easynet.daemon;

import java.util.List;
import java.util.Map;

public record PackageValidation(
    String profile,
    String kind,
    boolean valid,
    String packagePath,
    String manifestPath,
    String manifestHash,
    Manifest manifest,
    List<Object> errors,
    Map<String, Object> metadata) {
  public PackageValidation {
    if (!PublicationSupport.PROFILE.equals(profile) || !"package_validation".equals(kind)) {
      throw PublicationSupport.invalid("invalid package validation projection");
    }
    packagePath = PublicationSupport.required(packagePath, "package_path");
    manifestPath = PublicationSupport.required(manifestPath, "manifest_path");
    manifestHash = PublicationSupport.required(manifestHash, "manifest_hash");
    if (manifest == null) {
      throw PublicationSupport.invalid("manifest is required");
    }
    errors = errors == null ? List.of() : List.copyOf(errors);
    metadata = PublicationSupport.copyObject(metadata);
  }

  public static PackageValidation fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "package validation JSON");
    return new PackageValidation(
        PublicationSupport.requiredString(fields, "profile"),
        PublicationSupport.requiredString(fields, "kind"),
        PublicationSupport.requiredBoolean(fields, "valid"),
        PublicationSupport.requiredString(fields, "package_path"),
        PublicationSupport.requiredString(fields, "manifest_path"),
        PublicationSupport.requiredString(fields, "manifest_hash"),
        Manifest.fromObject(PublicationSupport.requiredObject(fields, "manifest")),
        PublicationSupport.requiredList(fields, "errors"),
        PublicationSupport.requiredObject(fields, "metadata"));
  }

  public record Manifest(
      String name,
      String namespace,
      String wireKey,
      String descriptorVersion,
      String description,
      String execKind,
      Long timeoutSeconds,
      Map<String, Object> inputSchema,
      Object outputSchema) {
    public Manifest {
      name = PublicationSupport.required(name, "name");
      namespace = PublicationSupport.required(namespace, "namespace");
      wireKey = PublicationSupport.required(wireKey, "wire_key");
      descriptorVersion = PublicationSupport.required(descriptorVersion, "descriptor_version");
      description = description == null ? "" : description;
      execKind = PublicationSupport.required(execKind, "exec_kind");
      if (timeoutSeconds != null && timeoutSeconds < 0) {
        throw PublicationSupport.invalid("timeout_seconds must be non-negative");
      }
      inputSchema = PublicationSupport.copyObject(inputSchema);
      if (inputSchema.isEmpty()) {
        throw PublicationSupport.invalid("input_schema is required");
      }
    }

    static Manifest fromObject(Map<String, Object> fields) {
      return new Manifest(
          PublicationSupport.requiredString(fields, "name"),
          PublicationSupport.requiredString(fields, "namespace"),
          PublicationSupport.requiredString(fields, "wire_key"),
          PublicationSupport.requiredString(fields, "descriptor_version"),
          PublicationSupport.requiredString(fields, "description"),
          PublicationSupport.requiredString(fields, "exec_kind"),
          PublicationSupport.optionalLong(fields.get("timeout_seconds"), "timeout_seconds"),
          PublicationSupport.requiredObject(fields, "input_schema"),
          fields.get("output_schema"));
    }
  }
}
