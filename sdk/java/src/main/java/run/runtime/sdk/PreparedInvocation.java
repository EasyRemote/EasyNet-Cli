package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public final class PreparedInvocation {
  private final String preparedId;
  private final String requestId;
  private final InvocationDraft draft;
  private final SigningMaterial signingMaterial;
  private final String descriptorRef;
  private final String descriptorHashHex;
  private final String schemaHashHex;
  private final String canonicalHashHex;
  private final long expiresAtUnixMS;
  private RuntimeClient runtime;

  public PreparedInvocation(
      String preparedId,
      String requestId,
      InvocationDraft draft,
      SigningMaterial signingMaterial,
      String descriptorRef,
      String descriptorHashHex,
      String schemaHashHex,
      String canonicalHashHex,
      long expiresAtUnixMS,
      boolean submitReady) {
    if (submitReady) {
      throw SDKError.validation("prepared_invocation", "PreparedInvocation must not be submit-ready");
    }
    this.preparedId = preparedId == null ? "" : preparedId;
    this.requestId = requestId == null ? "" : requestId;
    if (this.preparedId.isBlank()) {
      throw SDKError.validation("prepared_invocation", "prepared_id is required");
    }
    this.draft = java.util.Objects.requireNonNull(draft, "draft");
    this.signingMaterial = java.util.Objects.requireNonNull(signingMaterial, "signingMaterial");
    this.descriptorRef = required(descriptorRef, "descriptor_ref");
    if (!this.descriptorRef.equals(signingMaterial.descriptorRef())
        || !draft.inspectTuple().descriptor().equals(signingMaterial.descriptorRef())) {
      throw SDKError.validation(
          "prepared_invocation", "signing_material.descriptor_ref must match tuple descriptor_ref");
    }
    this.descriptorHashHex = descriptorHashHex == null ? "" : descriptorHashHex;
    this.schemaHashHex = schemaHashHex == null ? "" : schemaHashHex;
    this.canonicalHashHex = canonicalHashHex == null ? "" : canonicalHashHex;
    this.expiresAtUnixMS = expiresAtUnixMS < 0 ? signingMaterial.expiresAtUnixMS() : expiresAtUnixMS;
  }

  public static PreparedInvocation fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "prepared invocation"));
  }

  static PreparedInvocation fromObject(Map<String, Object> fields) {
    rejectUnknown(
        fields,
        "prepared_id",
        "request_id",
        "tuple",
        "signing_material",
        "descriptor_ref",
        "descriptor_hash_hex",
        "schema_hash_hex",
        "canonical_hash_hex",
        "expires_at_unix_ms",
        "submit_ready");
    Object ready = fields.get("submit_ready");
    if (ready != null && (!(ready instanceof Boolean bool) || bool)) {
      throw SDKError.validation("prepared_invocation", "PreparedInvocation must not be submit-ready");
    }
    InvocationDraft draft = InvocationDraft.fromWireObject(object(fields, "tuple"));
    SigningMaterial material = SigningMaterial.fromObject(object(fields, "signing_material"));
    return new PreparedInvocation(
        optionalString(fields, "prepared_id"),
        optionalString(fields, "request_id"),
        draft,
        material,
        optionalString(fields, "descriptor_ref"),
        optionalString(fields, "descriptor_hash_hex"),
        optionalString(fields, "schema_hash_hex"),
        optionalString(fields, "canonical_hash_hex"),
        optionalLong(fields, "expires_at_unix_ms", material.expiresAtUnixMS()),
        false);
  }

  private static void rejectUnknown(Map<String, Object> fields, String... allowed) {
    java.util.Set<String> allowedSet = java.util.Set.of(allowed);
    for (String key : fields.keySet()) {
      if (!allowedSet.contains(key)) {
        throw SDKError.validation("prepared_invocation", key + " is not supported");
      }
    }
  }

  PreparedInvocation bindRuntime(RuntimeClient runtime) {
    this.runtime = runtime;
    return this;
  }

  public InvocationTuple tuple() {
    return draft.inspectTuple();
  }

  public SigningMaterial signingMaterial() {
    return signingMaterial;
  }

  public String preparedId() {
    return preparedId;
  }

  public String requestId() {
    return requestId;
  }

  public String descriptorRef() {
    return descriptorRef;
  }

  public String canonicalHashHex() {
    return canonicalHashHex;
  }

  public long expiresAtUnixMS() {
    return expiresAtUnixMS;
  }

  public boolean submitReady() {
    return false;
  }

  public SignedInvocation signWithCallerSignature(InvocationSignature signature) {
    String signerId = signature.keyIdHint().isBlank() ? signature.signerPublicKeyBase64() : signature.keyIdHint();
    if (signingMaterial.signerPolicy() != null && !signingMaterial.signerPolicy().signerId().isBlank()) {
      signerId = signingMaterial.signerPolicy().signerId();
    }
    if (signerId.isBlank()) {
      throw SDKError.validation("prepared_invocation", "signer id is required");
    }
    return new SignedInvocation(
            this,
            signature,
            signerId,
            signingMaterial.signerPolicy() == null ? Map.of() : signingMaterial.signerPolicy().toObject())
        .bindRuntime(runtime);
  }

  Map<String, Object> toObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("prepared_id", preparedId);
    out.put("request_id", requestId);
    out.put("tuple", tuple().toWireObject());
    out.put("signing_material", signingMaterial.toObject());
    out.put("descriptor_ref", descriptorRef);
    out.put("descriptor_hash_hex", descriptorHashHex);
    out.put("schema_hash_hex", schemaHashHex);
    out.put("canonical_hash_hex", canonicalHashHex);
    out.put("expires_at_unix_ms", expiresAtUnixMS);
    out.put("submit_ready", false);
    return out;
  }

  private static Map<String, Object> object(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (!(value instanceof Map<?, ?> raw)) {
      throw SDKError.validation("prepared_invocation", field + " is required");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation("prepared_invocation", field + " keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static String optionalString(Map<String, Object> fields, String field) {
    Object value = fields.get(field);
    if (value == null) {
      return "";
    }
    if (!(value instanceof String string)) {
      throw SDKError.validation("prepared_invocation", field + " must be a string");
    }
    return string;
  }

  private static long optionalLong(Map<String, Object> fields, String field, long defaultValue) {
    Object value = fields.get(field);
    if (value == null) {
      return defaultValue;
    }
    if (value instanceof Long longValue) {
      return longValue;
    }
    if (value instanceof Integer integerValue) {
      return integerValue.longValue();
    }
    throw SDKError.validation("prepared_invocation", field + " must be an integer");
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("prepared_invocation", field + " is required");
    }
    return value;
  }
}
