package run.runtime.sdk;

public interface BidiSource extends AutoCloseable {
  void send(BidiFrame frame);

  BidiFrame next();

  default BidiFrame closeSend() {
    return BidiFrame.transportTerminal(0, "send_closed");
  }

  default BidiFrame cancel(String reason) {
    return BidiFrame.transportTerminal(0, "cancel_requested");
  }

  @Override
  default void close() {}
}
