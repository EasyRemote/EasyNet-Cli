package run.easynet.daemon;

public record StreamEvent(
    long sequence,
    String kind,
    String state,
    String payloadJson,
    boolean terminal,
    boolean transportTerminal,
    SDKError error) {
  public StreamEvent {
    if (sequence < 0) {
      throw SDKError.validation("stream", "sequence must be non-negative");
    }
    if (kind == null || kind.isBlank()) {
      throw SDKError.validation("stream", "kind is required");
    }
    if (state == null || state.isBlank()) {
      throw SDKError.validation("stream", "state is required");
    }
  }

  public static StreamEvent data(long sequence, String payloadJson) {
    return new StreamEvent(sequence, "data", "Open", payloadJson, false, false, null);
  }

  public static StreamEvent terminal(long sequence, String state) {
    return new StreamEvent(sequence, "terminal", state, "", true, false, null);
  }

  public static StreamEvent transportTerminal(long sequence, String kind, String state) {
    return new StreamEvent(sequence, kind, state, "", false, true, null);
  }

  public static StreamEvent backpressure(long sequence) {
    return new StreamEvent(
        sequence,
        "error",
        "Failed",
        "",
        false,
        true,
        new SDKError(
            ErrorCode.TRANSPORT,
            "stream",
            RetryHint.SAFE,
            true,
            "stream retained history exceeded bounded capacity",
            "",
            "",
            "",
            java.util.Map.of("terminal_state", "backpressure"),
            null));
  }
}
