package run.runtime.sdk;

/** Canonical runtime principal validation shared by SDK runtime boundaries. */
final class RuntimePrincipals {
  private static final String ALL_ZERO_PRINCIPAL_ID = "00000000-0000-0000-0000-000000000000";

  private RuntimePrincipals() {}

  static String requiredString(Object value, String field, String stage) {
    if (!(value instanceof String string) || string.isBlank() || !string.equals(string.trim())) {
      throw SDKError.validation(stage, field + " is required");
    }
    return string;
  }

  static String requiredPrincipalID(String value, String field, String stage) {
    String cleaned = requiredString(value, field, stage);
    if (containsAllZeroPrincipal(cleaned)) {
      throw SDKError.validation(stage, field + " must not be all-zero");
    }
    return cleaned;
  }

  static boolean containsAllZeroPrincipal(String value) {
    return value != null && value.trim().toLowerCase().contains(ALL_ZERO_PRINCIPAL_ID);
  }
}
