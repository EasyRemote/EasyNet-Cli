package run.easynet.daemon;

public enum CompanionBootPolicy {
  MANUAL("manual"),
  ENSURE_RUNNING_AFTER_DAEMON_READY("ensure_running_after_daemon_ready");

  private final String wireValue;

  CompanionBootPolicy(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionBootPolicy fromWire(String value, String field) {
    for (CompanionBootPolicy policy : values()) {
      if (policy.wireValue.equals(value)) {
        return policy;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
