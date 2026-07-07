package run.easynet.daemon;

public interface StreamSource extends AutoCloseable {
  StreamEvent next();

  default StreamEvent cancel(String reason) {
    return StreamEvent.terminal(0, "Cancelled");
  }

  @Override
  default void close() {}
}
