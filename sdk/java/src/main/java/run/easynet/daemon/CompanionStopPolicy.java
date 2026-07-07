package run.easynet.daemon;

public enum CompanionStopPolicy {
  KEEP_RUNNING("keep_running"),
  STOP_ON_RUNTIME_STOP("stop_on_runtime_stop"),
  STOP_ON_PLUGIN_DISABLE("stop_on_plugin_disable");

  private final String wireValue;

  CompanionStopPolicy(String wireValue) {
    this.wireValue = wireValue;
  }

  public String wireValue() {
    return wireValue;
  }

  static CompanionStopPolicy fromWire(String value, String field) {
    for (CompanionStopPolicy policy : values()) {
      if (policy.wireValue.equals(value)) {
        return policy;
      }
    }
    throw CompanionSupport.invalid(field + " is unsupported");
  }
}
