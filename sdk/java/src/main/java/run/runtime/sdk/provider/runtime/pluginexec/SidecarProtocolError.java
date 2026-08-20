package run.runtime.sdk.provider.runtime.pluginexec;

/** Malformed runtime/plugin sidecar frame. */
public final class SidecarProtocolError extends RuntimeException {
  private static final long serialVersionUID = 1L;

  public SidecarProtocolError(String message) {
    super(message);
  }

  public SidecarProtocolError(String message, Throwable cause) {
    super(message, cause);
  }
}
