package run.easynet.daemon;

import java.util.Objects;

public final class MissionClient implements AutoCloseable {
  private final MissionTransport transport;
  private boolean closed;

  public MissionClient(MissionTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public InvocationDraft buildRunEALInvocation(MissionRunRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return invocation(raw(() -> transport.buildRunEALInvocation(request.toJSON()), "mission run invocation failed"));
  }

  public InvocationDraft buildRunFileInvocation(MissionRunFileRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return invocation(raw(() -> transport.buildRunFileInvocation(request.toJSON()), "mission run-file invocation failed"));
  }

  public InvocationDraft buildTrackInvocation(MissionTrackRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return invocation(raw(() -> transport.buildTrackInvocation(request.toJSON()), "mission track invocation failed"));
  }

  public InvocationDraft buildCancelInvocation(MissionCancelRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return invocation(raw(() -> transport.buildCancelInvocation(request.toJSON()), "mission cancel invocation failed"));
  }

  public InvocationDraft buildEventsInvocation(MissionEventsRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return invocation(raw(() -> transport.buildEventsInvocation(request.toJSON()), "mission events invocation failed"));
  }

  public MissionRun runEAL(MissionRunRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return new MissionRun(status(raw(() -> transport.runEAL(request.toJSON()), "mission run failed")));
  }

  public MissionRun runFile(MissionRunFileRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return new MissionRun(status(raw(() -> transport.runFile(request.toJSON()), "mission run-file failed")));
  }

  public MissionStatus track(MissionTrackRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return status(raw(() -> transport.track(request.toJSON()), "mission track failed"));
  }

  public MissionStatus cancel(MissionCancelRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return status(raw(() -> transport.cancel(request.toJSON()), "mission cancel failed"));
  }

  public MissionEventPage events(MissionEventsRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return MissionEventPage.fromJSON(raw(() -> transport.events(request.toJSON()), "mission events failed"));
  }

  public MissionEventStream openEventStream(MissionEventsRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return new MissionEventStream(transport.openEventStream(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw MissionSupport.transport("mission event stream failed", error);
    }
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private InvocationDraft invocation(byte[] raw) {
    return InvocationDraft.fromWireObject(JsonValueReader.object(raw, "mission invocation JSON"));
  }

  private MissionStatus status(byte[] raw) {
    return MissionStatus.fromJSON(raw);
  }

  private byte[] raw(MissionBytesOperation operation, String message) {
    try {
      return operation.call();
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw MissionSupport.transport(message, error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed(MissionSupport.PROFILE);
    }
  }

  @FunctionalInterface
  private interface MissionBytesOperation {
    byte[] call();
  }
}
