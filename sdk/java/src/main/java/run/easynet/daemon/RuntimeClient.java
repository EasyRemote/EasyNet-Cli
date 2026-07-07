package run.easynet.daemon;

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
    return InvocationHandle.fromJSON(raw).bindRuntime(this);
  }

  public InvocationHandle submitSigned(PreparedInvocation prepared) {
    requireOpen();
    Objects.requireNonNull(prepared, "prepared");
    throw SDKError.validation("runtime", "signed invocation is required");
  }

  public StreamHandle openStream(InvocationDraft draft) {
    requireOpen();
    return new StreamHandle(transport.openStream(Objects.requireNonNull(draft, "draft")));
  }

  public BidiSession openBidi(InvocationDraft draft, BidiFrame frame0) {
    requireOpen();
    return new BidiSession(
        transport.openBidi(Objects.requireNonNull(draft, "draft"), Objects.requireNonNull(frame0)));
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
}
