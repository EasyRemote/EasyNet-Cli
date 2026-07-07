package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class DirectoryClient implements AutoCloseable {
  public static final int DEFAULT_DIRECTORY_PAGE_SIZE = 50;
  public static final int MAX_DIRECTORY_PAGE_SIZE = 500;

  private final DirectoryTransport transport;
  private boolean closed;

  public DirectoryClient(DirectoryTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public DirectoryResolvedRef resolve(DirectoryResolveRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return DirectoryResolvedRef.fromJSON(transport.resolve(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("directory resolve transport failed", error);
    }
  }

  public DirectoryResolvedRef resolve(ResolveQuery query) {
    requireOpen();
    Objects.requireNonNull(query, "query");
    try {
      return DirectoryResolvedRef.fromJSON(transport.resolve(query.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("directory resolve transport failed", error);
    }
  }

  public Map<String, Object> buildResolveInvocation(ResolveQuery query) {
    return buildCarrier(query.toJSON(), transport::buildResolveInvocation, "directory resolve carrier failed");
  }

  public Map<String, Object> buildDirectorySubscriptionInvocation(
      DirectorySubscriptionRequest request) {
    Objects.requireNonNull(request, "request");
    return buildCarrier(
        request.toJSON(),
        transport::buildDirectorySubscriptionInvocation,
        "directory subscription carrier failed");
  }

  public StreamHandle subscribeDirectory(DirectorySubscriptionRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return new StreamHandle(transport.subscribeDirectory(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("directory subscribe transport failed", error);
    }
  }

  public DirectorySubscription projectSubscription(byte[] subscriptionJSON) {
    requireOpen();
    Objects.requireNonNull(subscriptionJSON, "subscriptionJSON");
    try {
      return DirectorySubscription.fromJSON(transport.projectSubscription(subscriptionJSON));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("directory project subscription transport failed", error);
    }
  }

  public DirectoryPage listDevices(DirectoryListRequest request) {
    return listPage(request, transport::listDevices, "directory list devices transport failed");
  }

  public DirectoryPage listDevices(DirectoryQueryBase query) {
    return listPage(query.toJSON(), transport::listDevices, "directory list devices transport failed");
  }

  public Map<String, Object> buildListDevicesInvocation(DirectoryQueryBase query) {
    return buildCarrier(
        query.toJSON(), transport::buildListDevicesInvocation, "directory list devices carrier failed");
  }

  public DirectoryPage listAgents(DirectoryListRequest request) {
    return listPage(request, transport::listAgents, "directory list agents transport failed");
  }

  public DirectoryPage listAgents(DirectoryQueryBase query) {
    return listPage(query.toJSON(), transport::listAgents, "directory list agents transport failed");
  }

  public Map<String, Object> buildListAgentsInvocation(DirectoryQueryBase query) {
    return buildCarrier(
        query.toJSON(), transport::buildListAgentsInvocation, "directory list agents carrier failed");
  }

  public DirectoryPage listAbilities(DirectoryListRequest request) {
    return listPage(request, transport::listAbilities, "directory list abilities transport failed");
  }

  public DirectoryPage listAbilities(AbilityQuery query) {
    return listPage(query.toJSON(), transport::listAbilities, "directory list abilities transport failed");
  }

  public Map<String, Object> buildListAbilitiesInvocation(AbilityQuery query) {
    return buildCarrier(
        query.toJSON(),
        transport::buildListAbilitiesInvocation,
        "directory list abilities carrier failed");
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private DirectoryPage listPage(
      DirectoryListRequest request, DirectoryListOperation operation, String failureMessage) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    return listPage(request.toJSON(), operation, failureMessage);
  }

  private DirectoryPage listPage(
      byte[] requestJSON, DirectoryListOperation operation, String failureMessage) {
    requireOpen();
    try {
      return DirectoryPage.fromJSON(operation.call(requestJSON));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure(failureMessage, error);
    }
  }

  private Map<String, Object> buildCarrier(
      byte[] requestJSON, DirectoryListOperation operation, String failureMessage) {
    requireOpen();
    try {
      return JsonValueReader.object(operation.call(requestJSON), "directory invocation carrier JSON");
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure(failureMessage, error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("directory");
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
        Map.of(),
        cause);
  }

  @FunctionalInterface
  private interface DirectoryListOperation {
    byte[] call(byte[] requestJSON);
  }
}
