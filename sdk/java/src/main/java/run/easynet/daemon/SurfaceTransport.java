package run.easynet.daemon;

public interface SurfaceTransport extends AutoCloseable {
  default byte[] buildListPagesInvocation(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface list-pages invocation transport is not available");
  }

  default byte[] buildCreatePageInvocation(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface create-page invocation transport is not available");
  }

  default byte[] buildDeletePageInvocation(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface delete-page invocation transport is not available");
  }

  default byte[] buildManifestInvocation(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface manifest invocation transport is not available");
  }

  default byte[] buildHealthInvocation(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface health invocation transport is not available");
  }

  default byte[] listPages(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface list pages transport is not available");
  }

  default byte[] createPage(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface create page transport is not available");
  }

  default byte[] deletePage(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface delete page transport is not available");
  }

  default byte[] surfaceManifest(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface manifest transport is not available");
  }

  default byte[] publicPageRef(byte[] pageJSON) {
    throw SurfaceSupport.unsupported("surface public page ref transport is not available");
  }

  default byte[] surfaceHealth(byte[] requestJSON) {
    throw SurfaceSupport.unsupported("surface health transport is not available");
  }

  @Override
  default void close() {}
}
