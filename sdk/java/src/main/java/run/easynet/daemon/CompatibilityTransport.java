package run.easynet.daemon;

public interface CompatibilityTransport extends AutoCloseable {
  default byte[] buildListModelsInvocation(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility list-models invocation transport is not available");
  }

  default byte[] buildChatCompletionInvocation(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility chat invocation transport is not available");
  }

  default byte[] buildStreamChatCompletionInvocation(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility stream invocation transport is not available");
  }

  default byte[] listModels(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility list-models transport is not available");
  }

  default byte[] chatCompletions(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility chat transport is not available");
  }

  default byte[] streamChatCompletions(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility stream transport is not available");
  }

  default byte[] uploadFile(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility file upload transport is not available");
  }

  default byte[] getFile(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility file get transport is not available");
  }

  default byte[] deleteFile(byte[] requestJSON) {
    throw CompatibilitySupport.unsupported("compatibility file delete transport is not available");
  }

  default byte[] projectModelPage(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility model-page projection transport is not available");
  }

  default byte[] projectChatCompletion(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility chat projection transport is not available");
  }

  default byte[] projectChatStream(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility stream projection transport is not available");
  }

  default byte[] projectFileUpload(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility file-upload projection transport is not available");
  }

  default byte[] projectFile(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility file projection transport is not available");
  }

  default byte[] projectFileDeleteResult(byte[] valueJSON) {
    throw CompatibilitySupport.unsupported("compatibility file-delete projection transport is not available");
  }

  @Override
  default void close() {}
}
