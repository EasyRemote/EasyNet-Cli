package run.runtime.sdk;

import java.util.Objects;

public final class Client implements AutoCloseable {
  private final DiscoveryTransport transport;
  private boolean closed;

  public Client(DiscoveryTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public FeatureSet featureDiscovery() {
    requireOpen();
    return transport.featureDiscovery();
  }

  public FeatureSet requireABI(int expected) {
    FeatureSet features = featureDiscovery();
    if (features.abiVersion() != expected) {
      throw new SDKError(
          ErrorCode.VERSION_INCOMPATIBLE,
          "feature_discovery",
          RetryHint.NEVER,
          false,
          "ABI version mismatch",
          "",
          "",
          "",
          java.util.Map.of("expected", expected, "actual", features.abiVersion()),
          null);
    }
    return features;
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
      throw SDKError.closed("client");
    }
  }
}
