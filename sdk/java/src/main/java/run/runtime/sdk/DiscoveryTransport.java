package run.runtime.sdk;

public interface DiscoveryTransport extends AutoCloseable {
  FeatureSet featureDiscovery();

  @Override
  default void close() {}
}
