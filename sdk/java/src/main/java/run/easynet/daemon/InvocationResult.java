package run.easynet.daemon;

import java.util.Map;

public record InvocationResult(
    boolean ok,
    InvocationTerminalState terminalState,
    String outputJson,
    SDKError error,
    Map<String, Object> receipt) {
  public InvocationResult {
    if (terminalState == null) {
      throw SDKError.validation("runtime", "terminalState is required");
    }
    receipt = receipt == null ? Map.of() : Map.copyOf(receipt);
    if (ok && error != null) {
      throw SDKError.validation("runtime", "ok result must not carry error");
    }
    if (!ok && error == null) {
      throw SDKError.validation("runtime", "failed result must carry error");
    }
  }
}
