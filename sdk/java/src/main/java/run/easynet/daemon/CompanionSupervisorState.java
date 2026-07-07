package run.easynet.daemon;

public enum CompanionSupervisorState {
  UNSUPPORTED_PLATFORM("unsupported_platform"),
  UNSUPPORTED_SESSION("unsupported_session"),
  NOT_INSTALLED("not_installed"),
  INSTALLED_DISABLED("installed_disabled"),
  INSTALLED_ENABLED("installed_enabled"),
  INSTALL_ERROR("install_error"),
  ENABLE_ERROR("enable_error"),
  DISABLE_ERROR("disable_error");

  private final String wireValue;

  CompanionSupervisorState(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionSupervisorState fromWire(String value, String field) {
    for (CompanionSupervisorState state : values()) {
      if (state.wireValue.equals(value)) {
        return state;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
