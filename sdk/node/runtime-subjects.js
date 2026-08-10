import { containsAllZeroPrincipal } from "./runtime-principals.js";

const RUNTIME_STATE_READ_SUBJECT_PATH = "runtime-state/read";

export function runtimeStateReadSubjectURA(realm, userID, errors) {
  const invalidInvocation = requiredErrorFactory(errors?.invalidInvocation, "invalidInvocation");
  const invalidRuntime = requiredErrorFactory(errors?.invalidRuntime, "invalidRuntime");
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

export function isRuntimeOwnerReadSubjectURA(subjectURA, calleeURA) {
  const subject = String(subjectURA ?? "").trim();
  const callee = String(calleeURA ?? "").trim();
  if (subject === "" || subject !== callee) {
    return false;
  }
  const subjectOwner = canonicalRuntimeOwnerSubject(subject);
  const calleeOwner = canonicalRuntimeOwnerSubject(callee);
  return (
    subjectOwner !== null &&
    calleeOwner !== null &&
    subjectOwner.kind === calleeOwner.kind &&
    subjectOwner.realm === calleeOwner.realm
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

function canonicalRuntimeOwnerSubject(subjectURA) {
  if (containsAllZeroPrincipal(subjectURA)) {
    return null;
  }
  const parsed = canonicalURAPath(subjectURA);
  if (!parsed) {
    return null;
  }
  if (parsed.path === "authority") {
    return { kind: "authority", realm: parsed.realm };
  }
  if (parsed.path.startsWith("device/")) {
    const deviceID = parsed.path.slice("device/".length).trim();
    if (deviceID !== "" && !deviceID.includes("/")) {
      return { kind: "device", realm: parsed.realm };
    }
  }
  if (parsed.path.startsWith("agent/device.")) {
    const scopedAgentID = parsed.path.slice("agent/device.".length).trim();
    const separator = scopedAgentID.indexOf(".");
    if (separator > 0 && separator < scopedAgentID.length - 1 && !scopedAgentID.includes("/")) {
      return { kind: "system-agent", realm: parsed.realm };
    }
  }
  return null;
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

function requiredErrorFactory(factory, name) {
  if (typeof factory === "function") {
    return factory;
  }
  throw new Error(`runtime subject ${name} error factory is required`);
}
