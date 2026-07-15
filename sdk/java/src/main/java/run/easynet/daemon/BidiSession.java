package run.easynet.daemon;

import java.util.ArrayDeque;
import java.util.Iterator;
import java.util.List;
import java.util.Objects;

public final class BidiSession implements AutoCloseable, Iterator<BidiFrame> {
  public static final int MAX_RETAINED_FRAMES = 1024;

  private final BidiSource source;
  private final ArrayDeque<BidiFrame> retained = new ArrayDeque<>();
  private boolean closed;
  private boolean sendClosed;
  private BidiFrame terminal;
  private BidiFrame transportTerminal;

  public BidiSession(BidiSource source) {
    this.source = Objects.requireNonNull(source, "source");
  }

  public void send(BidiFrame frame) {
    requireOpen();
    if (sendClosed) {
      throw new SDKError(
          ErrorCode.CANCELLED,
          "bidi",
          RetryHint.NEVER,
          false,
          "bidi send side is closed",
          "",
          "",
          "",
          java.util.Map.of("state", "send_closed"),
          null);
    }
    if (terminal != null) {
      throw SDKError.closed("bidi");
    }
    source.send(Objects.requireNonNull(frame, "frame"));
  }

  @Override
  public boolean hasNext() {
    return !closed && terminal == null && transportTerminal == null;
  }

  @Override
  public BidiFrame next() {
    requireOpen();
    if (terminal != null) {
      return terminal;
    }
    if (transportTerminal != null) {
      return transportTerminal;
    }
    BidiFrame frame = source.next();
    retain(frame);
    if (frame.terminal()) {
      terminal = frame;
    } else if (frame.transportTerminal()) {
      transportTerminal = frame;
    }
    return frame;
  }

  public BidiFrame closeSend() {
    requireOpen();
    if (sendClosed) {
      throw SDKError.closed("bidi_send");
    }
    BidiFrame frame = source.closeSend();
    if (frame == null) {
      throw SDKError.validation("bidi", "bidi source returned null close-send frame");
    }
    sendClosed = true;
    return frame;
  }

  public BidiFrame cancel(String reason) {
    requireOpen();
    BidiFrame frame = source.cancel(reason == null ? "" : reason);
    retain(frame);
    if (frame.terminal()) {
      terminal = frame;
    } else if (frame.transportTerminal()) {
      transportTerminal = frame;
    }
    return frame;
  }

  public List<BidiFrame> retainedFrames() {
    return List.copyOf(retained);
  }

  public BidiFrame terminalFrame() {
    return terminal;
  }

  public BidiFrame transportTerminalFrame() {
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

  private void retain(BidiFrame frame) {
    if (frame == null) {
      throw SDKError.validation("bidi", "bidi source returned null frame");
    }
    if (retained.size() >= MAX_RETAINED_FRAMES && terminal == null && transportTerminal == null) {
      transportTerminal = BidiFrame.transportTerminal(frame.sequence(), "backpressure_terminated");
      retained.addLast(transportTerminal);
      return;
    }
    if ((terminal == null && transportTerminal == null)
        || frame.terminal()
        || frame.transportTerminal()) {
      retained.addLast(frame);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("bidi");
    }
  }
}
