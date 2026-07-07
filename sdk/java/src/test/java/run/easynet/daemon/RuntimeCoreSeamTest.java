package run.easynet.daemon;

import java.util.ArrayDeque;
import java.util.Map;

public final class RuntimeCoreSeamTest {
  public static void main(String[] args) throws Exception {
    featureDiscoveryAndTypedErrors();
    completeInvocationDraftAndRuntimeDispatch();
    streamHistoryIsBounded();
    bidiHistoryIsBounded();
  }

  private static void featureDiscoveryAndTypedErrors() throws Exception {
    var transport =
        new DiscoveryTransport() {
          boolean closed;

          @Override
          public FeatureSet featureDiscovery() {
            if (closed) {
              throw SDKError.closed("discovery");
            }
            return new FeatureSet(4, "0.0.0-seam", Map.of("runtime_core", "seam"), Map.of());
          }

          @Override
          public void close() {
            closed = true;
          }
        };
    var client = new Client(transport);
    check(client.requireABI(4).profiles().get("runtime_core").equals("seam"), "feature profile");
    client.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, client::featureDiscovery);
    check(SDKError.validation("x", "bad").errorClass() == ErrorClass.VALIDATION, "error class");
  }

  private static void completeInvocationDraftAndRuntimeDispatch() throws Exception {
    var runtime = new RuntimeClient(new MemoryRuntimeTransport());
    var draft =
        runtime
            .newInvocation()
            .caller("easynet:///r/example/agent/alice")
            .callee("easynet:///r/example/agent/bob")
            .descriptor("easynet:///r/example/ability/bob.echo@1.0.0")
            .subject("easynet:///r/example/resource/message")
            .nonce("n-1")
            .causalContext("root")
            .argsJson("{\"text\":\"hi\"}")
            .inspect();
    check(draft.inspectTuple().descriptor().endsWith("@1.0.0"), "descriptor preserved");
    check(runtime.invoke(draft).terminalState() == InvocationTerminalState.COMPLETED, "invoke result");
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> new InvocationBuilder().caller("x").inspect());
    runtime.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, runtime::newInvocation);
  }

  private static void streamHistoryIsBounded() throws Exception {
    var source = new QueueStreamSource(StreamHandle.MAX_RETAINED_EVENTS + 2);
    var handle = new StreamHandle(source);
    for (int i = 0; i < StreamHandle.MAX_RETAINED_EVENTS + 2; i++) {
      handle.next();
    }
    check(handle.terminalEvent() != null, "stream terminal");
    check(handle.terminalEvent().error() != null, "stream typed overflow");
    check(handle.retainedEvents().size() == StreamHandle.MAX_RETAINED_EVENTS + 1, "stream bound");
    handle.close();
  }

  private static void bidiHistoryIsBounded() throws Exception {
    var source = new QueueBidiSource(BidiSession.MAX_RETAINED_FRAMES + 2);
    var session = new BidiSession(source);
    session.send(BidiFrame.data(0, "{\"hello\":true}"));
    for (int i = 0; i < BidiSession.MAX_RETAINED_FRAMES + 2; i++) {
      session.next();
    }
    check(session.terminalFrame() != null, "bidi terminal");
    check(session.terminalFrame().kind().equals("backpressure_terminated"), "bidi overflow");
    check(session.retainedFrames().size() == BidiSession.MAX_RETAINED_FRAMES + 1, "bidi bound");
    session.close();
  }

  private static void expectSDKError(ErrorCode code, ThrowingRunnable action) {
    try {
      action.run();
    } catch (SDKError error) {
      check(error.code() == code, "expected " + code + " got " + error.code());
      return;
    } catch (Exception error) {
      throw new AssertionError("expected SDKError, got " + error, error);
    }
    throw new AssertionError("expected SDKError " + code);
  }

  private static void check(boolean condition, String message) {
    if (!condition) {
      throw new AssertionError(message);
    }
  }

  @FunctionalInterface
  private interface ThrowingRunnable {
    void run() throws Exception;
  }

  private static final class MemoryRuntimeTransport implements RuntimeTransport {
    @Override
    public InvocationResult invoke(InvocationDraft draft) {
      return new InvocationResult(
          true, InvocationTerminalState.COMPLETED, "{\"ok\":true}", null, Map.of());
    }

    @Override
    public StreamSource openStream(InvocationDraft draft) {
      return new QueueStreamSource(1);
    }

    @Override
    public BidiSource openBidi(InvocationDraft draft, BidiFrame frame0) {
      return new QueueBidiSource(1);
    }
  }

  private static final class QueueStreamSource implements StreamSource {
    private final ArrayDeque<StreamEvent> events = new ArrayDeque<>();

    QueueStreamSource(int count) {
      for (int i = 0; i < count; i++) {
        events.add(StreamEvent.data(i, "{\"n\":" + i + "}"));
      }
    }

    @Override
    public StreamEvent next() {
      return events.isEmpty() ? StreamEvent.terminal(9999, "Completed") : events.removeFirst();
    }
  }

  private static final class QueueBidiSource implements BidiSource {
    private final ArrayDeque<BidiFrame> frames = new ArrayDeque<>();

    QueueBidiSource(int count) {
      for (int i = 0; i < count; i++) {
        frames.add(BidiFrame.data(i, "{\"n\":" + i + "}"));
      }
    }

    @Override
    public void send(BidiFrame frame) {}

    @Override
    public BidiFrame next() {
      return frames.isEmpty() ? BidiFrame.terminal(9999, "completed") : frames.removeFirst();
    }
  }
}
