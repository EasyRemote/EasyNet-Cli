package run.easynet.daemon;

public interface HealthTransport extends AutoCloseable {
  byte[] runtimeHealth();

  @Override
  default void close() {}
}

