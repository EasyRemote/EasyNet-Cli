package run.runtime.sdk;

/** Canonical runtime subject constructors shared by product facades. */
public final class RuntimeSubjects {
  static final String RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read";

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

  static String descriptorBoundSubjectURA(String subjectURA, String abilityName) {
    String subject = RuntimePrincipals.requiredString(subjectURA, "subject_ura", "runtime");
    String ability = RuntimePrincipals.requiredString(abilityName, "ability name", "runtime");
    ParsedSubject parsed = parsedSubject(subject);
    if (parsed == null) {
      throw invalid("subject_ura is not a valid URA");
    }
    if ("authority".equals(parsed.path())) {
      return "easynet:///r/" + parsed.realm() + "/resource/authority/invoke/" + ability;
    }
    if (parsed.path().startsWith("user/")) {
      String userID = parsed.path().substring("user/".length()).trim();
      if (userID.isEmpty()
          || userID.contains("/")
          || userID.contains("?")
          || userID.contains("#")) {
        throw invalid("subject_ura user id is not canonical");
      }
      return "easynet:///r/" + parsed.realm() + "/resource/user." + userID + "/invoke/" + ability;
    }
    if (parsed.path().startsWith("agent/")
        || parsed.path().startsWith("ability/")
        || parsed.path().startsWith("device/")
        || parsed.path().startsWith("resource/")) {
      return subject;
    }
    throw invalid("subject_ura kind is not descriptor-bound");
  }

  static String runtimeGovernanceReadSubjectURA(String subjectURA) {
    String subject = RuntimePrincipals.requiredString(subjectURA, "subject_ura", "runtime");
    ResourceSubject resource = canonicalResourceSubject(subject);
    if (resource != null && RUNTIME_STATE_READ_SUBJECT_PATH.equals(resource.path())) {
      return subject;
    }
    ParsedSubject parsed = parsedSubject(subject);
    if (parsed != null && parsed.path().startsWith("user/")) {
      String userID = parsed.path().substring("user/".length()).trim();
      return runtimeStateReadSubjectURA(parsed.realm(), userID);
    }
    throw invalid("runtime governance read subject must be a User or user-owned runtime-state resource");
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

  static boolean isRuntimeGovernanceReadSubject(String subjectURA, String calleeURA) {
    String subject = subjectURA == null ? "" : subjectURA.trim();
    if (subject.isEmpty() || RuntimePrincipals.containsAllZeroPrincipal(subject)) {
      return false;
    }
    ResourceSubject resource = canonicalResourceSubject(subject);
    if (resource != null) {
      String userOwner = resource.ownerID().startsWith("user.")
          ? resource.ownerID().substring("user.".length()).trim()
          : "";
      return !userOwner.isEmpty()
          && !userOwner.contains(".")
          && !RuntimePrincipals.containsAllZeroPrincipal(userOwner)
          && RUNTIME_STATE_READ_SUBJECT_PATH.equals(resource.path());
    }
    RuntimeOwnerSubject parsedSubject = runtimeOwnerSubject(subject);
    RuntimeOwnerSubject parsedCallee = runtimeOwnerSubject(calleeURA);
    return parsedSubject != null
        && parsedCallee != null
        && parsedSubject.kind().equals(parsedCallee.kind())
        && parsedSubject.realm().equals(parsedCallee.realm())
        && subject.equals(calleeURA == null ? "" : calleeURA.trim());
  }

  private static RuntimeOwnerSubject runtimeOwnerSubject(String ura) {
    ParsedSubject parsed = parsedSubject(ura);
    if (parsed == null) {
      return null;
    }
    String realm = parsed.realm();
    String path = parsed.path();
    if (realm.isEmpty() || realm.contains("/")) {
      return null;
    }
    if (path.startsWith("authority") && path.equals("authority")) {
      return new RuntimeOwnerSubject("authority", realm);
    }
    if (path.startsWith("device/")) {
      String deviceID = path.substring("device/".length()).trim();
      if (deviceID.isEmpty() || deviceID.contains("/")) {
        return null;
      }
      return new RuntimeOwnerSubject("device", realm);
    }
    return null;
  }

  private static ParsedSubject parsedSubject(String ura) {
    String raw = ura == null ? "" : ura.trim();
    String prefix = "easynet:///r/";
    if (!raw.startsWith(prefix) || RuntimePrincipals.containsAllZeroPrincipal(raw)) {
      return null;
    }
    String rest = raw.substring(prefix.length());
    int slash = rest.indexOf('/');
    if (slash <= 0 || slash == rest.length() - 1) {
      return null;
    }
    String realm = rest.substring(0, slash).trim();
    String path = rest.substring(slash + 1).trim();
    if (realm.isEmpty() || realm.contains("/") || path.isEmpty()) {
      return null;
    }
    return new ParsedSubject(realm, path);
  }

  static boolean canonicalSessionAuthorityID(String sessionID) {
    if (sessionID == null) {
      return false;
    }
    String cleaned = sessionID.trim();
    if (cleaned.isEmpty()) {
      return false;
    }
    for (int index = 0; index < cleaned.length(); index++) {
      char ch = cleaned.charAt(index);
      if ((ch >= 'a' && ch <= 'z')
          || (ch >= 'A' && ch <= 'Z')
          || (ch >= '0' && ch <= '9')
          || ch == '-'
          || ch == '.') {
        continue;
      }
      return false;
    }
    return true;
  }

  private static SDKError invalid(String message) {
    return SDKError.validation("runtime", message);
  }

  record ResourceSubject(String ownerID, String path) {}

  private record RuntimeOwnerSubject(String kind, String realm) {}

  private record ParsedSubject(String realm, String path) {}
}
