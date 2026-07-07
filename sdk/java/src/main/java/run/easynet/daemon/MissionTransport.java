package run.easynet.daemon;

public interface MissionTransport extends AutoCloseable {
  default byte[] buildRunEALInvocation(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission run invocation transport is required");
  }

  default byte[] buildRunFileInvocation(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission run-file invocation transport is required");
  }

  default byte[] buildTrackInvocation(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission track invocation transport is required");
  }

  default byte[] buildCancelInvocation(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission cancel invocation transport is required");
  }

  default byte[] buildEventsInvocation(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission events invocation transport is required");
  }

  default byte[] runEAL(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission run transport is required");
  }

  default byte[] runFile(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission run-file transport is required");
  }

  default byte[] track(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission track transport is required");
  }

  default byte[] cancel(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission cancel transport is required");
  }

  default byte[] events(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission events transport is required");
  }

  default StreamHandle openEventStream(byte[] requestJSON) {
    throw MissionSupport.unsupported("mission event stream transport is required");
  }

  @Override
  default void close() {}
}
