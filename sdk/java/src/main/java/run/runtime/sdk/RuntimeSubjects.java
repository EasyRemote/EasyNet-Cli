package run.runtime.sdk;

/** Canonical runtime subject constructors shared by product facades. */
public final class RuntimeSubjects {
  static final String RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read";
  private static final String RETIRED_INVOCATION_HISTORY_SUBJECT_PATH = "session/invocation_history";

  private RuntimeSubjects() {}

  /** Build the user-owned Resource URA used for runtime-state read projections. */
  public static String runtimeStateReadSubjectURA(String realm, String userID) {
    String cleanRealm = RuntimePrincipals.requiredString(realm, "realm", "runtime");
    String cleanUserID = RuntimePrincipals.requiredPrincipalID(userID, "user_id", "runtime");
    if (cleanRealm.contains("/") || cleanRealm.contains("?") || cleanRealm.contains("#")) {
      throw invalid("runtime-state read subject realm is not canonical");
    }
    if (cleanUserID.contains("/") || cleanUserID.contains("?") || cleanUserID.contains("#")) {
      throw invalid("runtime-state read subject user_id is not canonical");
    }
    String subject =
        "easynet:///r/"
            + cleanRealm
            + "/resource/user."
            + cleanUserID
            + "/"
            + RUNTIME_STATE_READ_SUBJECT_PATH;
    if (canonicalResourceSubject(subject) == null) {
      throw invalid("runtime-state read subject_ura must be canonical");
    }
    return subject;
  }

  static ResourceSubject canonicalResourceSubject(String subjectURA) {
    if (subjectURA == null || RuntimePrincipals.containsAllZeroPrincipal(subjectURA)) {
      return null;
    }
    String raw = subjectURA.trim();
    String prefix = "easynet:///r/";
    if (!raw.startsWith(prefix)) {
      return null;
    }
    String rest = raw.substring(prefix.length());
    int slash = rest.indexOf('/');
    if (slash <= 0) {
      return null;
    }
    String path = rest.substring(slash + 1);
    String resourcePrefix = "resource/";
    if (!path.startsWith(resourcePrefix)) {
      return null;
    }
    String resource = path.substring(resourcePrefix.length());
    int pathSlash = resource.indexOf('/');
    if (pathSlash <= 0 || pathSlash == resource.length() - 1) {
      return null;
    }
    String ownerID = resource.substring(0, pathSlash).trim();
    String resourcePath = resource.substring(pathSlash + 1).trim();
    if (ownerID.isEmpty()
        || ownerID.contains("/")
        || resourcePath.isEmpty()
        || resourcePath.startsWith("/")
        || resourcePath.contains("//")) {
      return null;
    }
    return new ResourceSubject(ownerID, resourcePath);
  }

  static boolean isRetiredInvocationHistorySubject(String subjectURA) {
    ResourceSubject subject = canonicalResourceSubject(subjectURA);
    if (subject == null || !subject.ownerID().startsWith("user.")) {
      return false;
    }
    String ownerUserID = subject.ownerID().substring("user.".length()).trim();
    return !ownerUserID.isEmpty()
        && !ownerUserID.contains(".")
        && !RuntimePrincipals.containsAllZeroPrincipal(ownerUserID)
        && subject.path().equals(RETIRED_INVOCATION_HISTORY_SUBJECT_PATH);
  }

  private static SDKError invalid(String message) {
    return SDKError.validation("runtime", message);
  }

  record ResourceSubject(String ownerID, String path) {}
}
