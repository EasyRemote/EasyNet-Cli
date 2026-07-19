// Provider-scoped helper for EasyNet-Cli declarative exec plugins.
//
// This module owns the JSON frame details used between `easynet-daemon` and a
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
    caller,
    callee,
    ability,
    subject,
    invocationNonce,
    causalContext,
    args,
    frameType = "invoke",
  }) {
    this.callID = requireString(callID, "call_id");
    this.caller = requireString(caller, "caller");
    this.callee = requireString(callee, "callee");
    this.ability = requireString(ability, "ability");
    this.subject = requireString(subject, "subject");
    this.invocationNonce = requireNonce(invocationNonce, "invocation_nonce");
    this.causalContext = causalContext ?? {};
    this.args = optionalObject(args, "args");
    this.frameType = requireString(frameType, "type");
  }

  static fromFrame(frame) {
    const value = optionalObject(frame, "sidecar request frame", { required: true });
    const frameType = requireString(value.type, "type");
    if (frameType !== "invoke") {
      throw new SidecarProtocolError(
        `exec sidecar expected invoke frame, got ${JSON.stringify(frameType)}`,
      );
    }
    const callID = requireString(value.call_id, "call_id");
    const invocation = optionalObject(value.invocation, "invocation", { required: true });
    return new SidecarInvocation({
      callID,
      caller: invocation.caller,
      callee: invocation.callee,
      ability: invocation.ability,
      subject: invocation.subject,
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
        return optionalObject(decoded, "sidecar request frame", { required: true });
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

function writeFrame(output, frame) {
  output.write(`${JSON.stringify(frame)}\n`);
}

function requireString(value, field) {
  if (typeof value !== "string" || value.length === 0) {
    throw new SidecarProtocolError(`sidecar frame field ${JSON.stringify(field)} must be a string`);
  }
  return value;
}

function optionalObject(value, field, { required = false } = {}) {
  if (value == null && !required) {
    return {};
  }
  if (typeof value !== "object" || Array.isArray(value)) {
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
