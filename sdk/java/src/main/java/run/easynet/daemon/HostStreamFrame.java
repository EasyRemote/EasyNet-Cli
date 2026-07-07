package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamFrame(
    String frameType,
    Long seq,
    Object value,
    SDKError error,
    HostStreamTerminalSummary terminal,
    String outputHash) {
  public HostStreamFrame {
    frameType = HostBindingSupport.required(frameType, "frame_type");
    switch (frameType) {
      case "item" -> {
        if (seq == null || error != null || terminal != null || outputHash != null) {
          throw HostBindingSupport.invalid("invalid item host stream frame");
        }
      }
      case "error" -> {
        if (seq != null || value != null || error == null || terminal != null || outputHash != null) {
          throw HostBindingSupport.invalid("invalid error host stream frame");
        }
      }
      case "terminal" -> {
        if (seq == null || value != null || error != null || terminal == null || outputHash == null
            || !outputHash.equals(terminal.outputHash())) {
          throw HostBindingSupport.invalid("invalid terminal host stream frame");
        }
      }
      default -> throw HostBindingSupport.invalid("unknown host stream frame type");
    }
  }

  public static HostStreamFrame fromJSON(byte[] raw) {
    var fields = JsonValueReader.object(raw, "host stream frame JSON");
    SDKError decodedError = null;
    Map<String, Object> errorObject = HostBindingSupport.optionalObject(fields.get("error"), "error");
    if (!errorObject.isEmpty()) {
      decodedError =
          new SDKError(
              ErrorCode.valueOf(HostBindingSupport.requiredString(errorObject, "code")),
              HostBindingSupport.requiredString(errorObject, "stage"),
              RetryHint.valueOf(HostBindingSupport.requiredString(errorObject, "retry").toUpperCase()),
              false,
              HostBindingSupport.requiredString(errorObject, "message"),
              HostBindingSupport.optionalString(errorObject.get("source"), "source"),
              HostBindingSupport.optionalString(errorObject.get("invocation_id"), "invocation_id"),
              HostBindingSupport.optionalString(errorObject.get("receipt_ura"), "receipt_ura"),
              HostBindingSupport.optionalObject(errorObject.get("details"), "details"),
              null);
    }
    Map<String, Object> terminalObject = HostBindingSupport.optionalObject(fields.get("terminal"), "terminal");
    return new HostStreamFrame(
        HostBindingSupport.requiredString(fields, "frame_type"),
        HostBindingSupport.optionalLong(fields.get("seq"), "seq"),
        fields.get("value"),
        decodedError,
        terminalObject.isEmpty() ? null : HostStreamTerminalSummary.fromObject(terminalObject),
        HostBindingSupport.optionalString(fields.get("output_hash"), "output_hash"));
  }

  public Map<String, Object> toObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("frame_type", frameType);
    object.put("seq", seq);
    object.put("value", value);
    object.put("error", error == null ? null : errorObject());
    object.put("terminal", terminal == null ? null : terminal.toObject());
    object.put("output_hash", outputHash);
    return object;
  }

  private Map<String, Object> errorObject() {
    LinkedHashMap<String, Object> object = new LinkedHashMap<>();
    object.put("code", error.code().name());
    object.put("stage", error.stage());
    object.put("message", error.getMessage());
    object.put("retry", error.retryHint().name().toLowerCase());
    object.put("source", error.source().isEmpty() ? null : error.source());
    object.put("invocation_id", error.invocationId().isEmpty() ? null : error.invocationId());
    object.put("receipt_ura", error.receiptURA().isEmpty() ? null : error.receiptURA());
    object.put("details", error.details());
    return object;
  }
}
