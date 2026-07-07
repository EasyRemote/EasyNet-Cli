package run.easynet.daemon;

public enum CompanionDesiredState {
  ENABLED("enabled"),
  DISABLED("disabled");

  private final String wireValue;

  CompanionDesiredState(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionDesiredState fromWire(String value, String field) {
    for (CompanionDesiredState state : values()) {
      if (state.wireValue.equals(value)) {
        return state;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
