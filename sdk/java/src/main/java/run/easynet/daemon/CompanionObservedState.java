package run.easynet.daemon;

public enum CompanionObservedState {
  UNKNOWN("unknown"),
  NOT_RUNNING("not_running"),
  STARTING("starting"),
  RUNNING("running"),
  STALE("stale"),
  EXITED("exited"),
  VERSION_MISMATCH("version_mismatch"),
  HEALTH_ERROR("health_error");

  private final String wireValue;

  CompanionObservedState(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionObservedState fromWire(String value, String field) {
    for (CompanionObservedState state : values()) {
      if (state.wireValue.equals(value)) {
        return state;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
