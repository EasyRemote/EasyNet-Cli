package run.runtime.sdk;

import java.util.Map;
import java.util.Objects;

public final class RuntimeClient implements AutoCloseable {
  private final RuntimeTransport transport;
  private boolean closed;

  public RuntimeClient(RuntimeTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public InvocationBuilder newInvocation() {
    requireOpen();
    return new InvocationBuilder();
  }

  public InvocationResult invoke(InvocationDraft draft) {
    requireOpen();
    return transport.invoke(Objects.requireNonNull(draft, "draft"));
  }

  public PreparedInvocation prepare(InvocationDraft draft) {
    return prepare(draft, Map.of());
  }

  public PreparedInvocation prepare(InvocationDraft draft, Map<String, Object> options) {
    requireOpen();
    byte[] raw =
        transport.prepare(
            Objects.requireNonNull(draft, "draft").toJSON(),
            JsonValueWriter.object(Objects.requireNonNullElse(options, Map.of())));
    return PreparedInvocation.fromJSON(raw).bindRuntime(this);
  }

  public InvocationHandle submitSigned(SignedInvocation signed) {
    requireOpen();
    byte[] raw = transport.submitSigned(Objects.requireNonNull(signed, "signed").toJSON());
    return InvocationHandle.fromRuntimeJSON(raw).bindRuntime(this);
  }

  public InvocationHandle submitSigned(PreparedInvocation prepared) {
    requireOpen();
    Objects.requireNonNull(prepared, "prepared");
    throw SDKError.validation("runtime", "signed invocation is required");
  }

  public InvocationResult awaitResult(InvocationHandle handle) {
    requireOpen();
    byte[] raw = transport.awaitHandle(Objects.requireNonNull(handle, "handle").controlCapability());
    return InvocationResult.fromJSON(raw);
  }

  public InvocationCancel cancel(InvocationHandle handle, String reason) {
    requireOpen();
    InvocationControlCapability control = Objects.requireNonNull(handle, "handle").controlCapability();
    byte[] raw =
        transport.cancelHandle(
            control,
            Objects.requireNonNullElse(reason, ""));
    return InvocationCancel.fromJSONWithControl(raw, control);
  }

  public InvocationHandle events(InvocationHandle handle) {
    requireOpen();
    InvocationControlCapability control = Objects.requireNonNull(handle, "handle").controlCapability();
    byte[] raw = transport.handleEvents(control);
    return InvocationHandle.fromJSONWithControl(raw, control).bindRuntime(this);
  }

  public void closeHandle(InvocationHandle handle) {
    requireOpen();
    transport.freeHandle(Objects.requireNonNull(handle, "handle").controlCapability());
  }

  public StreamHandle openStream(InvocationDraft draft) {
    requireOpen();
    return new StreamHandle(transport.openStream(Objects.requireNonNull(draft, "draft")));
  }

  public BidiSession openBidi(InvocationDraft draft, BidiFrame frame0) {
    requireOpen();
    return new BidiSession(
        transport.openBidi(Objects.requireNonNull(draft, "draft"), requireBidiFrameZero(frame0)));
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("runtime");
    }
  }

  static BidiFrame requireBidiFrameZero(BidiFrame frame0) {
    if (frame0 == null) {
      throw SDKError.validation("runtime", "bidi frame0 is required");
    }
    return frame0;
  }
}
