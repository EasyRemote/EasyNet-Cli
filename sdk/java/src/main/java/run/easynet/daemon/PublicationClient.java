package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class PublicationClient implements AutoCloseable {
  private final PublicationTransport transport;
  private boolean closed;

  public PublicationClient(PublicationTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public ResourceRef buildLocalResourceRef(LocalResourceRefRequest request) {
    ensureOpen();
    Objects.requireNonNull(request, "request");
    return ResourceRef.fromJSON(transport.buildResourceRef(request.toJSON()));
  }

  public PackageValidation validatePackage(String packagePath, ValidatePackageOptions options) {
    ensureOpen();
    var resolved = options == null ? new ValidatePackageOptions(null, Map.of()) : options;
    return PackageValidation.fromJSON(
        transport.validatePackage(JsonValueWriter.object(resolved.toObject(packagePath))));
  }

  public Map<String, Object> buildDeployInvocation(AbilityDeployRequest request) {
    ensureOpen();
    Objects.requireNonNull(request, "request");
    return JsonValueReader.object(
        transport.buildDeployInvocation(request.toJSON()), "publication deploy invocation JSON");
  }

  public Map<String, Object> buildUnpublishInvocation(UnpublishAbilityRequest request) {
    ensureOpen();
    Objects.requireNonNull(request, "request");
    return JsonValueReader.object(
        transport.buildUnpublishInvocation(request.toJSON()),
        "publication unpublish invocation JSON");
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private void ensureOpen() {
    if (closed) {
      throw SDKError.closed("publication");
    }
  }
}
