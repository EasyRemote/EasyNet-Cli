export class SidecarProtocolError extends Error {
  constructor(message: string);
}

export interface SidecarInvocationFields {
  callID: string;
  caller: string;
  callee: string;
  ability: string;
  subject: string;
  invocationNonce: number[];
  causalContext?: unknown;
  args?: Record<string, unknown>;
  frameType?: "invoke";
}

export class SidecarInvocation {
  callID: string;
  caller: string;
  callee: string;
  ability: string;
  subject: string;
  invocationNonce: number[];
  causalContext: unknown;
  args: Record<string, unknown>;
  frameType: string;
  constructor(fields: SidecarInvocationFields);
  static fromFrame(frame: unknown): SidecarInvocation;
}

export type SidecarHandler = (invocation: SidecarInvocation) => unknown | Promise<unknown>;

export interface SidecarReadable {
  [Symbol.asyncIterator](): AsyncIterableIterator<string | Uint8Array>;
}

export interface SidecarWritable {
  write(chunk: string): unknown;
}

export interface ServeExecPluginOptions {
  input?: SidecarReadable;
  output?: SidecarWritable;
}

export function serveExecPlugin(
  handler: SidecarHandler,
  options?: ServeExecPluginOptions,
): Promise<void>;
