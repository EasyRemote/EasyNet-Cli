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
  private BidiFrame terminal;

  public BidiSession(BidiSource source) {
    this.source = Objects.requireNonNull(source, "source");
  }

  public void send(BidiFrame frame) {
    requireOpen();
    if (terminal != null) {
      throw SDKError.closed("bidi");
    }
    source.send(Objects.requireNonNull(frame, "frame"));
  }

  @Override
  public boolean hasNext() {
    return !closed && terminal == null;
  }

  @Override
  public BidiFrame next() {
    requireOpen();
    if (terminal != null) {
      return terminal;
    }
    BidiFrame frame = source.next();
    retain(frame);
    if (frame.terminal()) {
      terminal = frame;
    }
    return frame;
  }

  public BidiFrame closeSend() {
    requireOpen();
    return source.closeSend();
  }

  public BidiFrame cancel(String reason) {
    requireOpen();
    terminal = source.cancel(reason == null ? "" : reason);
    retain(terminal);
    return terminal;
  }

  public List<BidiFrame> retainedFrames() {
    return List.copyOf(retained);
  }

  public BidiFrame terminalFrame() {
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

  private void retain(BidiFrame frame) {
    if (frame == null) {
      throw SDKError.validation("bidi", "bidi source returned null frame");
    }
    if (retained.size() >= MAX_RETAINED_FRAMES && terminal == null) {
      terminal = BidiFrame.terminal(frame.sequence(), "backpressure_terminated");
      retained.addLast(terminal);
      return;
    }
    if (terminal == null || frame.terminal()) {
      retained.addLast(frame);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("bidi");
    }
  }
}
