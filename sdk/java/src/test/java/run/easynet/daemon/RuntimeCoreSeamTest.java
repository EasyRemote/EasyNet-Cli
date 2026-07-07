package run.easynet.daemon;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

public final class RuntimeCoreSeamTest {
  public static void main(String[] args) throws Exception {
    featureDiscoveryAndTypedErrors();
    completeInvocationDraftAndRuntimeDispatch();
    preparedInvocationSeparatesCanonicalMaterialFromSignedSubmit();
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
    publicationProfileDelegatesResourceValidationAndCarriers();
    missionProfileDelegatesCarriersStatusAndStreams();
    hostBindingProfileDelegatesCodecHashAndLifecycle();
    eventsProfileDelegatesCarriersProjectionsHistoryAndStreams();
    surfaceProfileDelegatesCarriersAndProjections();
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

  private static void preparedInvocationSeparatesCanonicalMaterialFromSignedSubmit() throws Exception {
    final class SigningTransport extends MemoryRuntimeTransport {
      Map<String, Object> seenDraft = Map.of();
      Map<String, Object> seenOptions = Map.of();
      Map<String, Object> seenSigned = Map.of();
      boolean submitTouched;

      @Override
      public byte[] prepare(byte[] draftJson, byte[] optionsJson) {
        seenDraft = JsonValueReader.object(draftJson, "draft");
        seenOptions = JsonValueReader.object(optionsJson, "options");
        return fixture("prepared.signing-material.v4.json");
      }

      @Override
      public byte[] submitSigned(byte[] signedJson) {
        submitTouched = true;
        seenSigned = JsonValueReader.object(signedJson, "signed");
        return bytes("{\"handle_id\":7,\"state\":\"Submitted\",\"terminal\":false}");
      }
    }

    var transport = new SigningTransport();
    var runtime = new RuntimeClient(transport);
    var draft =
        runtime
            .newInvocation()
            .caller("easynet:///r/example/agent/alice.sdk")
            .callee("easynet:///r/example/device/dev-a")
            .descriptor("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0")
            .subject("easynet:///r/example/device/dev-a")
            .nonce("AQIDBAUGBwgJCgsMDQ4PEA==")
            .causalContext("{\"form\":\"none\"}")
            .argsJson("{}")
            .inspect();

    var prepared = runtime.prepare(draft, Map.of("deadline_unix_ms", 1783000000000L));
    check(!prepared.submitReady(), "prepared is not submit-ready");
    check(prepared.preparedId().equals("prepared-example-1"), "prepared id");
    check(
        prepared.signingMaterial().canonicalBytesBase64().equals("ZXhhbXBsZS1jYW5vbmljYWwtYnl0ZXM="),
        "canonical bytes");
    check(transport.seenDraft.get("descriptor_ref").equals(prepared.signingMaterial().descriptorRef()), "prepare descriptor");
    check(transport.seenOptions.get("deadline_unix_ms").equals(1783000000000L), "prepare options");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            PreparedInvocation.fromJSON(
                bytes(
                    new String(fixture("prepared.signing-material.v4.json"), StandardCharsets.UTF_8)
                        .replace("\"submit_ready\": false", "\"submit_ready\": true"))));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            PreparedInvocation.fromJSON(
                bytes(
                    new String(fixture("prepared.signing-material.v4.json"), StandardCharsets.UTF_8)
                        .replace(
                            """
                                "args_digest_hex": "0000000000000000000000000000000000000000000000000000000000000000",
                                "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
                            """,
                            """
                                "args_digest_hex": "0000000000000000000000000000000000000000000000000000000000000000",
                                "descriptor_ref": "easynet:///r/example/ability/other@1.0.0",
                            """))));

    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.submitSigned(prepared));
    check(!transport.submitTouched, "prepared submit rejected before transport");

    var signed =
        prepared.signWithCallerSignature(
            new InvocationSignature("ed25519", "c2lnbmF0dXJl", "signer-alice-key-1", ""));
    check(signed.submitReady(), "signed submit-ready");
    var handle = runtime.submitSigned(signed);
    check(handle.handleId() == 7, "handle id");
    check(transport.submitTouched, "signed submit reached transport");
    @SuppressWarnings("unchecked")
    var signedPrepared = (Map<String, Object>) transport.seenSigned.get("prepared");
    @SuppressWarnings("unchecked")
    var signature = (Map<String, Object>) transport.seenSigned.get("signature");
    check(transport.seenSigned.get("signer_id").equals("signer-alice-key-1"), "signer id preserved");
    check(signature.get("signature_base64").equals("c2lnbmF0dXJl"), "signature preserved");
    check(
        signedPrepared.get("canonical_bytes_base64").equals(prepared.signingMaterial().canonicalBytesBase64()),
        "canonical material preserved");
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

  private static void publicationProfileDelegatesResourceValidationAndCarriers() throws Exception {
    var publication = new PublicationClient(new FixturePublicationTransport());
    var resource = publication.buildLocalResourceRef(new LocalResourceRefRequest("/tmp/easynet-weather-package", "read"));
    check(resource.namespace().equals("fs"), "publication resource namespace");
    check(resource.capability().equals("read"), "publication resource capability");

    var manifest =
        new AbilityPackageManifest(
            "weather",
            "er",
            "Weather stream",
            Map.of("type", "object", "properties", Map.of()),
            Map.of("kind", "host_stream", "host_socket", "/tmp/easynet-weather.sock", "function", "weather.stream"));
    var validation = publication.validatePackage("", new ValidatePackageOptions(manifest));
    check(validation.valid(), "publication validation valid");
    check(validation.manifest().wireKey().equals("er.weather"), "publication validation wire key");
    check(validation.metadata().get("frame_contract_owner").equals("daemon_sdk"), "publication metadata owner");

    var deployRequest =
        new AbilityDeployRequest(
            "easynet:///r/example/agent/alice.sdk",
            "easynet:///r/example/device/dev-a",
            "easynet:///r/example/device/dev-a",
            "1.0.0",
            "AQIDBAUGBwgJCgsMDQ4PEA==",
            Map.of("form", "none"),
            resource,
            "local",
            Map.of("request_id", "publication-deploy-1"));
    var deploy = publication.buildDeployInvocation(deployRequest);
    check(
        deploy.get("descriptor_ref").equals("easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0"),
        "publication deploy descriptor");
    @SuppressWarnings("unchecked")
    var deployMetadata = (Map<String, Object>) deploy.get("metadata");
    check(deployMetadata.get("system_ability").equals("ability.deploy"), "publication deploy system ability");

    var unpublish =
        publication.buildUnpublishInvocation(
            new UnpublishAbilityRequest(
                "easynet:///r/example/agent/alice.sdk",
                "easynet:///r/example/device/dev-a",
                "easynet:///r/example/device/dev-a",
                "1.0.0",
                "AQIDBAUGBwgJCgsMDQ4PEA==",
                Map.of("form", "none"),
                "easynet:///r/example/ability/device.dev-a.er.weather",
                Map.of()));
    check(
        unpublish.get("descriptor_ref").equals("easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0"),
        "publication unpublish descriptor");

    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> new LocalResourceRefRequest("tmp/easynet-weather-package", "read"));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            publication.buildDeployInvocation(
                new AbilityDeployRequest(
                    "easynet:///r/example/agent/alice.sdk",
                    "easynet:///r/example/device/dev-a",
                    "easynet:///r/example/device/dev-a",
                    "1.0.0",
                    "AQIDBAUGBwgJCgsMDQ4PEA==",
                    Map.of("form", "none"),
                    new ResourceRef(
                        "easynet:///r/example/resource/device.dev-a/system/tmp/easynet-weather-package",
                        "easynet:///r/example/device/dev-a",
                        "system",
                        "tmp/easynet-weather-package",
                        "read",
                        4102444800000L,
                        "fs-local-mapping-v1"),
                    "local",
                    Map.of("request_id", "publication-deploy-1"))));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            publication.buildDeployInvocation(
                new AbilityDeployRequest(
                    "",
                    "easynet:///r/example/device/dev-a",
                    "easynet:///r/example/device/dev-a",
                    "1.0.0",
                    "AQIDBAUGBwgJCgsMDQ4PEA==",
                    Map.of("form", "none"),
                    resource,
                    "local",
                    Map.of("request_id", "publication-deploy-1"))));
    publication.close();
    expectSDKError(
        ErrorCode.INVALID_HANDLE,
        () -> publication.buildLocalResourceRef(new LocalResourceRefRequest("/tmp/easynet-weather-package", "read")));
  }

  private static void missionProfileDelegatesCarriersStatusAndStreams() throws Exception {
    var mission = new MissionClient(new FixtureMissionTransport());

    var run = mission.buildRunEALInvocation(missionRunRequest());
    check(
        run.inspectTuple().descriptor().equals("easynet:///r/example/ability/device.dev-a.mission.run@1.0.0"),
        "mission run descriptor");

    var runFile = mission.buildRunFileInvocation(missionRunFileRequest());
    check(
        runFile.inspectTuple().descriptor().equals("easynet:///r/example/ability/device.dev-a.mission.run@1.0.0"),
        "mission run-file descriptor");

    var track = mission.buildTrackInvocation(missionTrackRequest());
    check(
        track.inspectTuple().descriptor().equals("easynet:///r/example/ability/device.dev-a.mission.track@1.0.0"),
        "mission track descriptor");

    var cancel = mission.buildCancelInvocation(missionCancelRequest());
    check(
        cancel.inspectTuple().descriptor().equals("easynet:///r/example/ability/device.dev-a.mission.cancel@1.0.0"),
        "mission cancel descriptor");

    var events = mission.buildEventsInvocation(missionEventsRequest());
    check(
        events.inspectTuple().descriptor().equals("easynet:///r/example/ability/device.dev-a.mission.events@1.0.0"),
        "mission events descriptor");

    var status = mission.track(missionTrackRequest());
    check(status.terminal() && status.state().equals("partial"), "mission status terminal state");
    check(status.parentReceiptURA().equals("easynet:///r/example/receipt/parent"), "mission parent receipt");
    check(status.childInvocations().size() == 1, "mission child invocation facts");
    check(status.childReceipts().get(0).receiptURA().equals("easynet:///r/example/receipt/child"), "mission child receipt");
    check(status.outputRefs().size() == 4, "mission output refs");

    var page = mission.events(missionEventsRequest());
    check(page.events().size() == 2 && page.nextCursorSequence() == 7, "mission event page");
    check(page.events().get(1).terminal(), "mission terminal event");

    var stream = mission.openEventStream(missionEventsRequest());
    var event = stream.next();
    check(event.eventType().equals("progress") && event.sequence() == 7, "mission event stream payload");
    check(stream.cancel("done").terminal(), "mission event stream cancel terminal");
    stream.close();

    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> new MissionTrackRequest(missionCarrier(), "../weather"));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            MissionStatus.fromJSON(
                bytes(
                    new String(fixture("mission-status.v4.json"), StandardCharsets.UTF_8)
                        .replace("\"request_id\": \"req-1\"", "\"request_id\": null"))));

    mission.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, () -> mission.track(missionTrackRequest()));
  }

  private static MissionCarrierBase missionCarrier() {
    return missionCarrier("mission-run-1");
  }

  private static MissionCarrierBase missionCarrier(String requestID) {
    return new MissionCarrierBase(
        "easynet:///r/example/agent/alice.sdk",
        "easynet:///r/example/device/dev-a",
        "easynet:///r/example/device/dev-a",
        "1.0.0",
        "AQIDBAUGBwgJCgsMDQ4PEA==",
        Map.of("form", "none"),
        Map.of("request_id", requestID));
  }

  private static MissionRunRequest missionRunRequest() {
    return new MissionRunRequest(
        missionCarrier(), "mission weather\nlet r = local.observe_health()", "weather");
  }

  private static MissionRunFileRequest missionRunFileRequest() {
    return new MissionRunFileRequest(missionCarrier("mission-run-file-1"), "/tmp/easynet-sdk-demo.eal", "file-weather");
  }

  private static MissionTrackRequest missionTrackRequest() {
    return new MissionTrackRequest(missionCarrier("mission-track-1"), "2026-07-04_010203_weather");
  }

  private static MissionCancelRequest missionCancelRequest() {
    return new MissionCancelRequest(missionCarrier("mission-cancel-1"), "2026-07-04_010203_weather");
  }

  private static MissionEventsRequest missionEventsRequest() {
    return new MissionEventsRequest(missionCarrier("mission-events-1"), "2026-07-04_010203_weather", 4, 100);
  }

  private static void hostBindingProfileDelegatesCodecHashAndLifecycle() throws Exception {
    var provider = new FixtureHostLifecycleProvider();
    var hostBinding = new HostBindingClient(new FixtureHostBindingTransport(), provider);
    var binding = hostBinding.buildHostStreamBinding(hostStreamBindingRequest());
    check(binding.lifecycle().get("frame_contract_owner").equals("daemon_sdk"), "host binding lifecycle owner");
    check(binding.metadata().get("hash_algorithm").equals(HostBindingSupport.HASH_ALGORITHM), "host binding hash algorithm");

    var request =
        hostBinding.decodeRequest(
            new HostStreamEnvelope(
                new HostStreamEnvelope.HostStreamEnvelopeRequest(
                    "weather.stream",
                    Map.of("city", "Singapore"),
                    "call-weather-1",
                    "easynet:///r/example/user/alice")));
    check(request.function().equals("weather.stream"), "host binding request function");
    check(request.metadata().get("wire").equals("host_stream_request_v1"), "host binding request wire");

    var value = new LinkedHashMap<String, Object>();
    value.put("token", "hello");
    var item = hostBinding.encodeItem(0, value);
    check(item.frameType().equals("item") && item.seq() == 0, "host binding item frame");
    var errorFrame = hostBinding.encodeError(SDKError.validation("host", "bad input"));
    check(
        errorFrame.frameType().equals("error") && errorFrame.error().code() == ErrorCode.INVALID_ARGUMENT,
        "host binding error frame");
    var folded = hostBinding.foldOutputHash(HostStreamHashState.initial(), 0, value);
    check(
        folded.outputHash().equals("sha256:8196e03ca122ac3b47b3527c8f555735e53c0d3fe1eb8e30c0f974293cd5cd15"),
        "host binding folded hash");
    check(folded.equals(hostBinding.foldOutputHashLocal(HostStreamHashState.initial(), 0, value)), "host binding local hash");
    var terminal =
        hostBinding.encodeTerminal(HostStreamTerminalSummary.fromJSON(fixture("host-stream-terminal.v4.json")));
    check(
        terminal.frameType().equals("terminal") && terminal.outputHash().equals(folded.outputHash()),
        "host binding terminal frame");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            hostBinding.buildHostStreamBinding(
                new HostStreamBindingRequest(
                    "binding-weather-1",
                    "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                    "tmp/easynet-weather.sock",
                    HostBindingSupport.FRAME_SCHEMA,
                    Map.of("mode", "unlink_socket"),
                    30000L,
                    readinessDeclared(),
                    Map.of("owner", "easyremote"))));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            hostBinding.buildHostStreamBinding(
                new HostStreamBindingRequest(
                    "binding-weather-1",
                    "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
                    "/tmp/easynet-weather.sock",
                    "drift.schema.json",
                    Map.of("mode", "unlink_socket"),
                    30000L,
                    readinessDeclared(),
                    Map.of("owner", "easyremote"))));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> hostBinding.foldOutputHash(HostStreamHashState.fromJSON(fixture("host-stream-hash-state.v4.json")), 2, value));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> HostStreamHashState.fromJSON(fixture("host-stream-hash-state-corrupted-zero.v4.json")));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> HostStreamHashState.fromJSON(fixture("host-stream-hash-state-corrupted-gap.v4.json")));

    var lifecycle = hostBinding.openLifecycle(binding, null);
    var checked = lifecycle.checkReadiness();
    var cleanup = lifecycle.cleanup();
    var cleanupAgain = lifecycle.cleanup();
    check(checked.state().equals("ready") && Boolean.TRUE.equals(checked.endpointReady()), "host binding ready");
    check(cleanup.mode().equals("unlink_socket") && cleanup.metadata().get("cleaned").equals(true), "host binding cleanup");
    check(cleanupAgain.metadata().get("cleaned").equals(true), "host binding cleanup idempotent");
    check(provider.cleanupCalls == 1, "host binding cleanup once");
    lifecycle.close();
    check(lifecycle.state() == HostStreamLifecycleState.CLOSED, "host binding lifecycle closed");

    hostBinding.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, () -> hostBinding.encodeItem(0, value));
  }

  private static void eventsProfileDelegatesCarriersProjectionsHistoryAndStreams() throws Exception {
    var events = new EventClient(new FixtureEventTransport());
    var base = eventsCarrierBase(Map.of("request_id", "events-directory-subscribe-1"));
    var directoryRequest =
        new EventsSubscriptionRequest(
            base,
            null,
            null,
            "example",
            null,
            null,
            "easynet:///r/example/agent/alice.main",
            null,
            null,
            null,
            new EventCursor("directory", 7, ""),
            30000);
    var deviceRequest =
        new EventsSubscriptionRequest(
            eventsCarrierBase(Map.of("request_id", "events-device-subscribe-1")),
            "device",
            new EventFilter(null, null, "easynet:///r/example/device/dev-a", null, null, null),
            null,
            null,
            "easynet:///r/example/device/dev-a",
            null,
            null,
            null,
            null,
            new EventCursor("device", 2, ""),
            30000);
    var sessionRequest =
        new EventsSubscriptionRequest(
            eventsCarrierBase(Map.of("request_id", "events-session-subscribe-1")),
            "session",
            null,
            null,
            null,
            null,
            null,
            "run-1",
            null,
            null,
            new EventCursor("session", 4, ""),
            null);
    var invocationRequest =
        new EventsSubscriptionRequest(
            eventsCarrierBase(Map.of("request_id", "events-invocation-subscribe-1")),
            "invocation",
            new EventFilter(null, null, null, null, null, "inv-1"),
            null,
            null,
            null,
            null,
            null,
            null,
            "inv-1",
            new EventCursor("invocation", 9, ""),
            null);

    check(
        events.buildDirectorySubscriptionInvocation(directoryRequest)
            .get("descriptor_ref")
            .toString()
            .contains("federation.subscribe_directory_v2"),
        "events directory carrier");
    check(
        events.buildDeviceSubscriptionInvocation(deviceRequest)
            .get("descriptor_ref")
            .toString()
            .contains("events.device.subscribe"),
        "events device carrier");
    check(
        events.buildSessionSubscriptionInvocation(sessionRequest)
            .get("descriptor_ref")
            .toString()
            .contains("session.attach"),
        "events session carrier");
    check(
        events.buildInvocationSubscriptionInvocation(invocationRequest)
            .get("descriptor_ref")
            .toString()
            .contains("events.invocation.subscribe"),
        "events invocation carrier");

    var page =
        events.listDeviceEvents(
            new EventsDeviceEventListRequest(
                eventsCarrierBase(Map.of("request_id", "events-device-history-1")),
                new EventFilter(null, null, "easynet:///r/example/device/dev-a", null, null, null),
                "easynet:///r/example/device/dev-a",
                null,
                null));
    check(page.limit() == 50 && page.items().get(0).stream().equals("device"), "events device page");

    var directoryFrame =
        events.projectDirectoryEvent(
            new EventProjectionInput(
                new EventCursor("directory", 8, ""),
                Map.of("type", "agent_advertised"),
                null,
                null,
                null));
    check(directoryFrame.cursor().resumeToken().equals("directory:8"), "events directory projection");
    check(
        events.projectLiveEvent(
                new EventProjectionInput(
                    new EventCursor("device", 8, ""), Map.of("state", "online"), null, null, null))
            .stream()
            .equals("device"),
        "events live projection");
    check(
        events.projectDropReport(
                new EventDropReportInput(
                    new EventCursor("directory", 10, ""),
                    1783100000123L,
                    4,
                    1000,
                    "consumer_lagged",
                    null,
                    null,
                    null))
            .droppedCount()
            == 4,
        "events drop projection");
    check(
        events.projectTerminal(
                new EventTerminalInput(
                    new EventCursor("directory", 11, ""),
                    1783100000123L,
                    null,
                    "client_closed",
                    null,
                    null,
                    null))
            .terminal(),
        "events terminal projection");

    var stream = events.subscribeDirectory(directoryRequest);
    check(stream.receive().kind().equals("directory.agent_advertised"), "events stream frame");
    check(stream.receive().terminal(), "events stream terminal frame");
    check(stream.state().equals("Terminal"), "events stream terminal state");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            events.buildSessionSubscriptionInvocation(
                new EventsSubscriptionRequest(
                    base,
                    "session",
                    null,
                    null,
                    null,
                    null,
                    null,
                    null,
                    "easynet:///r/example/resource/session.run-1",
                    null,
                    null,
                    null)));
  }

  private static EventsCarrierBase eventsCarrierBase(Map<String, Object> metadata) {
    return new EventsCarrierBase(
        "easynet:///r/example/agent/alice.sdk",
        "easynet:///r/example/device/dev-a",
        "easynet:///r/example/device/dev-a",
        "1.0.0",
        "AQIDBAUGBwgJCgsMDQ4PEA==",
        Map.of("form", "none"),
        metadata);
  }

  private static void surfaceProfileDelegatesCarriersAndProjections() throws Exception {
    var transport = new FixtureSurfaceTransport();
    var surface = new SurfaceClient(transport);

    var list =
        new SurfaceListPagesRequest(
            surfaceCarrierBase(Map.of("request_id", "surface-list-1")), 50, "");
    var create =
        new SurfaceCreatePageRequest(
            surfaceCarrierBase(Map.of("request_id", "surface-create-1")),
            "docs",
            "/tmp/easynet-pages-docs",
            "public");
    var delete =
        new SurfaceDeletePageRequest(
            surfaceCarrierBase(Map.of("request_id", "surface-delete-1")), "docs");
    var manifest =
        new SurfaceManifestRequest(
            surfaceCarrierBase(Map.of("request_id", "surface-manifest-1")), "docs");
    var health =
        new SurfaceHealthRequest(
            surfaceCarrierBase(Map.of("request_id", "surface-health-1")),
            null,
            "easynet:///r/example/resource/alice.docs");

    check(
        surface.buildListPagesInvocation(list)
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/alice.pages.pages.list@1.0.0"),
        "surface list descriptor");
    check(
        surface.buildCreatePageInvocation(create)
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/alice.pages.pages.publish@1.0.0"),
        "surface create descriptor");
    check(
        surface.buildDeletePageInvocation(delete)
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/alice.pages.pages.unpublish@1.0.0"),
        "surface delete descriptor");
    check(
        surface.buildManifestInvocation(manifest)
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/alice.pages.pages.get@1.0.0"),
        "surface manifest descriptor");
    check(
        surface.buildHealthInvocation(health)
            .get("descriptor_ref")
            .equals("easynet:///r/example/ability/alice.pages.pages.health@1.0.0"),
        "surface health descriptor");

    var pagePage = surface.listPages(list);
    check(pagePage.limit() == 50 && pagePage.items().size() == 1, "surface page page");
    check(
        surface.projectPagePage(fixture("surface-page-page.v4.json")).source().equals("pages_read_model"),
        "surface project page");
    var record = surface.createPage(create);
    check(record.pageID().equals("docs"), "surface create page record");
    check(surface.deletePage(delete).removed(), "surface delete mutation");
    check(
        surface.surfaceManifest(manifest).entrypoint().get("kind").equals("public_page_ref"),
        "surface manifest entrypoint");
    check(surface.publicPageRef(record).routeKind().equals("hub_web"), "surface public ref");
    check(surface.surfaceHealth(health).ready(), "surface health");
    check(surface.surfaceStatus(health).descriptorRef().contains("pages.health"), "surface status alias");
    check(surface.projectManifest(fixture("surface-manifest.v4.json")).pageID().equals("docs"), "surface project manifest");
    check(surface.projectHealth(fixture("surface-health.v4.json")).pageCount() == 1, "surface project health");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> new SurfaceListPagesRequest(surfaceCarrierBase(Map.of()), 501, ""));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> new SurfaceCreatePageRequest(surfaceCarrierBase(Map.of()), "docs", "tmp/easynet-pages-docs", "public"));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> new SurfaceHealthRequest(surfaceCarrierBase(Map.of()), null, "https://example/web/alice/docs/"));
    surface.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, () -> surface.listPages(list));
  }

  private static SurfaceCarrierBase surfaceCarrierBase(Map<String, Object> metadata) {
    return new SurfaceCarrierBase(
        "easynet:///r/example/agent/alice.sdk",
        "easynet:///r/example/agent/alice.pages",
        "easynet:///r/example/agent/alice.pages",
        "1.0.0",
        "AQIDBAUGBwgJCgsMDQ4PEA==",
        Map.of("form", "none"),
        metadata);
  }

  private static HostStreamBindingRequest hostStreamBindingRequest() {
    return new HostStreamBindingRequest(
        "binding-weather-1",
        "easynet:///r/example/ability/device.dev-a.weather.stream@1.0.0",
        "/tmp/easynet-weather.sock",
        HostBindingSupport.FRAME_SCHEMA,
        Map.of("mode", "unlink_socket"),
        30000L,
        readinessDeclared(),
        Map.of("owner", "easyremote"));
  }

  private static Map<String, Object> readinessDeclared() {
    LinkedHashMap<String, Object> readiness = new LinkedHashMap<>();
    readiness.put("state", "declared");
    readiness.put("checked", false);
    readiness.put("endpoint_ready", null);
    return readiness;
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

  private static class MemoryRuntimeTransport implements RuntimeTransport {
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

  private static final class EventsDirectoryStreamSource implements StreamSource {
    private final ArrayDeque<StreamEvent> events = new ArrayDeque<>();

    EventsDirectoryStreamSource() {
      events.add(StreamEvent.data(8, new String(fixture("event.directory.v4.json"), StandardCharsets.UTF_8)));
      events.add(StreamEvent.data(11, new String(fixture("event.directory-terminal.v4.json"), StandardCharsets.UTF_8)));
    }

    @Override
    public StreamEvent next() {
      return events.removeFirst();
    }
  }

  private static final class MissionEventStreamSource implements StreamSource {
    private final ArrayDeque<StreamEvent> events = new ArrayDeque<>();

    MissionEventStreamSource() {
      events.add(
          StreamEvent.data(
              7,
              """
              {"profile":"mission","kind":"mission_event","mission_id":"2026-07-04_010203_weather","sequence":7,"event_type":"progress","occurred_unix_ms":1783126928000,"terminal":false,"payload":{"delta":"stream"},"receipt":{},"metadata":{"profile":"mission","carrier_owner":"daemon_sdk"}}
              """));
    }

    @Override
    public StreamEvent next() {
      return events.isEmpty() ? StreamEvent.terminal(8, "Completed") : events.removeFirst();
    }

    @Override
    public StreamEvent cancel(String reason) {
      return StreamEvent.terminal(8, "Cancelled");
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

  private static final class FixturePublicationTransport implements PublicationTransport {
    private void expectRequest(byte[] requestJSON, String fixtureName, String label) {
      var request = JsonValueReader.object(requestJSON, label);
      var expected = JsonValueReader.object(fixture(fixtureName), fixtureName);
      check(request.equals(expected), label);
    }

    @Override
    public byte[] buildResourceRef(byte[] requestJSON) {
      expectRequest(requestJSON, "local-resource-ref-request.v4.json", "publication resource-ref request");
      return fixture("resource-ref.local-fs.v4.json");
    }

    @Override
    public byte[] validatePackage(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "publication validate request");
      var expectedManifest = JsonValueReader.object(fixture("ability-package-manifest.v4.json"), "publication manifest");
      check(request.equals(Map.of("manifest", expectedManifest)), "publication validate request");
      return fixture("package-validation.v4.json");
    }

    @Override
    public byte[] buildDeployInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "ability-deploy-request.v4.json", "publication deploy request");
      return fixture("publication-deploy-invocation.v4.json");
    }

    @Override
    public byte[] buildUnpublishInvocation(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "publication unpublish request");
      check(request.get("ability_ura").equals("easynet:///r/example/ability/device.dev-a.er.weather"), "publication unpublish ability");
      return fixture("publication-unpublish-invocation.v4.json");
    }
  }

  private static final class FixtureMissionTransport implements MissionTransport {
    private void expectRequest(byte[] requestJSON, String fixtureName, String label) {
      var request = JsonValueReader.object(requestJSON, label);
      var expected = JsonValueReader.object(fixture(fixtureName), fixtureName);
      check(request.equals(expected), label);
    }

    @Override
    public byte[] buildRunEALInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-run-request.v4.json", "mission run request");
      return fixture("mission-run-invocation.v4.json");
    }

    @Override
    public byte[] buildRunFileInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-run-file-request.v4.json", "mission run-file request");
      return fixture("mission-run-invocation.v4.json");
    }

    @Override
    public byte[] buildTrackInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-track-request.v4.json", "mission track request");
      return fixture("mission-track-invocation.v4.json");
    }

    @Override
    public byte[] buildCancelInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-cancel-request.v4.json", "mission cancel request");
      return fixture("mission-cancel-invocation.v4.json");
    }

    @Override
    public byte[] buildEventsInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-events-request.v4.json", "mission events request");
      return missionEventsInvocation();
    }

    @Override
    public byte[] runEAL(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-run-request.v4.json", "mission run request");
      return fixture("mission-status.v4.json");
    }

    @Override
    public byte[] runFile(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-run-file-request.v4.json", "mission run-file request");
      return fixture("mission-status.v4.json");
    }

    @Override
    public byte[] track(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-track-request.v4.json", "mission track request");
      return fixture("mission-status.v4.json");
    }

    @Override
    public byte[] cancel(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-cancel-request.v4.json", "mission cancel request");
      return fixture("mission-status.v4.json");
    }

    @Override
    public byte[] events(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-events-request.v4.json", "mission events request");
      return fixture("mission-event-page.v4.json");
    }

    @Override
    public StreamHandle openEventStream(byte[] requestJSON) {
      expectRequest(requestJSON, "mission-events-request.v4.json", "mission event stream request");
      return new StreamHandle(new MissionEventStreamSource());
    }

    private byte[] missionEventsInvocation() {
      return bytes(
          """
          {
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "args": {
              "run_id": "2026-07-04_010203_weather",
              "cursor_sequence": 4,
              "limit": 100
            },
            "metadata": {"request_id": "mission-events-1", "profile": "mission", "system_ability": "mission.events", "carrier_owner": "daemon_sdk"}
          }
          """);
    }
  }

  private static final class FixtureHostBindingTransport implements HostBindingTransport {
    private void expectRequest(byte[] requestJSON, String fixtureName, String label) {
      var request = JsonValueReader.object(requestJSON, label);
      var expected = JsonValueReader.object(fixture(fixtureName), fixtureName);
      check(request.equals(expected), label);
    }

    @Override
    public byte[] buildHostStreamBinding(byte[] requestJSON) {
      expectRequest(requestJSON, "host-stream-binding-request.v4.json", "host binding request");
      return fixture("host-stream-binding.v4.json");
    }

    @Override
    public byte[] decodeRequest(byte[] envelopeJSON) {
      var envelope = JsonValueReader.object(envelopeJSON, "host stream envelope");
      @SuppressWarnings("unchecked")
      var request = (Map<String, Object>) envelope.get("request");
      check(request.get("fn").equals("weather.stream"), "host binding envelope function");
      return fixture("host-stream-request.v4.json");
    }

    @Override
    public byte[] encodeItem(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "host stream item request");
      check(request.get("seq").equals(0L), "host binding item seq");
      return fixture("host-stream-frame.v4.json");
    }

    @Override
    public byte[] encodeError(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "host stream error request");
      @SuppressWarnings("unchecked")
      var error = (Map<String, Object>) request.get("error");
      check(error.get("code").equals("INVALID_ARGUMENT"), "host binding error code");
      return bytes(
          """
          {
            "frame_type": "error",
            "seq": null,
            "value": null,
            "error": {
              "code": "INVALID_ARGUMENT",
              "stage": "host",
              "message": "bad input",
              "retry": "never",
              "details": {}
            },
            "terminal": null,
            "output_hash": null
          }
          """);
    }

    @Override
    public byte[] encodeTerminal(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "host stream terminal request");
      @SuppressWarnings("unchecked")
      var summary = (Map<String, Object>) request.get("summary");
      check(
          summary.equals(JsonValueReader.object(fixture("host-stream-terminal.v4.json"), "host terminal fixture")),
          "host binding terminal summary");
      LinkedHashMap<String, Object> frame = new LinkedHashMap<>();
      frame.put("frame_type", "terminal");
      frame.put("seq", summary.get("frames"));
      frame.put("value", null);
      frame.put("error", null);
      frame.put("terminal", summary);
      frame.put("output_hash", summary.get("output_hash"));
      return JsonValueWriter.object(frame);
    }

    @Override
    public byte[] foldOutputHash(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "host stream hash request");
      check(request.get("seq").equals(0L), "host binding hash seq");
      return fixture("host-stream-hash-state.v4.json");
    }
  }

  private static final class FixtureHostLifecycleProvider implements HostStreamLifecycleProvider {
    int cleanupCalls;

    @Override
    public HostStreamReadiness checkReadiness(HostStreamBinding binding) {
      return new HostStreamReadiness(
          "ready", true, true, Map.of("endpoint", binding.endpoint()));
    }

    @Override
    public HostStreamCleanup cleanup(HostStreamBinding binding) {
      cleanupCalls++;
      return new HostStreamCleanup("unlink_socket", Map.of("cleaned", true));
    }
  }

  private static final class FixtureEventTransport implements EventTransport {
    @Override
    public byte[] buildDirectorySubscriptionInvocation(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "events directory request");
      check(request.get("stream").equals("directory"), "events directory stream");
      return fixture("events-directory-subscription-invocation.v4.json");
    }

    @Override
    public byte[] buildDeviceSubscriptionInvocation(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "events device request");
      check(request.get("stream").equals("device"), "events device stream");
      return fixture("events-device-subscription-invocation.v4.json");
    }

    @Override
    public byte[] buildSessionSubscriptionInvocation(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "events session request");
      check(request.get("session_id").equals("run-1"), "events session id");
      return fixture("events-session-subscription-invocation.v4.json");
    }

    @Override
    public byte[] buildInvocationSubscriptionInvocation(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "events invocation request");
      check(request.get("invocation_id").equals("inv-1"), "events invocation id");
      return fixture("events-invocation-subscription-invocation.v4.json");
    }

    @Override
    public StreamSource subscribeDirectory(byte[] requestJSON) {
      return new EventsDirectoryStreamSource();
    }

    @Override
    public byte[] listDeviceEvents(byte[] requestJSON) {
      var request = JsonValueReader.object(requestJSON, "events device history request");
      check(request.get("limit").equals(50L), "events default history limit");
      return fixture("event.device-page.v4.json");
    }

    @Override
    public byte[] projectDirectoryEvent(byte[] eventJSON) {
      return fixture("event.directory.v4.json");
    }

    @Override
    public byte[] projectLiveEvent(byte[] eventJSON) {
      var request = JsonValueReader.object(eventJSON, "events live projection request");
      var cursor = EventsSupport.requiredObject(request, "cursor");
      return cursor.get("stream").equals("invocation")
          ? fixture("event.invocation-live.v4.json")
          : fixture("event.device-live.v4.json");
    }

    @Override
    public byte[] projectDropReport(byte[] dropJSON) {
      return fixture("event.directory-drop-report.v4.json");
    }

    @Override
    public byte[] projectTerminal(byte[] terminalJSON) {
      return fixture("event.directory-terminal.v4.json");
    }
  }

  private static final class FixtureSurfaceTransport implements SurfaceTransport {
    private void expectRequest(byte[] requestJSON, String fixtureName, String label) {
      var request = JsonValueReader.object(requestJSON, label);
      var expected = JsonValueReader.object(fixture(fixtureName), fixtureName);
      check(request.equals(expected), label);
    }

    @Override
    public byte[] buildListPagesInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-list-pages-request.v4.json", "surface list request");
      return fixture("surface-list-pages-invocation.v4.json");
    }

    @Override
    public byte[] buildCreatePageInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-create-page-request.v4.json", "surface create request");
      return fixture("surface-create-page-invocation.v4.json");
    }

    @Override
    public byte[] buildDeletePageInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-delete-page-request.v4.json", "surface delete request");
      return fixture("surface-delete-page-invocation.v4.json");
    }

    @Override
    public byte[] buildManifestInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-manifest-request.v4.json", "surface manifest request");
      return fixture("surface-manifest-invocation.v4.json");
    }

    @Override
    public byte[] buildHealthInvocation(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-health-request.v4.json", "surface health request");
      return fixture("surface-health-invocation.v4.json");
    }

    @Override
    public byte[] listPages(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-list-pages-request.v4.json", "surface list pages request");
      return fixture("surface-page-page.v4.json");
    }

    @Override
    public byte[] createPage(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-create-page-request.v4.json", "surface create page request");
      return fixture("surface-page-record.v4.json");
    }

    @Override
    public byte[] deletePage(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-delete-page-request.v4.json", "surface delete page request");
      return fixture("surface-mutation-result.v4.json");
    }

    @Override
    public byte[] surfaceManifest(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-manifest-request.v4.json", "surface manifest request");
      return fixture("surface-manifest.v4.json");
    }

    @Override
    public byte[] publicPageRef(byte[] pageJSON) {
      var page = JsonValueReader.object(pageJSON, "surface page record JSON");
      check(page.get("page_id").equals("docs"), "surface public page input");
      return fixture("surface-public-page-ref.v4.json");
    }

    @Override
    public byte[] surfaceHealth(byte[] requestJSON) {
      expectRequest(requestJSON, "surface-health-request.v4.json", "surface health request");
      return fixture("surface-health.v4.json");
    }
  }

}
