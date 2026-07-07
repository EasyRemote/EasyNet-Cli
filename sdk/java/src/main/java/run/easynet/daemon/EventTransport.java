package run.easynet.daemon;

public interface EventTransport extends AutoCloseable {
  default byte[] buildDirectorySubscriptionInvocation(byte[] requestJSON) {
    throw EventsSupport.unsupported("events directory subscription invocation transport is not available");
  }

  default byte[] buildDeviceSubscriptionInvocation(byte[] requestJSON) {
    throw EventsSupport.unsupported("events device subscription invocation transport is not available");
  }

  default byte[] buildSessionSubscriptionInvocation(byte[] requestJSON) {
    throw EventsSupport.unsupported("events session subscription invocation transport is not available");
  }

  default byte[] buildInvocationSubscriptionInvocation(byte[] requestJSON) {
    throw EventsSupport.unsupported("events invocation subscription invocation transport is not available");
  }

  default StreamSource subscribeDirectory(byte[] requestJSON) {
    throw EventsSupport.unsupported("events subscribe directory transport is not available");
  }

  default StreamSource subscribeDevices(byte[] requestJSON) {
    throw EventsSupport.unsupported("events subscribe devices transport is not available");
  }

  default StreamSource subscribeSessions(byte[] requestJSON) {
    throw EventsSupport.unsupported("events subscribe sessions transport is not available");
  }

  default StreamSource subscribeInvocations(byte[] requestJSON) {
    throw EventsSupport.unsupported("events subscribe invocations transport is not available");
  }

  default byte[] listDeviceEvents(byte[] requestJSON) {
    throw EventsSupport.unsupported("events list device events transport is not available");
  }

  default byte[] projectDirectoryEvent(byte[] eventJSON) {
    throw EventsSupport.unsupported("events project directory event transport is not available");
  }

  default byte[] projectLiveEvent(byte[] eventJSON) {
    throw EventsSupport.unsupported("events project live event transport is not available");
  }

  default byte[] projectDropReport(byte[] dropJSON) {
    throw EventsSupport.unsupported("events project drop report transport is not available");
  }

  default byte[] projectTerminal(byte[] terminalJSON) {
    throw EventsSupport.unsupported("events project terminal transport is not available");
  }

  @Override
  default void close() {}
}
