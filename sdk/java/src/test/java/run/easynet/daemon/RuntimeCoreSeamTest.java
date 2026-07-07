package run.easynet.daemon;

import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Iterator;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

public final class RuntimeCoreSeamTest {
  public static void main(String[] args) throws Exception {
    featureDiscoveryAndTypedErrors();
    completeInvocationDraftAndRuntimeDispatch();
    asyncRuntimeUsesCompletableFutureAndCancellation();
    streamHistoryIsBounded();
    bidiHistoryIsBounded();
    runtimeHealthDistinguishesLivenessFromReadiness();
    runtimeDiagnosticsRequireTransportCapability();
    runtimeHealthWrapsTransportFailures();
    runtimeHealthRejectsMalformedPayload();
    runtimeHealthRejectsClosedClient();
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
    expectSDKError(ErrorCode.VERSION_INCOMPATIBLE, () -> client.requireABI(5));
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

  private static void asyncRuntimeUsesCompletableFutureAndCancellation() throws Exception {
    var executor = Executors.newSingleThreadExecutor();
    try {
      var async = new AsyncRuntimeClient(new MemoryRuntimeTransport(), executor);
      var draft =
          async
              .newInvocation()
              .caller("easynet:///r/example/agent/alice")
              .callee("easynet:///r/example/agent/bob")
              .descriptor("easynet:///r/example/ability/bob.echo@1.0.0")
              .subject("easynet:///r/example/resource/message")
              .nonce("n-2")
              .causalContext("root")
              .argsJson("{\"text\":\"async\"}")
              .inspect();

      var result = async.invokeAsync(draft).get(5, TimeUnit.SECONDS);
      check(result.terminalState() == InvocationTerminalState.COMPLETED, "async invoke result");

      var stream = async.openStreamAsync(draft).get(5, TimeUnit.SECONDS);
      check(stream instanceof Iterator<?>, "stream exposes iterator");
      check(stream.hasNext(), "stream iterator starts open");
      async.cancelStreamAsync(stream, "stop").get(5, TimeUnit.SECONDS);
      check(stream.terminalEvent() != null, "async stream cancel terminal");

      async.close();
      expectSDKError(ErrorCode.INVALID_HANDLE, () -> async.invokeAsync(draft));
    } finally {
      executor.shutdownNow();
    }

    var entered = new CountDownLatch(1);
    var release = new CountDownLatch(1);
    var executor2 = Executors.newSingleThreadExecutor();
    try {
      var async = new AsyncRuntimeClient(new BlockingRuntimeTransport(entered, release), executor2);
      var draft =
          async
              .newInvocation()
              .caller("easynet:///r/example/agent/alice")
              .callee("easynet:///r/example/agent/bob")
              .descriptor("easynet:///r/example/ability/bob.echo@1.0.0")
              .subject("easynet:///r/example/resource/message")
              .nonce("n-3")
              .causalContext("root")
              .argsJson("{\"text\":\"cancel\"}")
              .inspect();
      var future = async.invokeAsync(draft);
      check(entered.await(5, TimeUnit.SECONDS), "blocking transport entered");
      check(future.cancel(true), "future cancel accepted");
      check(future.isCancelled(), "future cancellation observable");
      release.countDown();
      async.close();
    } finally {
      release.countDown();
      executor2.shutdownNow();
    }
  }

  private static void streamHistoryIsBounded() throws Exception {
    var source = new QueueStreamSource(StreamHandle.MAX_RETAINED_EVENTS + 2);
    var handle = new StreamHandle(source);
    check(handle instanceof Iterator<?>, "stream handle is iterator");
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
    check(session instanceof Iterator<?>, "bidi session is iterator");
    session.send(BidiFrame.data(0, "{\"hello\":true}"));
    for (int i = 0; i < BidiSession.MAX_RETAINED_FRAMES + 2; i++) {
      session.next();
    }
    check(session.terminalFrame() != null, "bidi terminal");
    check(session.terminalFrame().kind().equals("backpressure_terminated"), "bidi overflow");
    check(session.retainedFrames().size() == BidiSession.MAX_RETAINED_FRAMES + 1, "bidi bound");
    session.close();
  }

  private static void runtimeHealthDistinguishesLivenessFromReadiness() throws Exception {
    var client =
        new HealthClient(
            new MemoryHealthTransport(
                """
                {
                  "api_ready": true,
                  "daemon_ready": true,
                  "invocation_ready": false,
                  "directory_ready": true,
                  "trust_ready": true,
                  "runtime_ready": false,
                  "version": "0.0.0-seam",
                  "abi_version": 4,
                  "mismatch": null,
                  "diagnostics": ["invocation endpoint unavailable"]
                }
                """,
                """
                {
                  "profile": "health",
                  "kind": "diagnostics_report",
                  "state": "Running",
                  "ready": true,
                  "version": "0.0.0-seam",
                  "abi_version": 4,
                  "control_endpoint": "/tmp/easynet/control.json",
                  "invocation_endpoint": "/tmp/easynet/daemon.sock",
                  "checks": [{"name": "runtime", "ready": true, "message": null}],
                  "diagnostics": []
                }
                """));

    RuntimeHealth health = client.runtimeHealth();
    check(health.apiAlive(), "health api liveness");
    check(!health.ready(), "health runtime readiness");
    check(!health.invocationReady(), "health invocation readiness");
    check(health.abiVersion() == 4, "health ABI version");

    DiagnosticsReport diagnostics = client.diagnostics();
    check(diagnostics.kind().equals("diagnostics_report"), "diagnostics kind");
    check(diagnostics.checks().size() == 1, "diagnostics checks");
  }

  private static void runtimeDiagnosticsRequireTransportCapability() throws Exception {
    var client =
        new HealthClient(
            () ->
                bytes(
                    """
                    {
                      "api_ready": true,
                      "daemon_ready": true,
                      "invocation_ready": true,
                      "directory_ready": true,
                      "trust_ready": true,
                      "runtime_ready": true,
                      "diagnostics": []
                    }
                    """));
    expectSDKError(ErrorCode.NOT_IMPLEMENTED, client::diagnostics);
  }

  private static void runtimeHealthWrapsTransportFailures() throws Exception {
    var client =
        new HealthClient(
            () -> {
              throw new IllegalStateException("transport down");
            });
    expectSDKError(ErrorCode.TRANSPORT, client::runtimeHealth);
  }

  private static void runtimeHealthRejectsMalformedPayload() throws Exception {
    var client = new HealthClient(() -> bytes("{\"api_ready\": true, \"runtime_ready\": false}"));
    expectSDKError(ErrorCode.INVALID_ARGUMENT, client::runtimeHealth);
  }

  private static void runtimeHealthRejectsClosedClient() throws Exception {
    var transport =
        new MemoryHealthTransport(
            """
            {
              "api_ready": true,
              "daemon_ready": true,
              "invocation_ready": true,
              "directory_ready": true,
              "trust_ready": true,
              "runtime_ready": true,
              "diagnostics": []
            }
            """,
            null);
    var client = new HealthClient(transport);
    client.close();
    check(transport.closed, "health transport closed");
    expectSDKError(ErrorCode.INVALID_HANDLE, client::runtimeHealth);
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

  private static final class BlockingRuntimeTransport implements RuntimeTransport {
    private final CountDownLatch entered;
    private final CountDownLatch release;

    BlockingRuntimeTransport(CountDownLatch entered, CountDownLatch release) {
      this.entered = entered;
      this.release = release;
    }

    @Override
    public InvocationResult invoke(InvocationDraft draft) {
      entered.countDown();
      try {
        release.await(5, TimeUnit.SECONDS);
      } catch (InterruptedException error) {
        Thread.currentThread().interrupt();
        throw new SDKError(
            ErrorCode.CANCELLED,
            "runtime",
            RetryHint.NEVER,
            false,
            "async runtime interrupted",
            "",
            "",
            "",
            Map.of(),
            error);
      }
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

  private static final class MemoryHealthTransport
      implements HealthTransport, DiagnosticsTransport {
    private final String health;
    private final String diagnostics;
    private boolean closed;

    MemoryHealthTransport(String health, String diagnostics) {
      this.health = health;
      this.diagnostics = diagnostics;
    }

    @Override
    public byte[] runtimeHealth() {
      if (closed) {
        throw SDKError.closed("health_transport");
      }
      return bytes(health);
    }

    @Override
    public byte[] runtimeDiagnostics() {
      if (diagnostics == null) {
        throw SDKError.validation("health", "diagnostics fixture is required");
      }
      return bytes(diagnostics);
    }

    @Override
    public void close() {
      closed = true;
    }
  }

  private static byte[] bytes(String value) {
    return value.getBytes(StandardCharsets.UTF_8);
  }
}
