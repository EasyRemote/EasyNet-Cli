package run.easynet.daemon;

import java.util.Map;

public record DesktopCompanionActionResult(
    String packageID,
    String action,
    boolean changed,
    DesktopCompanionStatus statusBefore,
    DesktopCompanionStatus statusAfter,
    Map<String, Object> error,
    Map<String, Object> metadata) {
  public DesktopCompanionActionResult {
    if (packageID == null || packageID.trim().isEmpty()) {
      throw CompanionSupport.invalid("package_id must be a non-empty string");
    }
    if (action == null || action.trim().isEmpty()) {
      throw CompanionSupport.invalid("action must be a non-empty string");
    }
    error = error == null ? null : CompanionSupport.optionalObject(error, "error");
    metadata = CompanionSupport.optionalObject(metadata, "metadata");
  }

  public static DesktopCompanionActionResult fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "desktop companion action JSON");
    return new DesktopCompanionActionResult(
        CompanionSupport.requiredString(fields, "package_id"),
        CompanionSupport.requiredString(fields, "action"),
        CompanionSupport.requiredBoolean(fields, "changed"),
        optionalStatus(fields.get("status_before"), "status_before"),
        optionalStatus(fields.get("status_after"), "status_after"),
        CompanionSupport.nullableObject(fields, "error"),
        CompanionSupport.optionalObject(fields, "metadata"));
  }

  private static DesktopCompanionStatus optionalStatus(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (!(value instanceof Map<?, ?> decoded)) {
      throw CompanionSupport.invalid(name + " must be an object or null");
    }
    return DesktopCompanionStatus.fromObject(CompanionSupport.optionalObject(decoded, name));
  }
}
