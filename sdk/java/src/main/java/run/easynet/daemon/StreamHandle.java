package run.easynet.daemon;

import java.util.ArrayDeque;
import java.util.Iterator;
import java.util.List;
import java.util.Objects;

public final class StreamHandle implements AutoCloseable, Iterator<StreamEvent> {
  public static final int MAX_RETAINED_EVENTS = 1024;

  private final StreamSource source;
  private final ArrayDeque<StreamEvent> retained = new ArrayDeque<>();
  private boolean closed;
  private StreamEvent terminal;
  private StreamEvent transportTerminal;

  public StreamHandle(StreamSource source) {
    this.source = Objects.requireNonNull(source, "source");
  }

  @Override
  public boolean hasNext() {
    return !closed && terminal == null && transportTerminal == null;
  }

  @Override
  public StreamEvent next() {
    requireOpen();
    if (terminal != null) {
      return terminal;
    }
    if (transportTerminal != null) {
      return transportTerminal;
    }
    StreamEvent event = source.next();
    retain(event);
    if (event.terminal()) {
      terminal = event;
    } else if (event.transportTerminal()) {
      transportTerminal = event;
    }
    return event;
  }

  public StreamEvent cancel(String reason) {
    requireOpen();
    StreamEvent event = source.cancel(reason == null ? "" : reason);
    retain(event);
    if (event.terminal()) {
      terminal = event;
    } else if (event.transportTerminal()) {
      transportTerminal = event;
    }
    return event;
  }

  public List<StreamEvent> retainedEvents() {
    return List.copyOf(retained);
  }

  public StreamEvent terminalEvent() {
    return terminal;
  }

  public StreamEvent transportTerminalEvent() {
    return transportTerminal;
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
    if (retained.size() >= MAX_RETAINED_EVENTS && terminal == null && transportTerminal == null) {
      transportTerminal = StreamEvent.backpressure(event.sequence());
      retained.addLast(transportTerminal);
      return;
    }
    if ((terminal == null && transportTerminal == null)
        || event.terminal()
        || event.transportTerminal()) {
      retained.addLast(event);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("stream");
    }
  }
}
