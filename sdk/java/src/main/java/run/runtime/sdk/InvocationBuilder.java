package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public final class InvocationBuilder {
  private String caller;
  private String callee;
  private String descriptor;
  private String subject;
  private String nonce;
  private String causalContext;
  private String argsJson;
  private Map<String, Object> metadata = Map.of();
  private SDKError authorityError;

  public InvocationBuilder caller(String value) {
    caller = value;
    return this;
  }

  public InvocationBuilder callee(String value) {
    callee = value;
    return this;
  }

  public InvocationBuilder descriptor(String value) {
    descriptor = value;
    return this;
  }

  public InvocationBuilder subject(String value) {
    subject = value;
    return this;
  }

  public InvocationBuilder nonce(String value) {
    nonce = value;
    return this;
  }

  public InvocationBuilder causalContext(String value) {
    causalContext = value;
    return this;
  }

  public InvocationBuilder argsJson(String value) {
    argsJson = value;
    return this;
  }

  public InvocationBuilder metadata(Map<String, Object> value) {
    metadata = copyMetadata(value);
    return this;
  }

  public InvocationBuilder authorityMetadata(AuthorityMetadata value) {
    try {
      metadata = value.mergeInto(metadata);
    } catch (SDKError error) {
      authorityError = error;
    }
    return this;
  }

  public InvocationDraft inspect() {
    if (authorityError != null) {
      throw authorityError;
    }
    AuthoritySupport.validateAuthorityMetadata(metadata);
    InvocationTuple tuple =
        new InvocationTuple(caller, callee, descriptor, subject, nonce, causalContext, argsJson, metadata);
    rejectReceiptHistoryPublicInvocation(tuple);
    InvocationAuthorityBindingValidator.validate(tuple);
    return new InvocationDraft(tuple);
  }

  private static void rejectReceiptHistoryPublicInvocation(InvocationTuple tuple) {
    String historyAbility =
        RuntimeAbilityProjection.receiptHistoryReadAbility(tuple.callee(), tuple.descriptor());
    if (!historyAbility.isBlank()) {
      throw SDKError.validation(
          "invocation",
          "receipt history ability `"
              + historyAbility
              + "` is not a public invocation action; use RuntimeReceiptProvider as the canonical invocation history read path");
    }
  }

  private static Map<String, Object> copyMetadata(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<String, Object> entry : value.entrySet()) {
      if (entry.getKey() == null || entry.getKey().isBlank()) {
        throw SDKError.validation("invocation", "metadata keys must be non-empty strings");
      }
      out.put(entry.getKey(), entry.getValue());
    }
    return Map.copyOf(out);
  }
}
