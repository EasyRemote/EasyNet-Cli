package run.easynet.daemon;

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

  StreamSource openStream(InvocationDraft draft);

  BidiSource openBidi(InvocationDraft draft, BidiFrame frame0);

  @Override
  default void close() {}
}
