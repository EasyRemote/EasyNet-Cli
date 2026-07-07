package run.easynet.daemon;

public interface ReceiptTransport extends AutoCloseable {
  default byte[] fetch(byte[] requestJSON) {
    throw ReceiptSupport.unsupported("receipt fetch transport is not available");
  }

  default byte[] causalRef(byte[] receiptJSON) {
    throw ReceiptSupport.unsupported("receipt causal-ref transport is not available");
  }

  @Override
  default void close() {}
}
