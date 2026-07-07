package run.easynet.daemon;

import java.nio.charset.StandardCharsets;
import java.util.Objects;

public final class MissionEventStream implements AutoCloseable {
  private final StreamHandle handle;

  MissionEventStream(StreamHandle handle) {
    this.handle = Objects.requireNonNull(handle, "handle");
  }

  public MissionEvent next() {
    StreamEvent event = handle.next();
    if (event.terminal()) {
      throw MissionSupport.invalid("mission event stream reached terminal state");
    }
    if (event.payloadJson() == null || event.payloadJson().isEmpty()) {
      throw MissionSupport.invalid("mission event stream data requires payload_json");
    }
    return MissionEvent.fromJSON(event.payloadJson().getBytes(StandardCharsets.UTF_8));
  }

  public StreamEvent cancel(String reason) {
    return handle.cancel(reason);
  }

  public StreamEvent terminalEvent() {
    return handle.terminalEvent();
  }

  @Override
  public void close() {
    handle.close();
  }
}
