package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class SurfaceClient implements AutoCloseable {
  private final SurfaceTransport transport;
  private boolean closed;

  public SurfaceClient(SurfaceTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public Map<String, Object> buildListPagesInvocation(SurfaceListPagesRequest request) {
    return build(request.toJSON(), transport::buildListPagesInvocation, "surface list-pages invocation failed");
  }

  public Map<String, Object> buildCreatePageInvocation(SurfaceCreatePageRequest request) {
    return build(request.toJSON(), transport::buildCreatePageInvocation, "surface create-page invocation failed");
  }

  public Map<String, Object> buildDeletePageInvocation(SurfaceDeletePageRequest request) {
    return build(request.toJSON(), transport::buildDeletePageInvocation, "surface delete-page invocation failed");
  }

  public Map<String, Object> buildManifestInvocation(SurfaceManifestRequest request) {
    return build(request.toJSON(), transport::buildManifestInvocation, "surface manifest invocation failed");
  }

  public Map<String, Object> buildHealthInvocation(SurfaceHealthRequest request) {
    return build(request.toJSON(), transport::buildHealthInvocation, "surface health invocation failed");
  }

  public SurfacePagePage listPages(SurfaceListPagesRequest request) {
    return SurfacePagePage.fromJSON(raw(request.toJSON(), transport::listPages, "surface list pages failed"));
  }

  public SurfacePageRecord createPage(SurfaceCreatePageRequest request) {
    return SurfacePageRecord.fromJSON(raw(request.toJSON(), transport::createPage, "surface create page failed"));
  }

  public SurfaceMutationResult deletePage(SurfaceDeletePageRequest request) {
    return SurfaceMutationResult.fromJSON(raw(request.toJSON(), transport::deletePage, "surface delete page failed"));
  }

  public SurfaceManifest surfaceManifest(SurfaceManifestRequest request) {
    return SurfaceManifest.fromJSON(raw(request.toJSON(), transport::surfaceManifest, "surface manifest failed"));
  }

  public SurfacePublicPageRef publicPageRef(SurfacePageRecord page) {
    return SurfacePublicPageRef.fromJSON(
        raw(JsonValueWriter.object(Objects.requireNonNull(page, "page").toObject()), transport::publicPageRef, "surface public page ref failed"));
  }

  public SurfaceHealth surfaceHealth(SurfaceHealthRequest request) {
    return SurfaceHealth.fromJSON(raw(request.toJSON(), transport::surfaceHealth, "surface health failed"));
  }

  public SurfaceHealth surfaceStatus(SurfaceHealthRequest request) {
    return surfaceHealth(request);
  }

  public SurfacePagePage projectPagePage(byte[] raw) {
    return SurfacePagePage.fromJSON(raw);
  }

  public SurfaceManifest projectManifest(byte[] raw) {
    return SurfaceManifest.fromJSON(raw);
  }

  public SurfaceHealth projectHealth(byte[] raw) {
    return SurfaceHealth.fromJSON(raw);
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private Map<String, Object> build(byte[] requestJSON, SurfaceBytesOperation operation, String message) {
    return JsonValueReader.object(raw(requestJSON, operation, message), "surface invocation JSON");
  }

  private byte[] raw(byte[] requestJSON, SurfaceBytesOperation operation, String message) {
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
          Map.of("profile", SurfaceSupport.PROFILE),
          error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("surface");
    }
  }

  @FunctionalInterface
  private interface SurfaceBytesOperation {
    byte[] call(byte[] requestJSON);
  }
}
