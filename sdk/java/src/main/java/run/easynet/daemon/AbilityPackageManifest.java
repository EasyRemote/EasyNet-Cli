package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record AbilityPackageManifest(
    String name,
    String namespace,
    String description,
    Map<String, Object> inputSchema,
    Object outputSchema,
    String descriptorVersion,
    Map<String, Object> exec) {
  public AbilityPackageManifest(
      String name,
      String namespace,
      String description,
      Map<String, Object> inputSchema,
      Map<String, Object> exec) {
    this(name, namespace, description, inputSchema, null, null, exec);
  }

  public AbilityPackageManifest {
    name = PublicationSupport.required(name, "name");
    namespace = PublicationSupport.required(namespace, "namespace");
    description = description == null ? "" : description;
    inputSchema = PublicationSupport.copyObject(inputSchema);
    if (inputSchema.isEmpty()) {
      throw PublicationSupport.invalid("input_schema is required");
    }
    descriptorVersion = PublicationSupport.optional(descriptorVersion, "descriptor_version");
    exec = PublicationSupport.copyObject(exec);
  }

  Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("name", name);
    out.put("namespace", namespace);
    out.put("description", description);
    out.put("input_schema", inputSchema);
    if (outputSchema != null) {
      out.put("output_schema", outputSchema);
    }
    if (descriptorVersion != null) {
      out.put("descriptor_version", descriptorVersion);
    }
    if (!exec.isEmpty()) {
      out.put("exec", exec);
    }
    return out;
  }
}
