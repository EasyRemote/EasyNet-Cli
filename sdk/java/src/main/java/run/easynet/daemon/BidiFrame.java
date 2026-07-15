package run.easynet.daemon;

public record BidiFrame(
    long sequence, String kind, String payloadJson, boolean terminal, boolean transportTerminal) {
  public BidiFrame {
    if (sequence < 0) {
      throw SDKError.validation("bidi", "sequence must be non-negative");
    }
    if (kind == null || kind.isBlank()) {
      throw SDKError.validation("bidi", "kind is required");
    }
    payloadJson = payloadJson == null ? "" : payloadJson;
  }

  public static BidiFrame data(long sequence, String payloadJson) {
    return new BidiFrame(sequence, "data", payloadJson, false, false);
  }

  public static BidiFrame terminal(long sequence, String kind) {
    return new BidiFrame(sequence, kind, "", true, false);
  }

  public static BidiFrame transportTerminal(long sequence, String kind) {
    return new BidiFrame(sequence, kind, "", false, true);
  }
}
