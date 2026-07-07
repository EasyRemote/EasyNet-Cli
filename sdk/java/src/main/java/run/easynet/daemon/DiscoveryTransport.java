package run.easynet.daemon;

public interface DiscoveryTransport extends AutoCloseable {
  FeatureSet featureDiscovery();

  @Override
  default void close() {}
}
