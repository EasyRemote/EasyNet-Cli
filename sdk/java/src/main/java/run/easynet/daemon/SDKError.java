package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class SDKError extends RuntimeException {
  private static final long serialVersionUID = 1L;

  private final ErrorCode code;
  private final String stage;
  private final RetryHint retryHint;
  private final boolean retryable;
  private final String source;
  private final String invocationId;
  private final String receiptURA;
  private final transient Map<String, Object> details;

  public SDKError(
      ErrorCode code,
      String stage,
      RetryHint retryHint,
      boolean retryable,
      String message,
      String source,
      String invocationId,
      String receiptURA,
      Map<String, Object> details,
      Throwable cause) {
    super(required(message, "message"), cause);
    this.code = Objects.requireNonNull(code, "code");
    this.stage = required(stage, "stage");
    this.retryHint = Objects.requireNonNullElse(retryHint, RetryHint.NEVER);
    this.retryable = retryable;
    this.source = source == null ? "" : source;
    this.invocationId = invocationId == null ? "" : invocationId;
    this.receiptURA = receiptURA == null ? "" : receiptURA;
    this.details = details == null ? Map.of() : Map.copyOf(details);
  }

  public static SDKError validation(String stage, String message) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        stage,
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of(),
        null);
  }

  public static SDKError closed(String stage) {
    return new SDKError(
        ErrorCode.INVALID_HANDLE,
        stage,
        RetryHint.NEVER,
        false,
        stage + " is closed",
        "",
        "",
        "",
        Map.of(),
        null);
  }

  public ErrorCode code() {
    return code;
  }

  public String stage() {
    return stage;
  }

  public RetryHint retryHint() {
    return retryHint;
  }

  public boolean retryable() {
    return retryable;
  }

  public String source() {
    return source;
  }

  public String invocationId() {
    return invocationId;
  }

  public String receiptURA() {
    return receiptURA;
  }

  public Map<String, Object> details() {
    return details;
  }

  public ErrorClass errorClass() {
    return switch (code) {
      case INVALID_ARGUMENT, INVALID_INVOCATION -> ErrorClass.VALIDATION;
      case INVALID_HANDLE, NOT_INITIALIZED -> ErrorClass.HANDLE;
      case ROUTE_UNAVAILABLE, TRANSPORT -> ErrorClass.AVAILABILITY;
      case PERMISSION_DENIED, HTTP_AUTH_DENIED -> ErrorClass.PERMISSION;
      case ADMISSION_DENIED, SIGNATURE_DENIED, POLICY_DENIED, AUTHORITY_DENIED,
              EXECUTION_FAILED ->
          ErrorClass.ADMISSION;
      case ABILITY_NOT_FOUND -> ErrorClass.ROUTING;
      case TIMEOUT -> ErrorClass.TIMEOUT;
      case CANCELLED -> ErrorClass.CANCELLATION;
      case PROTOCOL, PROTOCOL_MISMATCH -> ErrorClass.PROTOCOL;
      case VERSION_INCOMPATIBLE -> ErrorClass.VERSION;
      case CONTROL_ONLY -> ErrorClass.CONTROL;
      case NOT_IMPLEMENTED -> ErrorClass.UNSUPPORTED;
      case GENERIC -> ErrorClass.GENERIC;
    };
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException(field + " is required");
    }
    return value;
  }
}
