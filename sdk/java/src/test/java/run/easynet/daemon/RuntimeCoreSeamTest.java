package run.easynet.daemon;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
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
    bidiCloseSendKeepsReceiveOpenAndRejectsFurtherSend();
    runtimeHealthDistinguishesLivenessFromReadiness();
    runtimeDiagnosticsRequireTransportCapability();
    runtimeHealthWrapsTransportFailures();
    runtimeHealthRejectsMalformedPayload();
    runtimeHealthRejectsClosedClient();
    directoryIdentityBuildsCarriersAndProjectsReadModels();
    directorySubscriptionUsesStreamLifecycle();
    directoryIdentityRejectsInvalidState();
    identityDescriptorHelpersDelegateToTransport();
    receiptBuildsFetchCarrierAndProjectsSummary();
    receiptRejectsInvalidSelectorAndSummaryVerification();
    receiptOpaqueRefRequiresExplicitAnchorFacts();
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

  private static void bidiCloseSendKeepsReceiveOpenAndRejectsFurtherSend() throws Exception {
    var session = new BidiSession(new QueueBidiSource(1));
    session.send(BidiFrame.data(0, "{\"hello\":true}"));
    check(session.closeSend().kind().equals("send_closed"), "bidi close-send frame");
    expectSDKError(ErrorCode.CANCELLED, () -> session.send(BidiFrame.data(1, "{\"after\":true}")));
    check(session.next().payloadJson().contains("\"n\":0"), "bidi receive after close-send");
    session.close();
    session.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, session::next);
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

  private static void directoryIdentityBuildsCarriersAndProjectsReadModels() throws Exception {
    var directory = new DirectoryClient(new FixtureDirectoryTransport());
    var base =
        new DirectoryQueryBase(
            "easynet:///r/example/agent/alice.sdk",
            "easynet:///r/example/device/dev-a",
            "easynet:///r/example/device/dev-a",
            "1.0.0",
            "AQIDBAUGBwgJCgsMDQ4PEA==",
            Map.of("form", "none"),
            2,
            "0",
            Map.of("request_id", "directory-list-devices-1"));

    var deviceCarrier = directory.buildListDevicesInvocation(base);
    check(
        deviceCarrier
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/device.dev-a.node.list@1.0.0"),
        "directory device carrier descriptor");
    check(directory.listDevices(base).kind().equals("device_page"), "directory device page");

    var agentBase =
        new DirectoryQueryBase(
            base.callerURA(),
            base.calleeURA(),
            base.subjectURA(),
            base.descriptorVersion(),
            base.nonceBase64(),
            base.causalContext(),
            base.limit(),
            base.cursor(),
            Map.of("request_id", "directory-list-agents-1"));
    check(directory.buildListAgentsInvocation(agentBase).get("descriptor_ref").toString().contains("agent.list"), "directory agent carrier");
    check(directory.listAgents(agentBase).itemKind().equals("agent"), "directory agent page");

    var abilityBase =
        new DirectoryQueryBase(
            base.callerURA(),
            base.calleeURA(),
            base.subjectURA(),
            base.descriptorVersion(),
            base.nonceBase64(),
            base.causalContext(),
            base.limit(),
            base.cursor(),
            Map.of("request_id", "directory-list-abilities-1"));
    var abilityQuery =
        new AbilityQuery(
            abilityBase,
            "local",
            "easynet:///r/example/device/dev-a",
            "easynet:///r/example/ability/device.dev-a.fs.read");
    var abilityCarrier = directory.buildListAbilitiesInvocation(abilityQuery);
    check(abilityCarrier.get("descriptor_ref").toString().contains("meta.list_abilities"), "directory ability carrier");
    check(directory.listAbilities(abilityQuery).kind().equals("ability_page"), "directory ability page");

    var resolveQuery =
        new ResolveQuery(base, "easynet:///r/example/device/dev-a", "agent.list", "route", null, null);
    check(directory.buildResolveInvocation(resolveQuery).get("descriptor_ref").toString().contains("namespace.resolve"), "directory resolve carrier");
    check(
        directory.resolve(resolveQuery).abilityURA().equals("easynet:///r/example/ability/device.dev-a.agent.list"),
        "directory resolved ref");
  }

  private static void directorySubscriptionUsesStreamLifecycle() throws Exception {
    var directory = new DirectoryClient(new FixtureDirectoryTransport());
    var request =
        new DirectorySubscriptionRequest(
            "easynet:///r/example/agent/alice.sdk",
            "easynet:///r/example/device/dev-a",
            "easynet:///r/example/device/dev-a",
            "1.0.0",
            "AQIDBAUGBwgJCgsMDQ4PEA==",
            Map.of("form", "none"),
            "directory",
            null,
            null,
            null,
            null,
            null,
            "ability",
            null,
            null,
            Map.of("request_id", "directory-subscribe"));

    var carrier = directory.buildDirectorySubscriptionInvocation(request);
    check(
        carrier.get("descriptor_ref").toString().contains("directory.subscribe"),
        "directory subscription carrier");

    var projection = directory.projectSubscription(fixture("directory-subscription.v4.json"));
    check(projection.state().equals("Live"), "directory subscription state");
    check(projection.resumeToken().equals("directory:3"), "directory subscription cursor");
    check(projection.events().size() == 3, "directory subscription buffered events");
    check(projection.events().get(2).phase().equals("live"), "directory subscription live event");

    var stream = directory.subscribeDirectory(request);
    var first = stream.next();
    var second = stream.next();
    check(first.payloadJson().contains("\"phase\":\"live\""), "directory subscription stream event");
    check(second.terminal(), "directory subscription stream terminal");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new DirectorySubscriptionRequest(
                request.callerURA(),
                request.calleeURA(),
                request.subjectURA(),
                request.descriptorVersion(),
                request.nonceBase64(),
                request.causalContext(),
                "device",
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                null,
                Map.of()));
  }

  private static void directoryIdentityRejectsInvalidState() throws Exception {
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new DirectoryQueryBase(
                "easynet:///r/example/agent/alice.sdk",
                "easynet:///r/example/device/dev-a",
                "easynet:///r/example/device/dev-a",
                "1.0.0",
                "AQIDBAUGBwgJCgsMDQ4PEA==",
                Map.of("form", "none"),
                DirectoryQueryBase.MAX_PAGE_SIZE + 1,
                "",
                Map.of()));

    var directory = new DirectoryClient(new DirectoryTransport() {});
    var base =
        new DirectoryQueryBase(
            "easynet:///r/example/agent/alice.sdk",
            "easynet:///r/example/device/dev-a",
            "easynet:///r/example/device/dev-a",
            "1.0.0",
            "AQIDBAUGBwgJCgsMDQ4PEA==",
            Map.of("form", "none"),
            2,
            "0",
            Map.of());
    expectSDKError(ErrorCode.NOT_IMPLEMENTED, () -> directory.listDevices(base));
    directory.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, () -> directory.listAgents(base));
  }

  private static void identityDescriptorHelpersDelegateToTransport() throws Exception {
    var identity = new IdentityClient(new FixtureIdentityTransport());
    var projection =
        identity.projectDescriptorRef(
            new DescriptorRefRequest(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0", Map.of()));
    check(projection.valid(), "identity projection valid");
    check(
        identity
            .abilityURAFromDescriptorRef(
                "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
            .equals("easynet:///r/example/ability/device.dev-a.observe.health"),
        "identity ability URA");
    check(
        identity
            .ownerAbilityDescriptorRef(
                "easynet:///r/example/device/dev-a", "observe.health", "1.0.0")
            .equals("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0"),
        "identity owner descriptor ref");
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> identity.canonicalAbilityDescriptorRef("", ""));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> identity.projectDescriptorRef(new DescriptorRefRequest("not-a-descriptor", Map.of())));
    identity.close();
    expectSDKError(
        ErrorCode.INVALID_HANDLE,
        () ->
            identity.projectDescriptorRef(
                new DescriptorRefRequest(
                    "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                    Map.of())));
  }

  private static void receiptBuildsFetchCarrierAndProjectsSummary() throws Exception {
    var client = new ReceiptClient(new FixtureReceiptTransport());
    var carrier = client.buildFetchInvocation(receiptFetchRequest());
    var expected = JsonValueReader.object(fixture("receipt-fetch-invocation.v4.json"), "receipt fetch invocation");
    check(carrier.equals(expected), "receipt fetch carrier");

    var fetched = client.fetch(receiptFetchRequest());
    check(fetched.state().equals("completed"), "receipt fetch summary state");
    check(!fetched.verified(), "receipt fetch summary is not cryptographic proof");

    var projected = client.project(fixture("receipt.summary.v4.json"));
    check(projected.invocationID().equals("inv-example-1"), "receipt projection invocation id");
    check(!client.verifySummary(projected).verified(), "receipt summary verification claim");
  }

  private static void receiptRejectsInvalidSelectorAndSummaryVerification() throws Exception {
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new ReceiptFetchRequest(
                "easynet:///r/example/agent/alice.sdk",
                "easynet:///r/example/device/dev-a",
                "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
                "easynet:///r/example/device/dev-a",
                "1.0.0",
                "AQIDBAUGBwgJCgsMDQ4PEA==",
                Map.of("form", "none"),
                "",
                "",
                "",
                Map.of()));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new ReceiptFetchRequest(
                "easynet:///r/example/agent/alice.sdk",
                "easynet:///r/example/device/dev-a",
                "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
                "easynet:///r/example/device/dev-a",
                "1.0.0",
                "AQIDBAUGBwgJCgsMDQ4PEA==",
                Map.of("form", "none"),
                "easynet:///r/example/invocation/inv-example-1",
                "inv-example-1",
                "",
                Map.of()));

    var client = new ReceiptClient(new FixtureReceiptTransport());
    var summary = client.project(fixture("receipt.summary.v4.json"));
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> client.verifySummary(summary).requireCryptographic());
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> ReceiptRef.fromSummary(summary));
  }

  private static void receiptOpaqueRefRequiresExplicitAnchorFacts() throws Exception {
    var ref = ReceiptRef.fromJSON(fixture("receipt-ref.v4.json"));
    check(ref.receiptURA().equals("easynet:///r/example/receipt/receipt-1"), "receipt ref URA");
    check(ref.receiptHashHex().length() == 64, "receipt ref hash");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new ReceiptRef(
                "easynet:///r/example/receipt/receipt-1",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "inv-example-1",
                "",
                0,
                Map.of()));
    var client = new ReceiptClient(new ReceiptTransport() {});
    expectSDKError(ErrorCode.NOT_IMPLEMENTED, () -> client.causalRef(ref));
  }

  private static ReceiptFetchRequest receiptFetchRequest() {
    return new ReceiptFetchRequest(
        "easynet:///r/example/agent/alice.sdk",
        "easynet:///r/example/device/dev-a",
        "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
        "easynet:///r/example/device/dev-a",
        "1.0.0",
        "AQIDBAUGBwgJCgsMDQ4PEA==",
        Map.of("form", "none"),
        "",
        "inv-example-1",
        "",
        Map.of("request_id", "receipt-fetch-1"));
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

  private static final class QueueDirectoryStreamSource implements StreamSource {
    private final ArrayDeque<StreamEvent> events = new ArrayDeque<>();

    QueueDirectoryStreamSource() {
      events.add(
          StreamEvent.data(
              3,
              """
              {"profile":"directory_identity","stream":"directory","kind":"upsert","event_id":"evt-3","phase":"live","cursor":{"stream":"directory","sequence":3,"token":"directory:3"},"resume_token":"directory:3","terminal":false,"metadata":{"source":"directory.subscribe"}}
              """));
      events.add(StreamEvent.terminal(4, "Closed"));
    }

    @Override
    public StreamEvent next() {
      return events.removeFirst();
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

  private static byte[] fixture(String name) {
    try {
      return Files.readAllBytes(Path.of("sdk/conformance/fixtures", name));
    } catch (IOException error) {
      throw new AssertionError("fixture not found: " + name, error);
    }
  }

  private static final class FixtureDirectoryTransport implements DirectoryTransport {
    @Override
    public byte[] buildDirectorySubscriptionInvocation(byte[] requestJSON) {
      String request = new String(requestJSON, StandardCharsets.UTF_8);
      check(request.contains("\"stream\":\"directory\""), "directory subscription stream");
      check(!request.contains("\"limit\""), "directory subscription omits pagination limit");
      return fixture("directory-subscription-invocation.v4.json");
    }

    @Override
    public byte[] buildListDevicesInvocation(byte[] requestJSON) {
      check(new String(requestJSON, StandardCharsets.UTF_8).contains("\"limit\":2"), "device request limit");
      return fixture("directory-list-devices-invocation.v4.json");
    }

    @Override
    public byte[] buildListAgentsInvocation(byte[] requestJSON) {
      return fixture("directory-list-agents-invocation.v4.json");
    }

    @Override
    public byte[] buildListAbilitiesInvocation(byte[] requestJSON) {
      String request = new String(requestJSON, StandardCharsets.UTF_8);
      check(request.contains("\"scope\":\"local\""), "ability query scope");
      return fixture("directory-list-abilities-invocation.v4.json");
    }

    @Override
    public byte[] buildResolveInvocation(byte[] requestJSON) {
      return fixture("directory-resolve-invocation.v4.json");
    }

    @Override
    public byte[] listDevices(byte[] requestJSON) {
      return fixture("directory-device-page.v4.json");
    }

    @Override
    public byte[] listAgents(byte[] requestJSON) {
      return fixture("directory-agent-page.v4.json");
    }

    @Override
    public byte[] listAbilities(byte[] requestJSON) {
      return fixture("directory-ability-page.v4.json");
    }

    @Override
    public byte[] resolve(byte[] requestJSON) {
      return fixture("directory-resolved-ref.v4.json");
    }

    @Override
    public StreamSource subscribeDirectory(byte[] requestJSON) {
      String request = new String(requestJSON, StandardCharsets.UTF_8);
      check(request.contains("\"item_kind\":\"ability\""), "directory subscription item kind");
      return new QueueDirectoryStreamSource();
    }

    @Override
    public byte[] projectSubscription(byte[] subscriptionJSON) {
      return subscriptionJSON;
    }
  }

  private static final class FixtureIdentityTransport implements IdentityTransport {
    @Override
    public byte[] projectDescriptorRef(byte[] requestJSON) {
      String request = new String(requestJSON, StandardCharsets.UTF_8);
      if (request.contains("not-a-descriptor")) {
        throw IdentityProjection.invalid("descriptor_ref is invalid");
      }
      return fixture("identity.descriptor-ref.v4.json");
    }

    @Override
    public byte[] buildDescriptorRef(byte[] requestJSON) {
      return fixture("identity.descriptor-ref.v4.json");
    }

    @Override
    public byte[] ownerAbilityURA(byte[] requestJSON) {
      return bytes(
          """
          {
            "ability_ura": "easynet:///r/example/ability/device.dev-a.observe.health"
          }
          """);
    }
  }

  private static final class FixtureReceiptTransport implements ReceiptTransport {
    @Override
    public byte[] fetch(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "receipt fetch request");
      check(request.equals(JsonValueReader.object(fixture("receipt-fetch-request.v4.json"), "receipt fetch fixture")), "receipt fetch request");
      return fixture("receipt.summary.v4.json");
    }
  }
}
