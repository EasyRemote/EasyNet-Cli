package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class EventClient implements AutoCloseable {
  private final EventTransport transport;
  private boolean closed;

  public EventClient(EventTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public Map<String, Object> buildDirectorySubscriptionInvocation(EventsSubscriptionRequest request) {
    return buildSubscriptionInvocation(request, "directory", transport::buildDirectorySubscriptionInvocation, "events directory subscription invocation failed");
  }

  public Map<String, Object> buildDeviceSubscriptionInvocation(EventsSubscriptionRequest request) {
    return buildSubscriptionInvocation(request, "device", transport::buildDeviceSubscriptionInvocation, "events device subscription invocation failed");
  }

  public Map<String, Object> buildSessionSubscriptionInvocation(EventsSubscriptionRequest request) {
    return buildSubscriptionInvocation(request, "session", transport::buildSessionSubscriptionInvocation, "events session subscription invocation failed");
  }

  public Map<String, Object> buildInvocationSubscriptionInvocation(EventsSubscriptionRequest request) {
    return buildSubscriptionInvocation(request, "invocation", transport::buildInvocationSubscriptionInvocation, "events invocation subscription invocation failed");
  }

  public EventStream subscribeDirectory(EventsSubscriptionRequest request) {
    return subscribe(request, "directory", transport::subscribeDirectory, "events subscribe directory failed");
  }

  public EventStream subscribeDevices(EventsSubscriptionRequest request) {
    return subscribe(request, "device", transport::subscribeDevices, "events subscribe devices failed");
  }

  public EventStream subscribeSessions(EventsSubscriptionRequest request) {
    return subscribe(request, "session", transport::subscribeSessions, "events subscribe sessions failed");
  }

  public EventStream subscribeInvocations(EventsSubscriptionRequest request) {
    return subscribe(request, "invocation", transport::subscribeInvocations, "events subscribe invocations failed");
  }

  public DeviceEventPage listDeviceEvents(EventsDeviceEventListRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return DeviceEventPage.fromJSON(transport.listDeviceEvents(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("events list device events failed", error);
    }
  }

  public EventFrame projectDirectoryEvent(EventProjectionInput input) {
    return project(input.toJSON("directory"), transport::projectDirectoryEvent, "events project directory event failed");
  }

  public EventFrame projectLiveEvent(EventProjectionInput input) {
    return project(input.toJSON(""), transport::projectLiveEvent, "events project live event failed");
  }

  public EventFrame projectDropReport(EventDropReportInput input) {
    return project(input.toJSON(), transport::projectDropReport, "events project drop report failed");
  }

  public EventFrame projectTerminal(EventTerminalInput input) {
    return project(input.toJSON(), transport::projectTerminal, "events project terminal failed");
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private Map<String, Object> buildSubscriptionInvocation(
      EventsSubscriptionRequest request, String stream, EventBytesOperation operation, String message) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return JsonValueReader.object(operation.call(request.toJSON(stream)), "events invocation JSON");
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure(message, error);
    }
  }

  private EventStream subscribe(
      EventsSubscriptionRequest request, String stream, EventStreamOperation operation, String message) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return new EventStream(stream, operation.call(request.toJSON(stream)));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure(message, error);
    }
  }

  private EventFrame project(byte[] requestJSON, EventBytesOperation operation, String message) {
    requireOpen();
    try {
      return EventFrame.fromJSON(operation.call(requestJSON));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure(message, error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("events");
    }
  }

  private static SDKError transportFailure(String message, RuntimeException cause) {
    return new SDKError(
        ErrorCode.TRANSPORT,
        "transport",
        RetryHint.SAFE,
        true,
        message,
        "",
        "",
        "",
        Map.of("profile", EventsSupport.PROFILE),
        cause);
  }

  @FunctionalInterface
  private interface EventBytesOperation {
    byte[] call(byte[] requestJSON);
  }

  @FunctionalInterface
  private interface EventStreamOperation {
    StreamSource call(byte[] requestJSON);
  }
}
