package run.runtime.sdk;

public enum InvocationTerminalState {
  COMPLETED,
  FAILED,
  CANCELLED,
  TIMED_OUT,
  BACKPRESSURE_TERMINATED
}
