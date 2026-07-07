package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class WrapperClient implements AutoCloseable {
  private final WrapperTransport transport;
  private boolean closed;

  public WrapperClient(WrapperTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public WrapperFileRecord projectFileRecord(byte[] valueJSON) {
    return WrapperFileRecord.fromJSON(raw(valueJSON, transport::projectFileRecord, "wrapper file projection failed"));
  }

  public WrapperFileRecord projectFileRecord(WrapperFileRecord value) {
    return projectFileRecord(Objects.requireNonNull(value, "value").toJSON());
  }

  public WrapperTerminalSession projectTerminalSession(byte[] valueJSON) {
    return WrapperTerminalSession.fromJSON(raw(valueJSON, transport::projectTerminalSession, "wrapper terminal projection failed"));
  }

  public WrapperTerminalSession projectTerminalSession(WrapperTerminalSession value) {
    return projectTerminalSession(Objects.requireNonNull(value, "value").toJSON());
  }

  public WrapperRemoteDesktopSession projectRemoteDesktopSession(byte[] valueJSON) {
    return WrapperRemoteDesktopSession.fromJSON(
        raw(valueJSON, transport::projectRemoteDesktopSession, "wrapper remote desktop projection failed"));
  }

  public WrapperRemoteDesktopSession projectRemoteDesktopSession(WrapperRemoteDesktopSession value) {
    return projectRemoteDesktopSession(Objects.requireNonNull(value, "value").toJSON());
  }

  public WrapperBrowserSession projectBrowserSession(byte[] valueJSON) {
    return WrapperBrowserSession.fromJSON(raw(valueJSON, transport::projectBrowserSession, "wrapper browser projection failed"));
  }

  public WrapperBrowserSession projectBrowserSession(WrapperBrowserSession value) {
    return projectBrowserSession(Objects.requireNonNull(value, "value").toJSON());
  }

  public WrapperMediaSession projectMediaSession(byte[] valueJSON) {
    return WrapperMediaSession.fromJSON(raw(valueJSON, transport::projectMediaSession, "wrapper media projection failed"));
  }

  public WrapperMediaSession projectMediaSession(WrapperMediaSession value) {
    return projectMediaSession(Objects.requireNonNull(value, "value").toJSON());
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private byte[] raw(byte[] valueJSON, WrapperBytesOperation operation, String message) {
    requireOpen();
    try {
      return operation.call(valueJSON);
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw new SDKError(
          ErrorCode.TRANSPORT,
          "transport",
          RetryHint.SAFE,
          true,
          message,
          "",
          "",
          "",
          Map.of("profile", WrapperSupport.PROFILE),
          error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("wrappers");
    }
  }

  @FunctionalInterface
  private interface WrapperBytesOperation {
    byte[] call(byte[] valueJSON);
  }
}
