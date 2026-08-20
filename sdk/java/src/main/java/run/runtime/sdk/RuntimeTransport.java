package run.runtime.sdk;

public interface RuntimeTransport extends AutoCloseable {
  InvocationResult invoke(InvocationDraft draft);

  default byte[] prepare(byte[] draftJson, byte[] optionsJson) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime prepare transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default byte[] submitSigned(byte[] signedJson) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime submit-signed transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default byte[] awaitHandle(InvocationControlCapability control) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime await-handle transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default byte[] cancelHandle(InvocationControlCapability control, String reason) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime cancel-handle transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default byte[] handleEvents(InvocationControlCapability control) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime handle-events transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default void freeHandle(InvocationControlCapability control) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime free-handle transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  default byte[] resolveDescriptorRef(byte[] requestJson) {
    throw new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "runtime",
        RetryHint.NEVER,
        false,
        "runtime descriptor resolver transport is not implemented",
        "",
        "",
        "",
        java.util.Map.of(),
        null);
  }

  StreamSource openStream(InvocationDraft draft);

  BidiSource openBidi(InvocationDraft draft, BidiFrame frame0);

  @Override
  default void close() {}
}
