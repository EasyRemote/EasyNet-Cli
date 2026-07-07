package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

public final class HostBindingClient implements AutoCloseable {
  private final HostBindingTransport transport;
  private final HostStreamLifecycleProvider lifecycleProvider;
  private boolean closed;

  public HostBindingClient(HostBindingTransport transport) {
    this(transport, null);
  }

  public HostBindingClient(HostBindingTransport transport, HostStreamLifecycleProvider lifecycleProvider) {
    this.transport = Objects.requireNonNull(transport, "transport");
    this.lifecycleProvider = lifecycleProvider;
  }

  public HostStreamBinding buildHostStreamBinding(HostStreamBindingRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return HostStreamBinding.fromJSON(raw(() -> transport.buildHostStreamBinding(request.toJSON()), "host binding build failed"));
  }

  public HostStreamRequest decodeRequest(HostStreamEnvelope envelope) {
    requireOpen();
    Objects.requireNonNull(envelope, "envelope");
    return HostStreamRequest.fromJSON(raw(() -> transport.decodeRequest(envelope.toJSON()), "host binding decode request failed"));
  }

  public HostStreamFrame encodeItem(long seq, Object value) {
    requireOpen();
    LinkedHashMap<String, Object> request = new LinkedHashMap<>();
    request.put("seq", seq);
    request.put("value", value);
    return HostStreamFrame.fromJSON(
        raw(
            () -> transport.encodeItem(JsonValueWriter.object(request)),
            "host binding encode item failed"));
  }

  public HostStreamFrame encodeError(Throwable error) {
    requireOpen();
    Objects.requireNonNull(error, "error");
    LinkedHashMap<String, Object> payload = new LinkedHashMap<>();
    payload.put("message", error.getMessage());
    payload.put("code", error instanceof SDKError sdkError ? sdkError.code().name() : ErrorCode.GENERIC.name());
    payload.put("stage", error instanceof SDKError sdkError ? sdkError.stage() : HostBindingSupport.PROFILE);
    payload.put("retry", error instanceof SDKError sdkError ? sdkError.retryHint().name().toLowerCase() : "never");
    payload.put("details", error instanceof SDKError sdkError ? sdkError.details() : Map.of());
    return HostStreamFrame.fromJSON(
        raw(
            () -> transport.encodeError(JsonValueWriter.object(Map.of("error", payload))),
            "host binding encode error failed"));
  }

  public HostStreamFrame encodeTerminal(HostStreamTerminalSummary summary) {
    requireOpen();
    Objects.requireNonNull(summary, "summary");
    return HostStreamFrame.fromJSON(
        raw(
            () -> transport.encodeTerminal(JsonValueWriter.object(Map.of("summary", summary.toObject()))),
            "host binding encode terminal failed"));
  }

  public HostStreamHashState foldOutputHash(HostStreamHashState state, long seq, Object value) {
    requireOpen();
    HostBindingSupport.validateHashFold(state, seq);
    LinkedHashMap<String, Object> request = new LinkedHashMap<>();
    request.put("state", state.toObject());
    request.put("seq", seq);
    request.put("value", value);
    return HostStreamHashState.fromJSON(
        raw(() -> transport.foldOutputHash(JsonValueWriter.object(request)), "host binding hash fold failed"));
  }

  public HostStreamHashState foldOutputHashLocal(HostStreamHashState state, long seq, Object value) {
    requireOpen();
    return state.fold(seq, value);
  }

  public HostStreamLifecycleController openLifecycle(
      HostStreamBinding binding, HostStreamLifecycleProvider provider) {
    requireOpen();
    HostStreamLifecycleProvider resolved = provider == null ? lifecycleProvider : provider;
    if (resolved == null) {
      throw HostBindingSupport.invalid("host stream lifecycle provider is required");
    }
    return new HostStreamLifecycleController(binding, resolved);
  }

  public HostStreamReadiness checkReadiness(HostStreamBinding binding, HostStreamLifecycleProvider provider) {
    return openLifecycle(binding, provider).checkReadiness();
  }

  public HostStreamCleanup cleanup(HostStreamBinding binding, HostStreamLifecycleProvider provider) {
    return openLifecycle(binding, provider).cleanup();
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private byte[] raw(HostBindingBytesOperation operation, String message) {
    try {
      return operation.call();
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw HostBindingSupport.transport(message, error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("host binding");
    }
  }

  @FunctionalInterface
  private interface HostBindingBytesOperation {
    byte[] call();
  }
}
