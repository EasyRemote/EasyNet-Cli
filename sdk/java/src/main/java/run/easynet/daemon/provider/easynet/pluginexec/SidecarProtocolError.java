package run.easynet.daemon.provider.easynet.pluginexec;

/** Malformed daemon/plugin sidecar frame. */
public final class SidecarProtocolError extends RuntimeException {
  public SidecarProtocolError(String message) {
    super(message);
  }

  public SidecarProtocolError(String message, Throwable cause) {
    super(message, cause);
  }
}
