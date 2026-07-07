package run.easynet.daemon;

public interface ReceiptTransport extends AutoCloseable {
  default byte[] fetch(byte[] requestJSON) {
    throw ReceiptSupport.unsupported("receipt fetch transport is not available");
  }

  default byte[] project(byte[] receiptJSON) {
    throw ReceiptSupport.unsupported("receipt projection transport is not available");
  }

  default byte[] verify(byte[] receiptJSON) {
    throw ReceiptSupport.unsupported("receipt verification transport is not available");
  }

  default byte[] verifyChain(byte[] requestJSON) {
    throw ReceiptSupport.unsupported("receipt chain verification transport is not available");
  }

  default byte[] causalRef(byte[] receiptJSON) {
    throw ReceiptSupport.unsupported("receipt causal-ref transport is not available");
  }

  @Override
  default void close() {}
}
