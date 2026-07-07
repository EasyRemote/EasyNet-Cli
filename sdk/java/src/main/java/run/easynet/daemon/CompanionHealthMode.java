package run.easynet.daemon;

public enum CompanionHealthMode {
  PROCESS_NAME("process_name"),
  STATUS_FILE("status_file"),
  LOCAL_IPC("local_ipc");

  private final String wireValue;

  CompanionHealthMode(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionHealthMode fromWire(String value, String field) {
    for (CompanionHealthMode mode : values()) {
      if (mode.wireValue.equals(value)) {
        return mode;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
