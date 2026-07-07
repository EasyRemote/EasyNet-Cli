package run.easynet.daemon;

import java.util.Map;

public record DesktopCompanionStatus(
    String packageID,
    String packageVersion,
    String displayName,
    String platform,
    CompanionDesiredState desiredState,
    CompanionSupervisorState supervisorState,
    CompanionObservedState observedState,
    CompanionProjectedState projectedState,
    CompanionBootPolicy bootPolicy,
    CompanionStopPolicy stopPolicy,
    CompanionHealthMode health,
    Long pid,
    String version,
    Long lastSeenUnixMS,
    String launchMethod,
    Map<String, Object> error,
    Map<String, Object> metadata) {
  public DesktopCompanionStatus {
    packageID = require(packageID, "package_id");
    packageVersion = require(packageVersion, "package_version");
    displayName = require(displayName, "display_name");
    platform = require(platform, "platform");
    if (desiredState == null
        || supervisorState == null
        || observedState == null
        || projectedState == null
        || bootPolicy == null
        || stopPolicy == null
        || health == null) {
      throw CompanionSupport.invalid("desktop companion state fields are required");
    }
    error = error == null ? null : CompanionSupport.optionalObject(error, "error");
    metadata = CompanionSupport.optionalObject(metadata, "metadata");
  }

  public static DesktopCompanionStatus fromJSON(byte[] raw) {
    return fromObject(JsonValueReader.object(raw, "desktop companion status JSON"));
  }

  static DesktopCompanionStatus fromObject(Map<String, Object> fields) {
    return new DesktopCompanionStatus(
        CompanionSupport.requiredString(fields, "package_id"),
        CompanionSupport.requiredString(fields, "package_version"),
        CompanionSupport.requiredString(fields, "display_name"),
        CompanionSupport.requiredString(fields, "platform"),
        CompanionDesiredState.fromWire(CompanionSupport.requiredString(fields, "desired_state"), "desired_state"),
        CompanionSupervisorState.fromWire(
            CompanionSupport.requiredString(fields, "supervisor_state"), "supervisor_state"),
        CompanionObservedState.fromWire(CompanionSupport.requiredString(fields, "observed_state"), "observed_state"),
        CompanionProjectedState.fromWire(
            CompanionSupport.requiredString(fields, "projected_state"), "projected_state"),
        CompanionBootPolicy.fromWire(CompanionSupport.requiredString(fields, "boot_policy"), "boot_policy"),
        CompanionStopPolicy.fromWire(CompanionSupport.requiredString(fields, "stop_policy"), "stop_policy"),
        CompanionHealthMode.fromWire(CompanionSupport.requiredString(fields, "health"), "health"),
        CompanionSupport.optionalLong(fields, "pid"),
        CompanionSupport.optionalString(fields, "version"),
        CompanionSupport.optionalLong(fields, "last_seen_unix_ms"),
        CompanionSupport.optionalString(fields, "launch_method"),
        CompanionSupport.nullableObject(fields, "error"),
        CompanionSupport.optionalObject(fields, "metadata"));
  }

  private static String require(String value, String name) {
    if (value == null || value.trim().isEmpty()) {
      throw CompanionSupport.invalid(name + " must be a non-empty string");
    }
    return value;
  }
}
