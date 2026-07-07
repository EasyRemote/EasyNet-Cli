package run.easynet.daemon;

import java.util.ArrayDeque;
import java.util.List;
import java.util.Objects;

public final class StreamHandle implements AutoCloseable {
  public static final int MAX_RETAINED_EVENTS = 1024;

  private final StreamSource source;
  private final ArrayDeque<StreamEvent> retained = new ArrayDeque<>();
  private boolean closed;
  private StreamEvent terminal;

  public StreamHandle(StreamSource source) {
    this.source = Objects.requireNonNull(source, "source");
  }

  public StreamEvent next() {
    requireOpen();
    if (terminal != null) {
      return terminal;
    }
    StreamEvent event = source.next();
    retain(event);
    if (event.terminal()) {
      terminal = event;
    }
    return event;
  }

  public StreamEvent cancel(String reason) {
    requireOpen();
    terminal = source.cancel(reason == null ? "" : reason);
    retain(terminal);
    return terminal;
  }

  public List<StreamEvent> retainedEvents() {
    return List.copyOf(retained);
  }

  public StreamEvent terminalEvent() {
    return terminal;
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    source.close();
  }

  private void retain(StreamEvent event) {
    if (event == null) {
      throw SDKError.validation("stream", "stream source returned null event");
    }
    if (retained.size() >= MAX_RETAINED_EVENTS && terminal == null) {
      terminal = StreamEvent.backpressure(event.sequence());
      retained.addLast(terminal);
      return;
    }
    if (terminal == null || event.terminal()) {
      retained.addLast(event);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("stream");
    }
  }
}
