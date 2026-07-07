package run.easynet.daemon;

public enum CompanionProjectedState {
  DISABLED("disabled"),
  UNSUPPORTED_PLATFORM("unsupported_platform"),
  UNSUPPORTED_SESSION("unsupported_session"),
  NOT_INSTALLED("not_installed"),
  INSTALLED_DISABLED("installed_disabled"),
  READY_STOPPED("ready_stopped"),
  STARTING("starting"),
  RUNNING("running"),
  STALE("stale"),
  ERROR("error");

  private final String wireValue;

  CompanionProjectedState(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionProjectedState fromWire(String value, String field) {
    for (CompanionProjectedState state : values()) {
      if (state.wireValue.equals(value)) {
        return state;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
