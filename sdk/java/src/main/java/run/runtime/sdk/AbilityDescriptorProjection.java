package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record AbilityDescriptorProjection(
    String abilityURA,
    String descriptorRef,
    String name,
    String ownerURA,
    String version,
    String schemaHash,
    String descriptorHash,
    String callMode,
    String className,
    Map<String, Object> receiptSemantics,
    String visibility,
    String source,
    String description,
    Map<String, Object> hints,
    Map<String, Object> schemaSummary,
    Map<String, Object> inputSchema,
    Map<String, Object> metadata) {
  public AbilityDescriptorProjection {
    abilityURA = required(abilityURA, "ability_ura");
    descriptorRef = required(descriptorRef, "descriptor_ref");
    name = required(name, "name");
    ownerURA = required(ownerURA, "owner_ura");
    version = required(version, "version");
    schemaHash = optional(schemaHash);
    descriptorHash = optional(descriptorHash);
    callMode = optional(callMode);
    className = optional(className);
    receiptSemantics = copyObject(receiptSemantics, "receipt_semantics");
    visibility = optional(visibility);
    source = optional(source);
    description = optional(description);
    hints = copyObject(hints, "hints");
    schemaSummary = copyObject(schemaSummary, "schema_summary");
    inputSchema = copyObject(inputSchema, "input_schema");
    metadata = copyObject(metadata, "metadata");
    String descriptorAbilityURA =
        RuntimeAbilityProjection.abilityURAForDescriptorRef(ownerURA, descriptorRef);
    if (!descriptorAbilityURA.equals(abilityURA)) {
      throw SDKError.validation(
          "ability_descriptor", "ability descriptor descriptor_ref does not bind ability_ura");
    }
  }

  static AbilityDescriptorProjection fromMap(Map<String, Object> fields) {
    return new AbilityDescriptorProjection(
        string(fields, "ability_ura"),
        string(fields, "descriptor_ref"),
        string(fields, "name"),
        string(fields, "owner_ura"),
        string(fields, "version"),
        optionalString(fields, "schema_hash"),
        optionalString(fields, "descriptor_hash"),
        optionalString(fields, "call_mode"),
        optionalString(fields, "class"),
        object(fields.get("receipt_semantics"), "receipt_semantics"),
        optionalString(fields, "visibility"),
        optionalString(fields, "source"),
        optionalString(fields, "description"),
        object(fields.get("hints"), "hints"),
        object(fields.get("schema_summary"), "schema_summary"),
        object(fields.get("input_schema"), "input_schema"),
        object(fields.get("metadata"), "metadata"));
  }

  static AbilityDescriptorProjection fromRuntimeMap(Map<String, Object> fields, int index) {
    return new AbilityDescriptorProjection(
        string(fields, runtimeRowField(index, "ability_ura"), "ability_ura"),
        string(fields, runtimeRowField(index, "descriptor_ref"), "descriptor_ref"),
        string(fields, runtimeRowField(index, "name"), "name"),
        string(fields, runtimeRowField(index, "owner_ura"), "owner_ura"),
        string(fields, runtimeRowField(index, "descriptor_version"), "descriptor_version"),
        optionalString(fields, runtimeRowField(index, "schema_hash"), "schema_hash"),
        optionalString(fields, runtimeRowField(index, "descriptor_hash"), "descriptor_hash"),
        optionalString(fields, runtimeRowField(index, "call_mode"), "call_mode"),
        optionalString(fields, runtimeRowField(index, "class"), "class"),
        object(fields.get("receipt_semantics"), runtimeRowField(index, "receipt_semantics")),
        optionalString(fields, runtimeRowField(index, "visibility"), "visibility"),
        optionalString(fields, runtimeRowField(index, "source"), "source"),
        optionalString(fields, runtimeRowField(index, "description"), "description"),
        object(fields.get("hints"), runtimeRowField(index, "hints")),
        object(fields.get("schema_summary"), runtimeRowField(index, "schema_summary")),
        object(fields.get("input_schema"), runtimeRowField(index, "input_schema")),
        object(fields.get("metadata"), runtimeRowField(index, "metadata")));
  }

  public Map<String, Object> toWireObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("ability_ura", abilityURA);
    out.put("descriptor_ref", descriptorRef);
    out.put("name", name);
    out.put("owner_ura", ownerURA);
    out.put("version", version);
    if (!schemaHash.isBlank()) out.put("schema_hash", schemaHash);
    if (!descriptorHash.isBlank()) out.put("descriptor_hash", descriptorHash);
    if (!callMode.isBlank()) out.put("call_mode", callMode);
    if (!className.isBlank()) out.put("class", className);
    out.put("receipt_semantics", receiptSemantics);
    if (!visibility.isBlank()) out.put("visibility", visibility);
    if (!source.isBlank()) out.put("source", source);
    if (!description.isBlank()) out.put("description", description);
    out.put("hints", hints);
    out.put("schema_summary", schemaSummary);
    out.put("input_schema", inputSchema);
    out.put("metadata", metadata);
    return Map.copyOf(out);
  }

  private static String string(Map<String, Object> fields, String field) {
    return string(fields, field, field);
  }

  private static String string(Map<String, Object> fields, String field, String sourceField) {
    Object value = fields.get(sourceField);
    if (!(value instanceof String string) || string.isBlank()) {
      throw SDKError.validation("ability_descriptor", field + " is required");
    }
    return string;
  }

  private static String optionalString(Map<String, Object> fields, String field) {
    return optionalString(fields, field, field);
  }

  private static String optionalString(Map<String, Object> fields, String field, String sourceField) {
    Object value = fields.get(sourceField);
    if (value == null) {
      return "";
    }
    if (!(value instanceof String string)) {
      throw SDKError.validation("ability_descriptor", field + " must be a string or null");
    }
    return string;
  }

  private static String runtimeRowField(int index, String field) {
    return "ability descriptor row " + index + " " + field;
  }

  private static Map<String, Object> object(Object value, String field) {
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> raw)) {
      throw SDKError.validation("ability_descriptor", field + " must be an object");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation("ability_descriptor", field + " keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static Map<String, Object> copyObject(Map<String, Object> value, String field) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<String, Object> entry : value.entrySet()) {
      if (entry.getKey() == null || entry.getKey().isBlank()) {
        throw SDKError.validation("ability_descriptor", field + " keys must be non-empty strings");
      }
      out.put(entry.getKey(), entry.getValue());
    }
    return Map.copyOf(out);
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
