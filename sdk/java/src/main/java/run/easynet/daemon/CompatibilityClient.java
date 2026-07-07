package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class CompatibilityClient implements AutoCloseable {
  private final CompatibilityTransport transport;
  private boolean closed;

  public CompatibilityClient(CompatibilityTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public Map<String, Object> buildListModelsInvocation(CompatibilityListModelsRequest request) {
    return build(request.toJSON(), transport::buildListModelsInvocation, "compatibility list-models invocation failed");
  }

  public Map<String, Object> buildChatCompletionInvocation(CompatibilityChatCompletionRequest request) {
    return build(request.toJSON(), transport::buildChatCompletionInvocation, "compatibility chat invocation failed");
  }

  public Map<String, Object> buildStreamChatCompletionInvocation(CompatibilityStreamChatCompletionRequest request) {
    return build(request.toJSON(), transport::buildStreamChatCompletionInvocation, "compatibility stream invocation failed");
  }

  public CompatibilityModelPage listModels(CompatibilityListModelsRequest request) {
    return CompatibilityModelPage.fromJSON(raw(request.toJSON(), transport::listModels, "compatibility list models failed"));
  }

  public CompatibilityChatCompletion chatCompletions(CompatibilityChatCompletionRequest request) {
    return CompatibilityChatCompletion.fromJSON(raw(request.toJSON(), transport::chatCompletions, "compatibility chat failed"));
  }

  public CompatibilityChatCompletionStream streamChatCompletions(CompatibilityStreamChatCompletionRequest request) {
    return CompatibilityChatCompletionStream.fromJSON(
        raw(request.toJSON(), transport::streamChatCompletions, "compatibility stream failed"));
  }

  public CompatibilityFile uploadFile(CompatibilityFileUploadRequest request) {
    return CompatibilityFile.fromJSON(raw(request.toJSON(), transport::uploadFile, "compatibility file upload failed"));
  }

  public CompatibilityFile getFile(CompatibilityFileRequest request) {
    return CompatibilityFile.fromJSON(raw(request.toJSON(), transport::getFile, "compatibility file get failed"));
  }

  public CompatibilityFileDeleteResult deleteFile(CompatibilityFileDeleteRequest request) {
    return CompatibilityFileDeleteResult.fromJSON(
        raw(request.toJSON(), transport::deleteFile, "compatibility file delete failed"));
  }

  public CompatibilityModelPage projectModelPage(byte[] valueJSON) {
    return CompatibilityModelPage.fromJSON(raw(valueJSON, transport::projectModelPage, "compatibility model projection failed"));
  }

  public CompatibilityModelPage projectModelPage(CompatibilityModelPage value) {
    return projectModelPage(Objects.requireNonNull(value, "value").toJSON());
  }

  public CompatibilityChatCompletion projectChatCompletion(byte[] valueJSON) {
    return CompatibilityChatCompletion.fromJSON(
        raw(valueJSON, transport::projectChatCompletion, "compatibility chat projection failed"));
  }

  public CompatibilityChatCompletion projectChatCompletion(CompatibilityChatCompletion value) {
    return projectChatCompletion(Objects.requireNonNull(value, "value").toJSON());
  }

  public CompatibilityChatCompletionStream projectChatStream(byte[] valueJSON) {
    return CompatibilityChatCompletionStream.fromJSON(
        raw(valueJSON, transport::projectChatStream, "compatibility stream projection failed"));
  }

  public CompatibilityChatCompletionStream projectChatStream(CompatibilityChatCompletionStream value) {
    return projectChatStream(Objects.requireNonNull(value, "value").toJSON());
  }

  public CompatibilityFile projectFile(byte[] valueJSON) {
    return CompatibilityFile.fromJSON(raw(valueJSON, transport::projectFile, "compatibility file projection failed"));
  }

  public CompatibilityFile projectFileUpload(CompatibilityFileUploadRequest request) {
    return CompatibilityFile.fromJSON(raw(Objects.requireNonNull(request, "request").toJSON(), transport::projectFileUpload, "compatibility file-upload projection failed"));
  }

  public CompatibilityFile projectFile(CompatibilityFileRequest request) {
    return CompatibilityFile.fromJSON(raw(Objects.requireNonNull(request, "request").toJSON(), transport::projectFile, "compatibility file projection failed"));
  }

  public CompatibilityFile projectFile(CompatibilityFile value) {
    return projectFile(Objects.requireNonNull(value, "value").toJSON());
  }

  public CompatibilityFileDeleteResult projectFileDeleteResult(byte[] valueJSON) {
    return CompatibilityFileDeleteResult.fromJSON(
        raw(valueJSON, transport::projectFileDeleteResult, "compatibility file-delete projection failed"));
  }

  public CompatibilityFileDeleteResult projectFileDeleteResult(CompatibilityFileDeleteRequest request) {
    return CompatibilityFileDeleteResult.fromJSON(
        raw(Objects.requireNonNull(request, "request").toJSON(), transport::projectFileDeleteResult, "compatibility file-delete projection failed"));
  }

  public CompatibilityFileDeleteResult projectFileDeleteResult(CompatibilityFileDeleteResult value) {
    return projectFileDeleteResult(Objects.requireNonNull(value, "value").toJSON());
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private Map<String, Object> build(byte[] requestJSON, CompatibilityBytesOperation operation, String message) {
    return JsonValueReader.object(raw(requestJSON, operation, message), "compatibility invocation JSON");
  }

  private byte[] raw(byte[] requestJSON, CompatibilityBytesOperation operation, String message) {
    requireOpen();
    try {
      return operation.call(requestJSON);
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
          Map.of("profile", CompatibilitySupport.PROFILE),
          error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("compatibility");
    }
  }

  @FunctionalInterface
  private interface CompatibilityBytesOperation {
    byte[] call(byte[] requestJSON);
  }
}
