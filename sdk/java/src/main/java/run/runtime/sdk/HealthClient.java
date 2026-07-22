package run.runtime.sdk;

import java.util.Map;
import java.util.Objects;

public final class HealthClient implements AutoCloseable {
  private final HealthTransport transport;
  private boolean closed;

  public HealthClient(HealthTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public RuntimeHealth runtimeHealth() {
    requireOpen();
    try {
      return RuntimeHealth.fromJSON(transport.runtimeHealth());
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("runtime health transport failed", error);
    }
  }

  public DiagnosticsReport diagnostics() {
    requireOpen();
    if (!(transport instanceof DiagnosticsTransport diagnosticsTransport)) {
      throw new SDKError(
          ErrorCode.NOT_IMPLEMENTED,
          "transport",
          RetryHint.NEVER,
          false,
          "health diagnostics transport is not available",
          "",
          "",
          "",
          Map.of(),
          null);
    }
    try {
      return DiagnosticsReport.fromJSON(diagnosticsTransport.runtimeDiagnostics());
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("runtime diagnostics transport failed", error);
    }
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("health");
    }
  }

  private static SDKError transportFailure(String message, RuntimeException cause) {
    return new SDKError(
        ErrorCode.TRANSPORT,
        "transport",
        RetryHint.SAFE,
        true,
        message,
        "",
        "",
        "",
        Map.of(),
        cause);
  }
}

