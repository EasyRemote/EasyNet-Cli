package run.easynet.daemon;

import java.util.Objects;

public final class HostStreamLifecycleController implements AutoCloseable {
  private final HostStreamBinding binding;
  private final HostStreamLifecycleProvider provider;
  private HostStreamLifecycleState state = HostStreamLifecycleState.DECLARED;
  private HostStreamReadiness readiness;
  private HostStreamCleanup cleanupResult;

  public HostStreamLifecycleController(HostStreamBinding binding, HostStreamLifecycleProvider provider) {
    this.binding = Objects.requireNonNull(binding, "binding");
    this.provider = Objects.requireNonNull(provider, "provider");
    this.readiness = HostStreamReadiness.fromMap(binding.readiness());
  }

  public synchronized HostStreamLifecycleState state() {
    return state;
  }

  public synchronized HostStreamReadiness readiness() {
    return readiness;
  }

  public synchronized HostStreamCleanup cleanupResult() {
    return cleanupResult;
  }

  public HostStreamReadiness checkReadiness() {
    synchronized (this) {
      if (state == HostStreamLifecycleState.CLEANING
          || state == HostStreamLifecycleState.CLEANED
          || state == HostStreamLifecycleState.CLOSED) {
        throw HostBindingSupport.invalid("host stream lifecycle is not readable");
      }
      state = HostStreamLifecycleState.CHECKING;
    }
    try {
      HostStreamReadiness checked = provider.checkReadiness(binding);
      synchronized (this) {
        readiness = checked;
        state = Boolean.TRUE.equals(checked.endpointReady())
            ? HostStreamLifecycleState.READY
            : HostStreamLifecycleState.NOT_READY;
      }
      return checked;
    } catch (SDKError error) {
      synchronized (this) {
        state = HostStreamLifecycleState.FAILED;
      }
      throw error;
    } catch (RuntimeException error) {
      synchronized (this) {
        state = HostStreamLifecycleState.FAILED;
      }
      throw HostBindingSupport.transport("host binding readiness provider failed", error);
    }
  }

  public HostStreamCleanup cleanup() {
    synchronized (this) {
      if (state == HostStreamLifecycleState.CLEANED || state == HostStreamLifecycleState.CLOSED) {
        return cleanupResult == null ? HostStreamCleanup.fromMap(binding.cleanup()) : cleanupResult;
      }
      if (state == HostStreamLifecycleState.CLEANING) {
        throw HostBindingSupport.invalid("host stream lifecycle cleanup is already running");
      }
      if (state == HostStreamLifecycleState.CHECKING) {
        throw HostBindingSupport.invalid("host stream lifecycle readiness check is running");
      }
      state = HostStreamLifecycleState.CLEANING;
    }
    try {
      HostStreamCleanup cleaned = provider.cleanup(binding);
      synchronized (this) {
        cleanupResult = cleaned;
        state = HostStreamLifecycleState.CLEANED;
      }
      return cleaned;
    } catch (SDKError error) {
      synchronized (this) {
        state = HostStreamLifecycleState.FAILED;
      }
      throw error;
    } catch (RuntimeException error) {
      synchronized (this) {
        state = HostStreamLifecycleState.FAILED;
      }
      throw HostBindingSupport.transport("host binding cleanup provider failed", error);
    }
  }

  @Override
  public void close() {
    synchronized (this) {
      if (state == HostStreamLifecycleState.CLOSED) {
        return;
      }
    }
    cleanup();
    synchronized (this) {
      state = HostStreamLifecycleState.CLOSED;
    }
  }
}
