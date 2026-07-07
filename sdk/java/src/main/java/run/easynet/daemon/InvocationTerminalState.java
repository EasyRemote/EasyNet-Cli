package run.easynet.daemon;

public enum InvocationTerminalState {
  COMPLETED,
  FAILED,
  CANCELLED,
  TIMED_OUT,
  BACKPRESSURE_TERMINATED
}
