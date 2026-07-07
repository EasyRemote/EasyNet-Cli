package run.easynet.daemon;

public interface BidiSource extends AutoCloseable {
  void send(BidiFrame frame);

  BidiFrame next();

  default BidiFrame closeSend() {
    return BidiFrame.terminal(0, "send_closed");
  }

  default BidiFrame cancel(String reason) {
    return BidiFrame.terminal(0, "cancelled");
  }

  @Override
  default void close() {}
}
