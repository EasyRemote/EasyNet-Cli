package run.easynet.daemon;

public interface PublicationTransport extends AutoCloseable {
  default byte[] buildResourceRef(byte[] requestJSON) {
    throw PublicationSupport.unsupported("publication resource-ref transport is not available");
  }

  default byte[] validatePackage(byte[] requestJSON) {
    throw PublicationSupport.unsupported("publication package validation transport is not available");
  }

  default byte[] buildDeployInvocation(byte[] requestJSON) {
    throw PublicationSupport.unsupported("publication deploy invocation transport is not available");
  }

  default byte[] buildUnpublishInvocation(byte[] requestJSON) {
    throw PublicationSupport.unsupported("publication unpublish invocation transport is not available");
  }

  @Override
  default void close() {}
}
