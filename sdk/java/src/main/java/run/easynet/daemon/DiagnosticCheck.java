package run.easynet.daemon;

public record DiagnosticCheck(String name, boolean ready, String message) {
  public DiagnosticCheck {
    if (name == null || name.isBlank()) {
      throw RuntimeHealth.invalidField("checks", "name must be a non-empty string");
    }
  }
}

