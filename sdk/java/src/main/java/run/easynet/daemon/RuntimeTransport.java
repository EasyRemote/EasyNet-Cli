package run.easynet.daemon;

public interface RuntimeTransport extends AutoCloseable {
  InvocationResult invoke(InvocationDraft draft);

  StreamSource openStream(InvocationDraft draft);

  BidiSource openBidi(InvocationDraft draft, BidiFrame frame0);

  @Override
  default void close() {}
}
