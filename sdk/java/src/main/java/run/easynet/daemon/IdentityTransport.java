package run.easynet.daemon;

public interface IdentityTransport extends AutoCloseable {
  default byte[] projectDescriptorRef(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("identity projectDescriptorRef is not available");
  }

  default byte[] buildDescriptorRef(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("identity buildDescriptorRef is not available");
  }

  default byte[] ownerAbilityURA(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("identity ownerAbilityURA is not available");
  }

  default byte[] buildURA(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("identity buildURA is not available");
  }

  @Override
  default void close() {}
}
