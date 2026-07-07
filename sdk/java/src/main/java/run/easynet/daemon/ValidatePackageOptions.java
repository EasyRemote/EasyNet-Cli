package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record ValidatePackageOptions(AbilityPackageManifest manifest, Map<String, Object> metadata) {
  public ValidatePackageOptions(AbilityPackageManifest manifest) {
    this(manifest, Map.of());
  }

  public ValidatePackageOptions {
    metadata = PublicationSupport.copyObject(metadata);
  }

  Map<String, Object> toObject(String packagePath) {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    String cleaned = PublicationSupport.optional(packagePath, "package_path");
    if (cleaned != null) {
      out.put("package_path", cleaned);
    }
    if (manifest != null) {
      out.put("manifest", manifest.toObject());
    }
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    if (out.isEmpty()) {
      throw PublicationSupport.invalid("package path or manifest is required");
    }
    return out;
  }
}
