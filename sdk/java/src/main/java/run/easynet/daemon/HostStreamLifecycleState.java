package run.easynet.daemon;

public enum HostStreamLifecycleState {
  DECLARED,
  CHECKING,
  READY,
  NOT_READY,
  CLEANING,
  CLEANED,
  FAILED,
  CLOSED
}
