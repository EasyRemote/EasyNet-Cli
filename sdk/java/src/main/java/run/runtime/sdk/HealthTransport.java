package run.runtime.sdk;

public interface HealthTransport extends AutoCloseable {
  byte[] runtimeHealth();

  @Override
  default void close() {}
}

