package run.runtime.sdk;

import java.util.Objects;
import java.util.concurrent.Executor;
import java.util.concurrent.ForkJoinPool;

public final class AsyncRuntimeClient implements AutoCloseable {
  private final RuntimeClient runtime;
  private final Executor executor;
  private boolean closed;

  public AsyncRuntimeClient(RuntimeTransport transport) {
    this(transport, ForkJoinPool.commonPool());
  }

  public AsyncRuntimeClient(RuntimeTransport transport, Executor executor) {
    this.runtime = new RuntimeClient(Objects.requireNonNull(transport, "transport"));
    this.executor = Objects.requireNonNull(executor, "executor");
  }

  public InvocationBuilder newInvocation() {
    requireOpen();
    return runtime.newInvocation();
  }

  public RuntimeFuture<InvocationResult> invokeAsync(InvocationDraft draft) {
    requireOpen();
    Objects.requireNonNull(draft, "draft");
    return RuntimeFuture.supply(() -> runtime.invoke(draft), () -> {}, executor);
  }

  public RuntimeFuture<StreamHandle> openStreamAsync(InvocationDraft draft) {
    requireOpen();
    Objects.requireNonNull(draft, "draft");
    return RuntimeFuture.supply(() -> runtime.openStream(draft), () -> {}, executor);
  }

  public RuntimeFuture<BidiSession> openBidiAsync(InvocationDraft draft, BidiFrame frame0) {
    requireOpen();
    Objects.requireNonNull(draft, "draft");
    RuntimeClient.requireBidiFrameZero(frame0);
    return RuntimeFuture.supply(() -> runtime.openBidi(draft, frame0), () -> {}, executor);
  }

  public RuntimeFuture<StreamEvent> cancelStreamAsync(StreamHandle handle, String reason) {
    requireOpen();
    Objects.requireNonNull(handle, "handle");
    return RuntimeFuture.supply(() -> handle.cancel(reason), () -> handle.cancel(reason), executor);
  }

  public RuntimeFuture<BidiFrame> cancelBidiAsync(BidiSession session, String reason) {
    requireOpen();
    Objects.requireNonNull(session, "session");
    return RuntimeFuture.supply(() -> session.cancel(reason), () -> session.cancel(reason), executor);
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    runtime.close();
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("async_runtime");
    }
  }
}
