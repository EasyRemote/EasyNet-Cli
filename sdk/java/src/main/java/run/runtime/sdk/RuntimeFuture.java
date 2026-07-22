package run.runtime.sdk;

import java.util.Objects;
import java.util.concurrent.Callable;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;
import java.util.concurrent.atomic.AtomicBoolean;

public final class RuntimeFuture<T> extends CompletableFuture<T> {
  private final Runnable cancelAction;
  private final AtomicBoolean cancelActionStarted = new AtomicBoolean();

  private RuntimeFuture(Runnable cancelAction) {
    this.cancelAction = cancelAction == null ? () -> {} : cancelAction;
  }

  static <T> RuntimeFuture<T> supply(Callable<T> action, Runnable cancelAction, Executor executor) {
    Objects.requireNonNull(action, "action");
    Objects.requireNonNull(executor, "executor");
    RuntimeFuture<T> future = new RuntimeFuture<>(cancelAction);
    executor.execute(
        () -> {
          if (future.isCancelled()) {
            return;
          }
          try {
            future.complete(action.call());
          } catch (Throwable error) {
            future.completeExceptionally(error);
          }
        });
    return future;
  }

  @Override
  public boolean cancel(boolean mayInterruptIfRunning) {
    boolean accepted = super.cancel(mayInterruptIfRunning);
    if (accepted && cancelActionStarted.compareAndSet(false, true)) {
      cancelAction.run();
    }
    return accepted;
  }
}
