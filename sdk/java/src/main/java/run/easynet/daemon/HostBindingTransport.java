package run.easynet.daemon;

public interface HostBindingTransport extends AutoCloseable {
  default byte[] buildHostStreamBinding(byte[] requestJSON) {
    throw HostBindingSupport.unsupported("host binding build transport is not available");
  }

  default byte[] decodeRequest(byte[] envelopeJSON) {
    throw HostBindingSupport.unsupported("host binding decode request transport is not available");
  }

  default byte[] encodeItem(byte[] requestJSON) {
    throw HostBindingSupport.unsupported("host binding encode item transport is not available");
  }

  default byte[] encodeError(byte[] requestJSON) {
    throw HostBindingSupport.unsupported("host binding encode error transport is not available");
  }

  default byte[] encodeTerminal(byte[] requestJSON) {
    throw HostBindingSupport.unsupported("host binding encode terminal transport is not available");
  }

  default byte[] foldOutputHash(byte[] requestJSON) {
    throw HostBindingSupport.unsupported("host binding hash transport is not available");
  }

  @Override
  default void close() {}
}
