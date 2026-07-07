package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public final class SignedInvocation {
  private final PreparedInvocation prepared;
  private final InvocationSignature signature;
  private final String signerId;
  private final Map<String, Object> policy;
  private RuntimeClient runtime;

  public SignedInvocation(
      PreparedInvocation prepared,
      InvocationSignature signature,
      String signerId,
      Map<String, Object> policy) {
    this.prepared = java.util.Objects.requireNonNull(prepared, "prepared");
    this.signature = java.util.Objects.requireNonNull(signature, "signature");
    if (signerId == null || signerId.isBlank()) {
      throw SDKError.validation("signed_invocation", "signer_id is required");
    }
    this.signerId = signerId;
    this.policy = policy == null ? Map.of() : Map.copyOf(policy);
    if (!submitReady()) {
      throw SDKError.validation("signed_invocation", "signed invocation is not submit-ready");
    }
  }

  SignedInvocation bindRuntime(RuntimeClient runtime) {
    this.runtime = runtime;
    return this;
  }

  public PreparedInvocation prepared() {
    return prepared;
  }

  public InvocationSignature signature() {
    return signature;
  }

  public String signerId() {
    return signerId;
  }

  public boolean submitReady() {
    return !signerId.isBlank()
        && !signature.algorithm().isBlank()
        && !signature.signatureBase64().isBlank()
        && !prepared.descriptorRef().isBlank()
        && !prepared.signingMaterial().canonicalBytesBase64().isBlank();
  }

  public InvocationHandle submit() {
    if (runtime == null) {
      throw SDKError.validation("signed_invocation", "runtime binding is required");
    }
    return runtime.submitSigned(this);
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }

  Map<String, Object> toObject() {
    Map<String, Object> preparedOut = new LinkedHashMap<>();
    preparedOut.put("prepared_id", prepared.preparedId());
    preparedOut.put("request_id", prepared.requestId());
    preparedOut.put("descriptor_ref", prepared.descriptorRef());
    preparedOut.put("canonical_hash_hex", prepared.canonicalHashHex());
    preparedOut.put("expires_at_unix_ms", prepared.expiresAtUnixMS());
    preparedOut.put("canonical_bytes_base64", prepared.signingMaterial().canonicalBytesBase64());
    preparedOut.put("tuple", prepared.tuple().toWireObject());

    Map<String, Object> out = new LinkedHashMap<>();
    out.put("signer_id", signerId);
    out.put("prepared", preparedOut);
    out.put("signature", signature.toObject());
    if (!policy.isEmpty()) {
      out.put("policy", policy);
    }
    return out;
  }
}
