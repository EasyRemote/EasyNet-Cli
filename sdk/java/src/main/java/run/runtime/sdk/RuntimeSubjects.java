package run.runtime.sdk;

/** Canonical runtime subject constructors shared by product facades. */
public final class RuntimeSubjects {
  private RuntimeSubjects() {}

  /** Build the user-owned Resource URA used for runtime-state read projections. */
  public static String runtimeStateReadSubjectURA(String realm, String userID) {
    return AuthoritySupport.runtimeStateReadSubjectURA(realm, userID);
  }
}
