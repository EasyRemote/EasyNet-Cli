import { containsAllZeroPrincipal } from "./runtime-principals.js";

const RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read";
const RETIRED_INVOCATION_HISTORY_SUBJECT_PATH = "session/invocation_history";

export function runtimeStateReadSubjectURA(realm, userID, errors = {}) {
  const invalidInvocation = errorFactory(errors.invalidInvocation, TypeError);
  const invalidRuntime = errorFactory(errors.invalidRuntime, TypeError);
  const cleanRealm = runtimeStateSubjectSegment(realm, "realm", invalidInvocation);
  const cleanUserID = runtimeStateSubjectSegment(userID, "user_id", invalidInvocation);
  if (containsAllZeroPrincipal(cleanUserID)) {
    throw invalidInvocation("runtime-state read subject user_id must not be all-zero");
  }
  const subject = `easynet:///r/${cleanRealm}/resource/user.${cleanUserID}/${RUNTIME_STATE_READ_SUBJECT_PATH}`;
  if (!canonicalResourceSubject(subject)) {
    throw invalidRuntime("runtime-state read subject_ura must be canonical");
  }
  return subject;
}

export function isRuntimeStateReadSubjectURA(subjectURA) {
  const subject = canonicalResourceSubject(subjectURA);
  if (!subject || !subject.ownerID.startsWith("user.")) {
    return false;
  }
  const ownerUserID = subject.ownerID.slice("user.".length).trim();
  return (
    ownerUserID !== "" &&
    !containsAllZeroPrincipal(ownerUserID) &&
    subject.resourcePath === RUNTIME_STATE_READ_SUBJECT_PATH
  );
}

export function isRetiredInvocationHistorySubjectURA(subjectURA) {
  const subject = canonicalResourceSubject(subjectURA);
  if (!subject || !subject.ownerID.startsWith("user.")) {
    return false;
  }
  const ownerUserID = subject.ownerID.slice("user.".length).trim();
  return (
    ownerUserID !== "" &&
    !ownerUserID.includes(".") &&
    !containsAllZeroPrincipal(ownerUserID) &&
    subject.resourcePath === RETIRED_INVOCATION_HISTORY_SUBJECT_PATH
  );
}

export function canonicalResourceSubject(subjectURA) {
  if (containsAllZeroPrincipal(subjectURA)) {
    return null;
  }
  const parsed = canonicalURAPath(subjectURA);
  if (!parsed || !parsed.path.startsWith("resource/")) {
    return null;
  }
  const resource = parsed.path.slice("resource/".length);
  const slash = resource.indexOf("/");
  if (slash <= 0 || slash === resource.length - 1) {
    return null;
  }
  const ownerID = resource.slice(0, slash).trim();
  const resourcePath = resource.slice(slash + 1).trim();
  if (
    ownerID === "" ||
    ownerID.includes("/") ||
    resourcePath === "" ||
    resourcePath.startsWith("/") ||
    resourcePath.includes("//")
  ) {
    return null;
  }
  return { ownerID, resourcePath };
}

function runtimeStateSubjectString(value, field, invalidInvocation) {
  if (typeof value !== "string" || value.trim() === "") {
    throw invalidInvocation(`runtime-state read subject ${field} is required`);
  }
  return value.trim();
}

function runtimeStateSubjectSegment(value, field, invalidInvocation) {
  const clean = runtimeStateSubjectString(value, field, invalidInvocation);
  if (clean.includes("/") || clean.includes("?") || clean.includes("#")) {
    throw invalidInvocation(`runtime-state read subject ${field} is not canonical`);
  }
  return clean;
}

function canonicalURAPath(value) {
  const raw = String(value ?? "").trim();
  const prefix = "easynet:///r/";
  if (!raw.startsWith(prefix)) {
    return null;
  }
  const rest = raw.slice(prefix.length);
  const slash = rest.indexOf("/");
  if (slash <= 0 || slash === rest.length - 1) {
    return null;
  }
  return {
    realm: rest.slice(0, slash),
    path: rest.slice(slash + 1),
  };
}

function errorFactory(factory, fallback) {
  if (typeof factory === "function") {
    return factory;
  }
  return (message) => new fallback(message);
}
