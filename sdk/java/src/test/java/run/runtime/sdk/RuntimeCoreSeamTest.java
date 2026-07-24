package run.runtime.sdk;

import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.Base64;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class RuntimeCoreSeamTest {
  private static final String CALLER = "easynet:///r/example/agent/alice.sdk";
  private static final String CALLEE = "easynet:///r/example/device/dev-a";
  private static final String DESCRIPTOR =
      "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0";
  private static final String NONCE = "AQIDBAUGBwgJCgsMDQ4PEA==";
  private static final List<String> TEST_SELECTORS =
      List.of(
          "productNeutralJarExportsOnlyGenericRuntimeConcepts",
          "discoveryAndLifecycleAreExplicit",
          "healthKeepsLivenessSeparateFromReadiness",
          "invocationPrepareSignSubmitPreservesTheCompleteTuple",
          "invocationResultUsesTerminalReceipt",
          "runtimeReceiptProofFactsAreMandatory",
          "authorityMetadataIsTypedAndMutuallyExclusive",
          "invocationAuthorityMetadataIsTupleBound",
          "authorityMetadataRejectsAllZeroSessionOwners",
          "authorityMetadataBindsSessionAuthoritySubjects",
          "streamAndBidiLifecyclesAreBounded",
          "bidiFrame0IsRequiredBeforeRuntimeSessionEntry",
          "asyncRuntimeDelegatesToTheSameRuntimeStateMachine",
          "typedErrorsPreserveStableCategories",
          "abiCompatibleAcceptsExactVersion",
          "abiIncompatibleRejectsMismatch",
          "retryHintsPreserveRetryability",
          "canonicalSigningMaterialComesFromPrepare",
          "preparedInvocationRequiresExplicitDescriptorRef",
          "completeTupleRejectsMissingCaller",
          "completeTupleRejectsAllZeroPrincipals",
          "preparedInvocationCannotBeSubmitted",
          "streamAndBidiBackpressureAreBounded",
          "streamOrderAndTerminalArePreserved");

  public static void main(String[] args) throws Exception {
    if (args.length == 0) {
      for (String selector : TEST_SELECTORS) {
        runSelector(selector);
      }
      return;
    }
    if (args.length != 1) {
      throw new IllegalArgumentException("expected one test selector or --list");
    }
    if ("--list".equals(args[0])) {
      TEST_SELECTORS.forEach(System.out::println);
      return;
    }
    runSelector(args[0]);
  }

  public void testRuntimeCoreSeam() throws Exception {
    main(new String[0]);
  }

  private static void runSelector(String selector) throws Exception {
    switch (selector) {
      case "productNeutralJarExportsOnlyGenericRuntimeConcepts" ->
          productNeutralJarExportsOnlyGenericRuntimeConcepts();
      case "discoveryAndLifecycleAreExplicit" -> discoveryAndLifecycleAreExplicit();
      case "healthKeepsLivenessSeparateFromReadiness" ->
          healthKeepsLivenessSeparateFromReadiness();
      case "invocationPrepareSignSubmitPreservesTheCompleteTuple" ->
          invocationPrepareSignSubmitPreservesTheCompleteTuple();
      case "invocationResultUsesTerminalReceipt" -> invocationResultUsesTerminalReceipt();
      case "runtimeReceiptProofFactsAreMandatory" -> runtimeReceiptProofFactsAreMandatory();
      case "authorityMetadataIsTypedAndMutuallyExclusive" ->
          authorityMetadataIsTypedAndMutuallyExclusive();
      case "invocationAuthorityMetadataIsTupleBound" ->
          invocationAuthorityMetadataIsTupleBound();
      case "authorityMetadataRejectsAllZeroSessionOwners" ->
          authorityMetadataRejectsAllZeroSessionOwners();
      case "authorityMetadataBindsSessionAuthoritySubjects" ->
          authorityMetadataBindsSessionAuthoritySubjects();
      case "streamAndBidiLifecyclesAreBounded" -> streamAndBidiLifecyclesAreBounded();
      case "bidiFrame0IsRequiredBeforeRuntimeSessionEntry" ->
          bidiFrame0IsRequiredBeforeRuntimeSessionEntry();
      case "asyncRuntimeDelegatesToTheSameRuntimeStateMachine" ->
          asyncRuntimeDelegatesToTheSameRuntimeStateMachine();
      case "typedErrorsPreserveStableCategories" -> typedErrorsPreserveStableCategories();
      case "abiCompatibleAcceptsExactVersion" -> abiCompatibleAcceptsExactVersion();
      case "abiIncompatibleRejectsMismatch" -> abiIncompatibleRejectsMismatch();
      case "retryHintsPreserveRetryability" -> retryHintsPreserveRetryability();
      case "canonicalSigningMaterialComesFromPrepare" -> canonicalSigningMaterialComesFromPrepare();
      case "preparedInvocationRequiresExplicitDescriptorRef" ->
          preparedInvocationRequiresExplicitDescriptorRef();
      case "completeTupleRejectsMissingCaller" -> completeTupleRejectsMissingCaller();
      case "completeTupleRejectsAllZeroPrincipals" -> completeTupleRejectsAllZeroPrincipals();
      case "preparedInvocationCannotBeSubmitted" -> preparedInvocationCannotBeSubmitted();
      case "streamAndBidiBackpressureAreBounded" -> streamAndBidiBackpressureAreBounded();
      case "streamOrderAndTerminalArePreserved" -> streamOrderAndTerminalArePreserved();
      default -> throw new IllegalArgumentException("unknown test selector: " + selector);
    }
  }

  private static void productNeutralJarExportsOnlyGenericRuntimeConcepts() {
    List<String> removedProducts =
        List.of(
            "AdminClient",
            "CompanionClient",
            "CompatibilityClient",
            "DirectoryClient",
            "IdentityClient",
            "EventClient",
            "HostBindingClient",
            "MissionClient",
            "PublicationClient",
            "ReceiptClient",
            "SurfaceClient",
            "WrapperClient");
    for (String className : removedProducts) {
      try {
        Class.forName("run.runtime.sdk." + className);
        throw new AssertionError("product-neutral SDK must not export " + className);
      } catch (ClassNotFoundException expected) {
        // Absence is the public boundary under test.
      }
    }
  }

  private static void discoveryAndLifecycleAreExplicit() {
    final boolean[] closed = {false};
    Client client =
        new Client(
            new DiscoveryTransport() {
              @Override
              public FeatureSet featureDiscovery() {
                return new FeatureSet(
                    5,
                    "0.0.0-seam",
                    Map.of("runtime_core", "seam", "health", "seam", "authority", "seam"),
                    Map.of("runtime_prepare", true, "runtime_submit_signed", true));
              }

              @Override
              public void close() {
                closed[0] = true;
              }
            });

    check(client.requireABI(5).symbols().get("runtime_prepare"), "runtime prepare discovery");
    expectSDKError(ErrorCode.VERSION_INCOMPATIBLE, () -> client.requireABI(4));
    client.close();
    client.close();
    check(closed[0], "discovery transport closed");
    expectSDKError(ErrorCode.INVALID_HANDLE, client::featureDiscovery);
  }

  private static void healthKeepsLivenessSeparateFromReadiness() {
    HealthClient health = new HealthClient(new MemoryHealthTransport());
    RuntimeHealth state = health.runtimeHealth();
    check(state.apiAlive(), "health liveness");
    check(!state.ready(), "health readiness remains separate");
    check(state.abiVersion() == 5, "health ABI version");
    DiagnosticsReport diagnostics = health.diagnostics();
    check(diagnostics.checks().size() == 1, "diagnostics checks");
    health.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, health::runtimeHealth);
  }

  private static void invocationPrepareSignSubmitPreservesTheCompleteTuple() {
    MemoryRuntimeTransport transport = new MemoryRuntimeTransport();
    RuntimeClient runtime = new RuntimeClient(transport);
    InvocationDraft draft = completeDraft(runtime);

    InvocationResult result = runtime.invoke(draft);
    check(result.ok(), "generic invocation result");
    check(
        "inv-direct".equals(result.terminalReceipt().get("invocation_id")),
        "canonical terminal receipt facts are preserved");

    PreparedInvocation prepared = runtime.prepare(draft, Map.of("deadline_ms", 1000));
    check(!prepared.submitReady(), "prepared invocation is not submit-ready");
    check(prepared.tuple().descriptor().equals(DESCRIPTOR), "prepared tuple descriptor");
    check(prepared.tuple().caller().equals(CALLER), "prepared tuple caller");

    InvocationSignature signature =
        new InvocationSignature("ed25519", "c2lnbmF0dXJl", "caller-key-1", "");
    SignedInvocation signed = prepared.signWithCallerSignature(signature);
    check(signed.submitReady(), "signed invocation is submit-ready");
    InvocationHandle handle = signed.submit();
    check(!handle.terminal(), "submitted invocation handle");
    InvocationResult awaited = runtime.awaitResult(handle);
    check(awaited.ok(), "awaited invocation result");
    InvocationCancel cancelled = runtime.cancel(handle, "done");
    check(cancelled.requestAccepted() && cancelled.terminal(), "cancelled invocation handle");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            InvocationCancel.fromJSON(
                "{\"handle_id\":7,\"cancelled\":true,\"state\":\"Cancelled\",\"terminal\":true}"
                    .getBytes(StandardCharsets.UTF_8)));
    InvocationHandle events = runtime.events(handle);
    check(events.terminal(), "invocation events snapshot");
    runtime.closeHandle(handle);
    check(transport.submittedSigner.equals("caller-key-1"), "caller signer preserved");
    InvocationHandle forged =
        InvocationHandle.fromJSON(
            "{\"handle_id\":7,\"state\":\"Running\",\"terminal\":false}"
                .getBytes(StandardCharsets.UTF_8));
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.awaitResult(forged));
    transport.eventHandleId = 8;
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.events(handle));

    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.submitSigned(prepared));
    runtime.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, runtime::newInvocation);
  }

  private static void invocationResultUsesTerminalReceipt() {
    Map<String, Object> terminal = canonicalRuntimeReceiptFixture("inv-result", "completed", "Completed", 1);
    InvocationResult canonical =
        InvocationResult.fromJSON(
            JsonValueWriter.object(
                Map.of(
                    "ok",
                    true,
                    "terminal_state",
                    "Completed",
                    "terminal_receipt",
                    terminal)));
    check(
        "inv-result".equals(canonical.terminalReceipt().get("invocation_id")),
        "terminal_receipt populates terminalReceipt");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "terminal_receipt is required",
        () ->
            InvocationResult.fromJSON(
                JsonValueWriter.object(
                    Map.of(
                        "ok",
                        true,
                        "terminal_state",
                        "Completed"))));

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "terminal_receipt is required",
        () ->
            InvocationResult.fromJSON(
                JsonValueWriter.object(
                    nullableMapOf(
                        "ok",
                        true,
                        "terminal_state",
                        "Completed",
                        "terminal_receipt",
                        null))));

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "retired receipt alias is not accepted",
        () ->
            InvocationResult.fromJSON(
                JsonValueWriter.object(
                    Map.of(
                        "ok",
                        true,
                        "terminal_state",
                        "Completed",
                        "receipt",
                        terminal))));
  }

  private static void runtimeReceiptProofFactsAreMandatory() {
    Map<String, Object> complete =
        canonicalRuntimeReceiptFixture("inv-proof", "completed", "Completed", 1);
    RuntimeReceipt receipt = RuntimeReceipt.fromMap(complete);
    check("COMPLETED".equals(receipt.lifecycleState()), "canonical receipt lifecycle state");

    Map<String, Object> mismatchedType = new LinkedHashMap<>(complete);
    mismatchedType.put("receipt_type", "terminal");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "receipt_type",
        () -> RuntimeReceipt.fromMap(mismatchedType));

    Map<String, Object> missingProof = new LinkedHashMap<>(complete);
    missingProof.remove("authority_proof");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "authority_proof",
        () -> RuntimeReceipt.fromMap(missingProof));

    Map<String, Object> mismatchedProofHash = new LinkedHashMap<>(complete);
    Map<String, Object> mismatchedProof = mutableAuthorityProof(mismatchedProofHash);
    mismatchedProof.put("proof_hash_hex", "ff".repeat(32));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "authority_proof_hash_mismatch",
        () -> RuntimeReceipt.fromMap(mismatchedProofHash));

    Map<String, Object> bindingHashProof = new LinkedHashMap<>(complete);
    Map<String, Object> proof = mutableAuthorityProof(bindingHashProof);
    proof.put("proof_payload_base64", "");
    proof.put("proof_hash_hex", authorityBindingProofHashSelf(CALLEE));
    proof.remove("signature");
    RuntimeReceipt bindingHashReceipt = RuntimeReceipt.fromMap(bindingHashProof);
    check(
        "COMPLETED".equals(bindingHashReceipt.lifecycleState()),
        "binding-hash proof without payload/signature is accepted");

    Map<String, Object> wrongIssuer = new LinkedHashMap<>(complete);
    Map<String, Object> wrongIssuerProof = mutableAuthorityProof(wrongIssuer);
    wrongIssuerProof.put(
        "issuer",
        Map.of("ura", "easynet:///r/example/device/other", "profile", "axon-strict-v2"));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "authority_proof issuer does not match callee_binding",
        () -> RuntimeReceipt.fromMap(wrongIssuer));

    for (String retiredProfile : List.of("axon-legacy-v1", "opaque")) {
      Map<String, Object> retiredCalleeProfile = new LinkedHashMap<>(complete);
      retiredCalleeProfile.put("callee_binding", Map.of("ura", CALLEE, "profile", retiredProfile));
      expectSDKError(
          ErrorCode.INVALID_ARGUMENT,
          "callee_binding.profile is not canonical",
          () -> RuntimeReceipt.fromMap(retiredCalleeProfile));
    }

    Map<String, Object> hostedSignerWithoutAttestation = new LinkedHashMap<>(complete);
    hostedSignerWithoutAttestation.put(
        "signer_binding",
        Map.of("ura", "easynet:///r/example/device/runtime-host", "profile", "axon-strict-v2"));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "hosted runtime receipt is missing host_attestation_base64",
        () -> RuntimeReceipt.fromMap(hostedSignerWithoutAttestation));

    Map<String, Object> selfSignerWithAttestation = new LinkedHashMap<>(complete);
    selfSignerWithAttestation.put(
        "host_attestation_base64", Base64.getEncoder().encodeToString(repeatedByte(0x73, 64)));
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "self-signed runtime receipt must not carry host_attestation_base64",
        () -> RuntimeReceipt.fromMap(selfSignerWithAttestation));

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "authority_proof",
        () ->
            InvocationResult.fromJSON(
                JsonValueWriter.object(
                    Map.of(
                        "ok",
                        true,
                        "terminal_state",
                        "Completed",
                        "terminal_receipt",
                        missingProof))));
  }

  private static void authorityMetadataIsTypedAndMutuallyExclusive() {
    String delegationValue = delegationMetadataValue();
    String sessionValue = sessionMetadataValue();
    AuthorityClient authority =
        new AuthorityClient(new MemoryAuthorityTransport(delegationValue, sessionValue));

    DelegationProof delegation =
        authority.mintDelegationProof(
            new DelegationRequest(
                "easynet:///r/example/user/alice",
                "easynet:///r/example/user/alice",
                CALLER,
                CALLEE,
                List.of("invoke"),
                10,
                20,
                Map.of("trace", "delegation")));
    check(delegation.metadataValue().equals(delegationValue), "delegation projection");

    SessionAuthority session =
        authority.mintSessionAuthority(
            new SessionAuthorityRequest(
                CALLER,
                "session-1",
                "alice",
                CALLER,
                CALLEE,
                "easynet:///r/example/user/alice",
                CALLEE,
                List.of("invoke"),
                List.of("invoke"),
                List.of("observe.health"),
                10,
                20,
                Map.of("trace", "session")));
    check(session.metadataValue().equals(sessionValue), "session projection");

    InvocationDraft authorized =
        completeBuilder().authorityMetadata(delegation.metadata()).inspect();
    check(
        authorized.inspectTuple().metadata().containsKey("x-runtime-delegation"),
        "delegation attached once");

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            completeBuilder()
                .metadata(
                    Map.of(
                        "x-runtime-delegation",
                        delegationValue,
                        "x-runtime-session-authority",
                        sessionValue))
                .inspect());
    authority.close();
    expectSDKError(
        ErrorCode.INVALID_HANDLE,
        () ->
            authority.mintDelegationProof(
                new DelegationRequest(
                    "easynet:///r/example/user/alice",
                    "easynet:///r/example/user/alice",
                    CALLER,
                    CALLEE,
                    List.of("invoke"),
                    10,
                    20,
                Map.of())));
  }

  private static void invocationAuthorityMetadataIsTupleBound() {
    DelegationProof validDelegation = DelegationProof.fromMetadata(delegationMetadataValue());
    completeBuilder().authorityMetadata(validDelegation.metadata()).inspect();

    Map<String, Object> delegationPayload = new LinkedHashMap<>();
    delegationPayload.put("issuer_ura", "easynet:///r/example/user/alice");
    delegationPayload.put("subject_ura", "easynet:///r/example/user/alice");
    delegationPayload.put("caller_ura", CALLER);
    delegationPayload.put("audience", CALLEE);
    delegationPayload.put("scopes", List.of("observe.health"));
    delegationPayload.put("issued_at_ms", 10);
    delegationPayload.put("expires_at_ms", 20);
    DelegationProof mismatchedDelegation =
        DelegationProof.fromMetadata(authorityMetadataValue(delegationPayload));
    expectSDKError(
        ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
        "delegation authority subject does not match invocation subject_ura",
        () -> completeBuilder().authorityMetadata(mismatchedDelegation.metadata()).inspect());

    SessionAuthority session = SessionAuthority.fromMetadata(sessionMetadataValue());
    expectSDKError(
        ErrorCode.AUTHORITY_SUBJECT_MISMATCH,
        "session authority subject does not admit invocation subject_ura",
        () -> completeBuilder().authorityMetadata(session.metadata()).inspect());
  }

  private static void authorityMetadataRejectsAllZeroSessionOwners() {
    Map<String, Object> payload = new LinkedHashMap<>();
    payload.put("issuer_ura", CALLER);
    payload.put("session_id", "session-1");
    payload.put("session_owner_user_id", "00000000-0000-0000-0000-000000000000");
    payload.put("creator_principal_id", CALLER);
    payload.put("callee_ura", CALLEE);
    payload.put("subject_ura", "easynet:///r/example/user/alice");
    payload.put("audience", CALLEE);
    payload.put("scopes", List.of("invoke"));
    payload.put("allowed_actions", List.of("invoke"));
    payload.put("allowed_followup_abilities", List.of("observe.health"));
    payload.put("issued_at_ms", 10);
    payload.put("expires_at_ms", 20);
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> SessionAuthority.fromMetadata(authorityMetadataValue(payload)));

    Map<String, Object> creatorPayload = new LinkedHashMap<>(payload);
    creatorPayload.put("session_owner_user_id", "alice");
    creatorPayload.put("creator_principal_id", "00000000-0000-0000-0000-000000000000");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> SessionAuthority.fromMetadata(authorityMetadataValue(creatorPayload)));

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () ->
            new SessionAuthorityRequest(
                CALLER,
                "session-1",
                "alice",
                "00000000-0000-0000-0000-000000000000",
                CALLEE,
                "easynet:///r/example/user/alice",
                CALLEE,
                List.of("invoke"),
                List.of("invoke"),
                List.of("observe.health"),
                10,
                20,
                Map.of()));
  }

  private static void authorityMetadataBindsSessionAuthoritySubjects() {
    Map<String, Object> payload = new LinkedHashMap<>();
    payload.put("issuer_ura", CALLER);
    payload.put("session_id", "session-1");
    payload.put("session_owner_user_id", "alice");
    payload.put("creator_principal_id", CALLER);
    payload.put("callee_ura", CALLEE);
    payload.put("subject_ura", "easynet:///r/example/user/bob");
    payload.put("audience", CALLEE);
    payload.put("scopes", List.of("invoke"));
    payload.put("allowed_actions", List.of("invoke"));
    payload.put("allowed_followup_abilities", List.of("observe.health"));
    payload.put("issued_at_ms", 10);
    payload.put("expires_at_ms", 20);
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "session authority user subject must match session_owner_user_id",
        () -> SessionAuthority.fromMetadata(authorityMetadataValue(payload)));

    Map<String, Object> sessionPayload = new LinkedHashMap<>(payload);
    sessionPayload.put("subject_ura", "easynet:///r/example/resource/user.alice/session/session-2");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "session authority subject_ura owner/session must match session_owner_user_id and session_id",
        () -> SessionAuthority.fromMetadata(authorityMetadataValue(sessionPayload)));

    Map<String, Object> dottedOwnerPayload = new LinkedHashMap<>(payload);
    dottedOwnerPayload.put("session_owner_user_id", "teamalice");
    dottedOwnerPayload.put(
        "subject_ura", "easynet:///r/example/resource/user.team.alice/session/session-1");
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "session authority subject_ura must be a canonical user or session subject",
        () -> SessionAuthority.fromMetadata(authorityMetadataValue(dottedOwnerPayload)));

    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        "session authority subject_ura must be a canonical user or session subject",
        () ->
            new SessionAuthorityRequest(
                CALLER,
                "session-1",
                "alice",
                CALLER,
                CALLEE,
                CALLEE,
                CALLEE,
                List.of("invoke"),
                List.of("invoke"),
                List.of("observe.health"),
                10,
                20,
                Map.of()));
  }

  private static void streamAndBidiLifecyclesAreBounded() {
    StreamHandle stream = new StreamHandle(new CountingStreamSource());
    for (int index = 0; index <= StreamHandle.MAX_RETAINED_EVENTS; index++) {
      stream.next();
    }
    check(
        stream.terminalEvent() == null
            && stream.transportTerminalEvent() != null
            && stream.transportTerminalEvent().transportTerminal(),
        "stream backpressure transport terminal");
    check(
        stream.retainedEvents().size() == StreamHandle.MAX_RETAINED_EVENTS + 1,
        "stream retained history bound");
    stream.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, stream::next);

    BidiSession bidi = new BidiSession(new CountingBidiSource());
    bidi.send(BidiFrame.data(0, "{\"hello\":true}"));
    BidiFrame sendClosed = bidi.closeSend();
    check(
        !sendClosed.terminal() && sendClosed.transportTerminal(),
        "bidi send side close is transport terminal");
    expectSDKError(ErrorCode.CANCELLED, () -> bidi.send(BidiFrame.data(1, "{}")));
    check(!bidi.next().terminal(), "receive side remains open after close-send");
    BidiFrame cancelled = bidi.cancel("done");
    check(!cancelled.terminal() && cancelled.transportTerminal(), "bidi cancellation transport terminal");
    bidi.close();
  }

  private static void bidiFrame0IsRequiredBeforeRuntimeSessionEntry() {
    MemoryRuntimeTransport transport = new MemoryRuntimeTransport();
    RuntimeClient runtime = new RuntimeClient(transport);
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.openBidi(completeDraft(runtime), null));
    check(transport.openedBidi == 0, "missing bidi frame0 must not enter runtime transport");

    AsyncRuntimeClient async = new AsyncRuntimeClient(transport, Runnable::run);
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT, () -> async.openBidiAsync(completeDraft(async), null));
    check(transport.openedBidi == 0, "async missing bidi frame0 must not enter runtime transport");
  }

  private static void asyncRuntimeDelegatesToTheSameRuntimeStateMachine() throws Exception {
    AsyncRuntimeClient runtime = new AsyncRuntimeClient(new MemoryRuntimeTransport(), Runnable::run);
    InvocationResult result = runtime.invokeAsync(completeDraft(runtime)).get();
    check(result.ok(), "async invocation result");
    runtime.close();
    expectSDKError(ErrorCode.INVALID_HANDLE, runtime::newInvocation);
  }

  private static void typedErrorsPreserveStableCategories() {
    check(SDKError.validation("test", "bad").errorClass() == ErrorClass.VALIDATION, "validation error class");
    check(
        sdkError(ErrorCode.AUTHORITY_DENIED).errorClass() == ErrorClass.ADMISSION,
        "authority admission class");
    check(
        sdkError(ErrorCode.ROUTE_UNAVAILABLE).errorClass() == ErrorClass.ROUTING,
        "route routing class");
    check(
        sdkError(ErrorCode.CALLER_IDENTITY_UNAVAILABLE).errorClass() == ErrorClass.PERMISSION,
        "caller identity permission class");
    check(
        sdkError(ErrorCode.CALLER_SIGNER_UNAVAILABLE).errorClass() == ErrorClass.ADMISSION,
        "caller signer admission class");
    check(
        sdkError(ErrorCode.DESCRIPTOR_NOT_FOUND).errorClass() == ErrorClass.ROUTING,
        "descriptor routing class");
    check(
        sdkError(ErrorCode.RUNTIME_OFFLINE).errorClass() == ErrorClass.AVAILABILITY,
        "runtime offline availability class");
  }

  private static void abiCompatibleAcceptsExactVersion() {
    Client client = discoveryClient();
    check(client.requireABI(5).abiVersion() == 5, "exact ABI accepted");
  }

  private static void abiIncompatibleRejectsMismatch() {
    Client client = discoveryClient();
    expectSDKError(ErrorCode.VERSION_INCOMPATIBLE, () -> client.requireABI(4));
  }

  private static void retryHintsPreserveRetryability() {
    SDKError safe = new SDKError(ErrorCode.TIMEOUT, "execution", RetryHint.SAFE, true, "timeout", "", "", "", Map.of(), null);
    SDKError never = SDKError.validation("input", "bad");
    check(safe.retryHint() == RetryHint.SAFE && safe.retryable(), "safe retry hint");
    check(never.retryHint() == RetryHint.NEVER && !never.retryable(), "never retry hint");
  }

  private static void canonicalSigningMaterialComesFromPrepare() {
    RuntimeClient runtime = new RuntimeClient(new MemoryRuntimeTransport());
    PreparedInvocation prepared = runtime.prepare(completeDraft(runtime), Map.of("deadline_ms", 1000));
    check(prepared.signingMaterial().descriptorRef().equals(DESCRIPTOR), "canonical descriptor binding");
    check(Base64.getDecoder().decode(prepared.signingMaterial().canonicalBytesBase64()).length > 0, "canonical bytes");
  }

  private static void preparedInvocationRequiresExplicitDescriptorRef() {
    RuntimeClient runtime = new RuntimeClient(new MemoryRuntimeTransport());
    Map<String, Object> material =
        Map.of(
            "algorithm", "ed25519",
            "canonical_bytes_base64", "Y2Fub25pY2Fs",
            "args_digest_hex", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "descriptor_ref", DESCRIPTOR,
            "expires_at_unix_ms", 4102444800000L);
    Map<String, Object> prepared = new LinkedHashMap<>();
    prepared.put("prepared_id", "prepared-1");
    prepared.put("request_id", "request-1");
    prepared.put("tuple", JsonValueReader.object(completeDraft(runtime).toJSON(), "draft"));
    prepared.put("signing_material", material);
    prepared.put("descriptor_hash_hex", "");
    prepared.put("schema_hash_hex", "");
    prepared.put("canonical_hash_hex", "");
    prepared.put("expires_at_unix_ms", 4102444800000L);
    prepared.put("submit_ready", false);
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> PreparedInvocation.fromJSON(JsonValueWriter.object(prepared)));
  }

  private static void completeTupleRejectsMissingCaller() {
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> new InvocationBuilder().callee(CALLEE).descriptor(DESCRIPTOR).subject(CALLEE).nonce(NONCE).causalContext("{\"form\":\"none\"}").argsJson("{}").inspect());
  }

  private static void completeTupleRejectsAllZeroPrincipals() {
    String placeholder =
        "easynet:///r/example/resource/user.00000000-0000-0000-0000-000000000000/session/invocation_history";
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> completeBuilder().caller(placeholder).inspect());
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> completeBuilder().callee(placeholder).inspect());
    expectSDKError(
        ErrorCode.INVALID_ARGUMENT,
        () -> completeBuilder().subject(placeholder).inspect());
  }

  private static void preparedInvocationCannotBeSubmitted() {
    RuntimeClient runtime = new RuntimeClient(new MemoryRuntimeTransport());
    PreparedInvocation prepared = runtime.prepare(completeDraft(runtime), Map.of("deadline_ms", 1000));
    expectSDKError(ErrorCode.INVALID_ARGUMENT, () -> runtime.submitSigned(prepared));
  }

  private static void streamAndBidiBackpressureAreBounded() {
    StreamHandle stream = new StreamHandle(new CountingStreamSource());
    for (int index = 0; index <= StreamHandle.MAX_RETAINED_EVENTS; index++) stream.next();
    check(stream.terminalEvent() == null, "stream backpressure is not runtime terminal");
    check(
        stream.transportTerminalEvent() != null
            && stream.transportTerminalEvent().transportTerminal(),
        "stream backpressure transport terminal");
    BidiSession bidi = new BidiSession(new CountingBidiSource());
    for (int index = 0; index <= BidiSession.MAX_RETAINED_FRAMES; index++) bidi.next();
    check(bidi.terminalFrame() == null, "bidi backpressure is not runtime terminal");
    check(
        bidi.transportTerminalFrame() != null
            && "backpressure_terminated".equals(bidi.transportTerminalFrame().kind())
            && bidi.transportTerminalFrame().transportTerminal(),
        "bidi backpressure transport terminal");
  }

  private static void streamOrderAndTerminalArePreserved() {
    StreamHandle stream = new StreamHandle(new OrderedTerminalStreamSource());
    check(stream.next().sequence() == 0, "first stream sequence");
    check(stream.next().sequence() == 1 && stream.terminalEvent().terminal(), "terminal stream sequence");
  }

  private static Client discoveryClient() {
    return new Client(new DiscoveryTransport() {
      public FeatureSet featureDiscovery() {
        return new FeatureSet(5, "test", Map.of("runtime_core", "seam"), Map.of("runtime_prepare", true));
      }
    });
  }

  private static InvocationDraft completeDraft(RuntimeClient runtime) {
    return complete(runtime.newInvocation());
  }

  private static InvocationDraft completeDraft(AsyncRuntimeClient runtime) {
    return complete(runtime.newInvocation());
  }

  private static InvocationBuilder completeBuilder() {
    return new InvocationBuilder()
        .caller(CALLER)
        .callee(CALLEE)
        .descriptor(DESCRIPTOR)
        .subject(CALLEE)
        .nonce(NONCE)
        .causalContext("{\"form\":\"none\"}")
        .argsJson("{\"probe\":true}")
        .metadata(Map.of("trace_id", "trace-1"));
  }

  private static InvocationDraft complete(InvocationBuilder builder) {
    return builder
        .caller(CALLER)
        .callee(CALLEE)
        .descriptor(DESCRIPTOR)
        .subject(CALLEE)
        .nonce(NONCE)
        .causalContext("{\"form\":\"none\"}")
        .argsJson("{\"probe\":true}")
        .metadata(Map.of("trace_id", "trace-1"))
        .inspect();
  }

  private static String delegationMetadataValue() {
    Map<String, Object> payload = new LinkedHashMap<>();
    payload.put("issuer_ura", "easynet:///r/example/user/alice");
    payload.put("subject_ura", CALLEE);
    payload.put("caller_ura", CALLER);
    payload.put("audience", CALLEE);
    payload.put("scopes", List.of("observe.health"));
    payload.put("issued_at_ms", 10);
    payload.put("expires_at_ms", 20);
    return authorityMetadataValue(payload);
  }

  private static String sessionMetadataValue() {
    Map<String, Object> payload = new LinkedHashMap<>();
    payload.put("issuer_ura", CALLER);
    payload.put("session_id", "session-1");
    payload.put("session_owner_user_id", "alice");
    payload.put("creator_principal_id", CALLER);
    payload.put("callee_ura", CALLEE);
    payload.put("subject_ura", "easynet:///r/example/user/alice");
    payload.put("audience", CALLEE);
    payload.put("scopes", List.of("invoke"));
    payload.put("allowed_actions", List.of("invoke"));
    payload.put("allowed_followup_abilities", List.of("observe.health"));
    payload.put("issued_at_ms", 10);
    payload.put("expires_at_ms", 20);
    return authorityMetadataValue(payload);
  }

  private static String authorityMetadataValue(Map<String, Object> payload) {
    String signature = Base64.getEncoder().encodeToString("signature".getBytes(StandardCharsets.UTF_8));
    byte[] wire = JsonValueWriter.object(Map.of("payload", payload, "signature", signature));
    return Base64.getEncoder().encodeToString(wire);
  }

  private static SDKError sdkError(ErrorCode code) {
    return new SDKError(
        code,
        "test",
        RetryHint.NEVER,
        false,
        "test error",
        "",
        "",
        "",
        Map.of(),
        null);
  }

  private static void expectSDKError(ErrorCode code, ThrowingRunnable action) {
    expectSDKError(code, "", action);
  }

  private static void expectSDKError(ErrorCode code, String messageFragment, ThrowingRunnable action) {
    try {
      action.run();
    } catch (SDKError error) {
      check(error.code() == code, "expected " + code + " but got " + error.code());
      if (!messageFragment.isBlank()) {
        check(
            error.getMessage().contains(messageFragment),
            "expected error message to contain " + messageFragment + " but got " + error.getMessage());
      }
      return;
    } catch (Exception error) {
      throw new AssertionError("expected SDKError but got " + error, error);
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

  private static final class MemoryHealthTransport implements HealthTransport, DiagnosticsTransport {
    @Override
    public byte[] runtimeHealth() {
      return bytes(
          """
          {
            "api_ready": true,
            "invocation_ready": false,
            "directory_ready": false,
            "trust_ready": true,
            "runtime_ready": false,
            "version": "0.0.0-seam",
            "abi_version": 5,
            "mismatch": null,
            "diagnostics": ["runtime warming"]
          }
          """);
    }

    @Override
    public byte[] runtimeDiagnostics() {
      return bytes(
          """
          {
            "profile": "health",
            "kind": "diagnostics_report",
            "state": "Running",
            "ready": false,
            "version": "0.0.0-seam",
            "abi_version": 5,
            "control_endpoint": "/tmp/easynet-control.sock",
            "invocation_endpoint": "/tmp/easynet-daemon.sock",
            "checks": [{"name":"runtime","ready":false,"message":"warming"}],
            "diagnostics": ["runtime warming"]
          }
          """);
    }
  }

  private static final class MemoryRuntimeTransport implements RuntimeTransport {
    private String submittedSigner = "";
    private long eventHandleId = 7;
    private int openedBidi = 0;

    @Override
    public InvocationResult invoke(InvocationDraft draft) {
      check(draft.inspectTuple().caller().equals(CALLER), "runtime caller preserved");
      return new InvocationResult(
          true,
          InvocationTerminalState.COMPLETED,
          "{\"ok\":true}",
          null,
          canonicalRuntimeReceiptFixture("inv-direct", "completed", "Completed", 1));
    }

    @Override
    public byte[] prepare(byte[] draftJson, byte[] optionsJson) {
      Map<String, Object> tuple = JsonValueReader.object(draftJson, "draft");
      Map<String, Object> options = JsonValueReader.object(optionsJson, "options");
      check(options.get("deadline_ms").equals(1000L), "prepare options preserved");
      Map<String, Object> material =
          Map.of(
              "algorithm", "ed25519",
              "canonical_bytes_base64", "Y2Fub25pY2Fs",
              "args_digest_hex", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "descriptor_ref", DESCRIPTOR,
              "expires_at_unix_ms", 4102444800000L);
      Map<String, Object> prepared = new LinkedHashMap<>();
      prepared.put("prepared_id", "prepared-1");
      prepared.put("request_id", "request-1");
      prepared.put("tuple", tuple);
      prepared.put("signing_material", material);
      prepared.put("descriptor_ref", DESCRIPTOR);
      prepared.put("descriptor_hash_hex", "");
      prepared.put("schema_hash_hex", "");
      prepared.put("canonical_hash_hex", "");
      prepared.put("expires_at_unix_ms", 4102444800000L);
      prepared.put("submit_ready", false);
      return JsonValueWriter.object(prepared);
    }

    @Override
    public byte[] submitSigned(byte[] signedJson) {
      Map<String, Object> signed = JsonValueReader.object(signedJson, "signed");
      submittedSigner = String.valueOf(signed.get("signer_id"));
      return JsonValueWriter.object(Map.of("handle_id", 7, "state", "Running", "terminal", false));
    }

    @Override
    public byte[] awaitHandle(InvocationControlCapability control) {
      return JsonValueWriter.object(
          Map.of(
              "ok",
              true,
              "terminal_state",
              "Completed",
              "output_json",
              Map.of("done", true),
              "terminal_receipt",
              canonicalRuntimeReceiptFixture("inv-await", "completed", "Completed", 1)));
    }

    @Override
    public byte[] cancelHandle(InvocationControlCapability control, String reason) {
      return JsonValueWriter.object(
          Map.of(
              "handle_id",
              control.adapterHandleId(),
              "request_accepted",
              true,
              "deduplicated",
              false,
              "cancelled",
              true,
              "state",
              "Cancelled",
              "terminal",
              true));
    }

    @Override
    public byte[] handleEvents(InvocationControlCapability control) {
      return JsonValueWriter.object(
          Map.of("handle_id", eventHandleId, "state", "Completed", "terminal", true));
    }

    @Override
    public void freeHandle(InvocationControlCapability control) {}

    @Override
    public StreamSource openStream(InvocationDraft draft) {
      return new CountingStreamSource();
    }

    @Override
    public BidiSource openBidi(InvocationDraft draft, BidiFrame frame0) {
      openedBidi++;
      return new CountingBidiSource();
    }
  }

  private static final class MemoryAuthorityTransport implements AuthorityTransport {
    private final String delegationValue;
    private final String sessionValue;

    private MemoryAuthorityTransport(String delegationValue, String sessionValue) {
      this.delegationValue = delegationValue;
      this.sessionValue = sessionValue;
    }

    @Override
    public byte[] mintDelegationProof(byte[] requestJSON) {
      check(
          JsonValueReader.object(requestJSON, "delegation request").get("caller_ura").equals(CALLER),
          "delegation caller");
      return JsonValueWriter.object(Map.of("metadata_value", delegationValue));
    }

    @Override
    public byte[] mintSessionAuthority(byte[] requestJSON) {
      check(
          JsonValueReader.object(requestJSON, "session request").get("session_id").equals("session-1"),
          "session id");
      return JsonValueWriter.object(
          Map.of("metadata", Map.of("x-runtime-session-authority", sessionValue)));
    }
  }

  private static final class CountingStreamSource implements StreamSource {
    private long sequence;

    @Override
    public StreamEvent next() {
      return StreamEvent.data(sequence++, "{\"sequence\":" + sequence + "}");
    }
  }

  private static final class OrderedTerminalStreamSource implements StreamSource {
    private long sequence;

    @Override
    public StreamEvent next() {
      return sequence++ == 0 ? StreamEvent.data(0, "{}") : StreamEvent.terminal(1, "Completed");
    }
  }

  private static final class CountingBidiSource implements BidiSource {
    private final ArrayDeque<BidiFrame> frames = new ArrayDeque<>();
    private long sequence;

    @Override
    public void send(BidiFrame frame) {
      frames.addLast(frame);
    }

    @Override
    public BidiFrame next() {
      if (!frames.isEmpty()) {
        return frames.removeFirst();
      }
      return BidiFrame.data(sequence++, "{}");
    }
  }

  private static Map<String, Object> canonicalRuntimeReceiptFixture(
      String invocationId, String receiptType, String state, long index) {
    byte[] proofPayload = bytes("canonical-runtime-test-proof");
    Map<String, Object> receipt = new LinkedHashMap<>();
    receipt.put(
        "receipt_ura",
        "easynet:///r/example/resource/runtime/invocation/"
            + invocationId
            + "/receipt/"
            + index);
    receipt.put("invocation_id", invocationId);
    receipt.put("receipt_type", receiptType);
    receipt.put("state", state);
    receipt.put("index", index);
    receipt.put("timestamp_unix_ms", 1_783_100_000_000L + index);
    receipt.put("prev_receipt_hash_hex", "00".repeat(32));
    receipt.put("self_hash_hex", "%064x".formatted(index + 1));
    receipt.put("cleanup_complete", !"admitted".equalsIgnoreCase(state));
    receipt.put("caller_binding", agentBinding(CALLER));
    receipt.put("callee_binding", agentBinding(CALLEE));
    receipt.put("subject_binding", agentBinding(CALLEE));
    receipt.put("invocation_nonce_base64", NONCE);
    receipt.put("causal_binding_kind", "none");
    receipt.put("causal_binding", Map.of("form", "none"));
    receipt.put(
        "callee_signature",
        Map.of(
            "algorithm",
            "ed25519",
            "signature_base64",
            Base64.getEncoder().encodeToString(repeatedByte(0x71, 64))));
    receipt.put("signer_binding", agentBinding(CALLEE));
    receipt.put("authority_binding_kind", "self");
    receipt.put(
        "authority_binding",
        Map.of("kind", "self", "principal_ura", CALLEE));
    receipt.put("ability_binding", DESCRIPTOR);
    receipt.put("subject_ref", Map.of("kind", 1L, "ura", CALLEE, "profile", "axon-strict-v2"));
    receipt.put("descriptor_version", "1.0.0");
    receipt.put("schema_hash_hex", "11".repeat(32));
    receipt.put("impl_hash_hex", "22".repeat(32));
    receipt.put("runtime_env", "java-test");
    receipt.put(
        "authority_proof",
        Map.of(
            "proof_type",
            "self",
            "binding_kind",
            "self",
            "binding",
            Map.of("kind", "self", "principal_ura", CALLEE),
            "proof_payload_base64",
            Base64.getEncoder().encodeToString(proofPayload),
            "proof_hash_hex",
            sha256Hex(proofPayload),
            "issuer",
            agentBinding(CALLEE),
            "signature",
            Map.of(
                "algorithm",
                "ed25519",
                "signature_base64",
                Base64.getEncoder().encodeToString(repeatedByte(0x72, 64))),
            "admission_hook",
            "test.runtime.admission"));
    receipt.put("input_hash_hex", "33".repeat(32));
    receipt.put("output_hash_hex", "44".repeat(32));
    receipt.put("parent_receipts", List.of());
    return Map.copyOf(receipt);
  }

  private static Map<String, Object> agentBinding(String ura) {
    return Map.of("ura", ura, "profile", "axon-strict-v2");
  }

  private static Map<String, Object> nullableMapOf(Object... entries) {
    if (entries.length % 2 != 0) {
      throw new IllegalArgumentException("nullable map entries must be key/value pairs");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (int i = 0; i < entries.length; i += 2) {
      Object key = entries[i];
      if (!(key instanceof String text) || text.isBlank()) {
        throw new IllegalArgumentException("nullable map keys must be non-empty strings");
      }
      out.put(text, entries[i + 1]);
    }
    return out;
  }

  private static byte[] repeatedByte(int value, int count) {
    byte[] bytes = new byte[count];
    java.util.Arrays.fill(bytes, (byte) value);
    return bytes;
  }

  private static Map<String, Object> mutableAuthorityProof(Map<String, Object> receipt) {
    @SuppressWarnings("unchecked")
    Map<String, Object> rawProof = (Map<String, Object>) receipt.get("authority_proof");
    Map<String, Object> proof = new LinkedHashMap<>(rawProof);
    @SuppressWarnings("unchecked")
    Map<String, Object> rawBinding = (Map<String, Object>) proof.get("binding");
    proof.put("binding", new LinkedHashMap<>(rawBinding));
    receipt.put("authority_proof", proof);
    return proof;
  }

  private static String authorityBindingProofHashSelf(String principalURA) {
    byte[] principal = bytes(principalURA);
    ByteBuffer canonical = ByteBuffer.allocate(1 + 4 + principal.length);
    canonical.put((byte) 0x01);
    canonical.putInt(principal.length);
    canonical.put(principal);
    return sha256Hex(canonical.array());
  }

  private static String sha256Hex(byte[] bytes) {
    try {
      return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    } catch (NoSuchAlgorithmException error) {
      throw new AssertionError(error);
    }
  }

  private static byte[] bytes(String value) {
    return value.getBytes(StandardCharsets.UTF_8);
  }
}
