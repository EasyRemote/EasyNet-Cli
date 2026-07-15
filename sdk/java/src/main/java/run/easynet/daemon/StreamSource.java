package run.easynet.daemon;

public interface StreamSource extends AutoCloseable {
  StreamEvent next();

  default StreamEvent cancel(String reason) {
    return StreamEvent.transportTerminal(0, "cancel_requested", "CancelRequested");
  }

  @Override
  default void close() {}
}
