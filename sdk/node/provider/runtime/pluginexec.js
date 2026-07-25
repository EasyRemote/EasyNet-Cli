// Provider-scoped helper for runtime declarative exec plugins.
//
// This module owns the JSON frame details used between the runtime host and a
// process-backed plugin. Plugin authors should implement handlers over
// SidecarInvocation instead of hand-writing stdin/stdout protocol frames.

import { createInterface } from "node:readline/promises";

export class SidecarProtocolError extends Error {
  constructor(message) {
    super(message);
    this.name = "SidecarProtocolError";
  }
}

export class SidecarInvocation {
  constructor({
    callID,
    callerURA,
    calleeURA,
    abilityURA,
    subjectURA,
    invocationNonce,
    causalContext,
    args,
    frameType = "invoke",
  }) {
    this.callID = requireString(callID, "call_id");
    this.callerURA = requireString(callerURA, "caller_ura");
    this.calleeURA = requireString(calleeURA, "callee_ura");
    this.abilityURA = requireString(abilityURA, "ability_ura");
    this.subjectURA = requireString(subjectURA, "subject_ura");
    this.invocationNonce = Object.freeze(requireNonce(invocationNonce, "invocation_nonce"));
    this.causalContext = immutableSidecarObject(requireObject(causalContext, "causal_context"));
    this.args = immutableSidecarObject(requireObject(args, "args"));
    this.frameType = requireString(frameType, "type");
  }

  static fromFrame(frame) {
    const value = requireObject(frame, "sidecar request frame");
    rejectUnknownRequestFields(value);
    const frameType = requireString(value.type, "type");
    if (frameType !== "invoke") {
      throw new SidecarProtocolError(
        `exec sidecar expected invoke frame, got ${JSON.stringify(frameType)}`,
      );
    }
    const callID = requireString(value.call_id, "call_id");
    const invocation = requireObject(value.invocation, "invocation");
    rejectRetiredTupleFields(invocation);
    rejectUnknownInvocationFields(invocation);
    return new SidecarInvocation({
      callID,
      callerURA: invocation.caller_ura,
      calleeURA: invocation.callee_ura,
      abilityURA: invocation.ability_ura,
      subjectURA: invocation.subject_ura,
      invocationNonce: invocation.invocation_nonce,
      causalContext: invocation.causal_context,
      args: invocation.args,
      frameType,
    });
  }
}

export async function serveExecPlugin(handler, options = {}) {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  let callID = "";
  try {
    const frame = await readFrame(input);
    callID = requireString(frame.call_id, "call_id");
    const invocation = SidecarInvocation.fromFrame(frame);
    const value = await handler(invocation);
    writeFrame(output, {
      type: "result",
      call_id: invocation.callID,
      value,
    });
  } catch (error) {
    writeFrame(output, {
      type: "error",
      call_id: callID,
      message: error instanceof Error ? error.message : String(error),
    });
  }
}

async function readFrame(input) {
  const reader = createInterface({ input, crlfDelay: Infinity });
  try {
    for await (const line of reader) {
      if (line.length === 0) {
        continue;
      }
      try {
        const decoded = JSON.parse(line);
        return requireObject(decoded, "sidecar request frame");
      } catch (error) {
        if (error instanceof SidecarProtocolError) {
          throw error;
        }
        throw new SidecarProtocolError(`invalid sidecar request JSON: ${error.message}`);
      }
    }
    throw new SidecarProtocolError("missing sidecar request frame");
  } finally {
    reader.close();
  }
}

function rejectRetiredTupleFields(value) {
  for (const [retired, canonical] of [
    ["caller", "caller_ura"],
    ["callee", "callee_ura"],
    ["ability", "ability_ura"],
    ["subject", "subject_ura"],
  ]) {
    if (Object.hasOwn(value, retired)) {
      throw new SidecarProtocolError(
        `sidecar frame field ${JSON.stringify(retired)} is retired; use ${JSON.stringify(canonical)}`,
      );
    }
  }
}

function rejectUnknownInvocationFields(value) {
  const allowed = new Set([
    "caller_ura",
    "callee_ura",
    "ability_ura",
    "subject_ura",
    "invocation_nonce",
    "causal_context",
    "args",
  ]);
  for (const field of Object.keys(value)) {
    if (!allowed.has(field)) {
      throw new SidecarProtocolError(
        `sidecar frame field ${JSON.stringify(field)} is not part of the canonical invocation frame`,
      );
    }
  }
}

function rejectUnknownRequestFields(value) {
  const allowed = new Set(["type", "call_id", "invocation"]);
  for (const field of Object.keys(value)) {
    if (!allowed.has(field)) {
      throw new SidecarProtocolError(
        `sidecar request frame field ${JSON.stringify(field)} is not part of the canonical request frame`,
      );
    }
  }
}

function writeFrame(output, frame) {
  output.write(`${JSON.stringify(frame)}\n`);
}

function requireString(value, field) {
  if (typeof value !== "string" || value.length === 0) {
    throw new SidecarProtocolError(`sidecar frame field ${JSON.stringify(field)} must be a string`);
  }
  return value;
}

function requireObject(value, field) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new SidecarProtocolError(`sidecar frame field ${JSON.stringify(field)} must be an object`);
  }
  return value;
}

function requireNonce(value, field) {
  if (!Array.isArray(value) || value.length === 0) {
    throw new SidecarProtocolError(`sidecar frame field ${JSON.stringify(field)} must be a byte array`);
  }
  return value.map((item) => {
    if (!Number.isInteger(item) || item < 0 || item > 255) {
      throw new SidecarProtocolError(
        `sidecar frame field ${JSON.stringify(field)} must contain bytes`,
      );
    }
    return item;
  });
}

function immutableSidecarObject(value) {
  const projected = {};
  for (const [key, item] of Object.entries(value)) {
    projected[key] = immutableSidecarValue(item);
  }
  return Object.freeze(projected);
}

function immutableSidecarValue(value) {
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => immutableSidecarValue(item)));
  }
  if (value !== null && typeof value === "object") {
    return immutableSidecarObject(value);
  }
  return value;
}
