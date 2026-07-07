package run.easynet.daemon;

public interface WrapperTransport extends AutoCloseable {
  default byte[] projectFileRecord(byte[] valueJSON) {
    throw WrapperSupport.unsupported("wrapper file projection transport is not available");
  }

  default byte[] projectTerminalSession(byte[] valueJSON) {
    throw WrapperSupport.unsupported("wrapper terminal projection transport is not available");
  }

  default byte[] projectRemoteDesktopSession(byte[] valueJSON) {
    throw WrapperSupport.unsupported("wrapper remote desktop projection transport is not available");
  }

  default byte[] projectBrowserSession(byte[] valueJSON) {
    throw WrapperSupport.unsupported("wrapper browser projection transport is not available");
  }

  default byte[] projectMediaSession(byte[] valueJSON) {
    throw WrapperSupport.unsupported("wrapper media projection transport is not available");
  }

  @Override
  default void close() {}
}
