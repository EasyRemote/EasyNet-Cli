package run.easynet.daemon;

public interface DirectoryTransport extends AutoCloseable {
  default byte[] buildListDevicesInvocation(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented(
        "directory buildListDevicesInvocation is not available");
  }

  default byte[] buildListAgentsInvocation(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented(
        "directory buildListAgentsInvocation is not available");
  }

  default byte[] buildListAbilitiesInvocation(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented(
        "directory buildListAbilitiesInvocation is not available");
  }

  default byte[] buildResolveInvocation(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented(
        "directory buildResolveInvocation is not available");
  }

  default byte[] resolve(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("directory resolve is not available");
  }

  default byte[] listDevices(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("directory listDevices is not available");
  }

  default byte[] listAgents(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("directory listAgents is not available");
  }

  default byte[] listAbilities(byte[] requestJSON) {
    throw DirectoryIdentitySupport.notImplemented("directory listAbilities is not available");
  }

  @Override
  default void close() {}
}
