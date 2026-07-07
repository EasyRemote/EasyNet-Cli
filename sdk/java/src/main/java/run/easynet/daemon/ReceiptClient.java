package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

public final class ReceiptClient implements AutoCloseable {
  private final ReceiptTransport transport;
  private boolean closed;

  public ReceiptClient(ReceiptTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public ReceiptSummary fetch(ReceiptFetchRequest request) {
    requireOpen();
    return ReceiptSummary.fromJSON(callRaw(() -> transport.fetch(request.toJSON()), "receipt fetch transport failed"));
  }

  public Map<String, Object> buildFetchInvocation(ReceiptFetchRequest request) {
    Objects.requireNonNull(request, "request");
    LinkedHashMap<String, Object> invocation = new LinkedHashMap<>();
    invocation.put("caller_ura", request.callerURA());
    invocation.put("callee_ura", request.calleeURA());
    invocation.put("descriptor_ref", request.descriptorRef());
    invocation.put("subject_ura", request.subjectURA());
    invocation.put("nonce_base64", request.nonceBase64());
    invocation.put("causal_context", request.causalContext());
    invocation.put("args", Map.of("key", selector(request)));
    invocation.put("content_type", "application/json");
    LinkedHashMap<String, Object> metadata = new LinkedHashMap<>(request.metadata());
    metadata.put("profile", ReceiptSupport.PROFILE);
    metadata.put("system_ability", ReceiptSupport.FETCH_ABILITY);
    metadata.put("carrier_owner", "daemon_sdk");
    invocation.put("metadata", metadata);
    return invocation;
  }

  public ReceiptSummary project(byte[] receiptJSON) {
    requireOpen();
    return ReceiptSummary.fromJSON(receiptJSON);
  }

  public ReceiptVerification verifySummary(ReceiptSummary summary) {
    return Objects.requireNonNull(summary, "summary").summaryVerification();
  }

  public Map<String, Object> causalRef(ReceiptRef ref) {
    requireOpen();
    return JsonValueReader.object(
        callRaw(() -> transport.causalRef(ref.toJSON()), "receipt causal-ref transport failed"),
        "receipt causal ref JSON");
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private byte[] callRaw(RawCall call, String message) {
    try {
      return call.run();
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw new SDKError(
          ErrorCode.TRANSPORT,
          "transport",
          RetryHint.SAFE,
          true,
          message,
          "",
          "",
          "",
          Map.of("profile", ReceiptSupport.PROFILE),
          error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("receipt");
    }
  }

  private static Map<String, Object> selector(ReceiptFetchRequest request) {
    if (!request.invocationURA().isEmpty()) {
      return Map.of("invocation_ura", request.invocationURA());
    }
    if (!request.requestID().isEmpty()) {
      return Map.of("request_id", request.requestID());
    }
    return Map.of("trace_id", request.traceID());
  }

  @FunctionalInterface
  private interface RawCall {
    byte[] run();
  }
}
