package run.easynet.daemon;

import java.util.Objects;

public final class CompanionClient implements AutoCloseable {
  private final CompanionTransport transport;
  private boolean closed;

  public CompanionClient(CompanionTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public DesktopCompanionList list() {
    requireOpen();
    try {
      return DesktopCompanionList.fromJSON(transport.companionList());
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw CompanionSupport.transport("desktop companion list failed", error);
    }
  }

  public DesktopCompanionStatus status(String packageID) {
    return status(packageID, "");
  }

  public DesktopCompanionStatus status(String packageID, String packageVersion) {
    requireOpen();
    var input = input(packageID, packageVersion);
    try {
      return DesktopCompanionStatus.fromJSON(transport.companionStatus(input.packageID, input.packageVersion));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw CompanionSupport.transport("desktop companion status failed", error);
    }
  }

  public DesktopCompanionActionResult enable(String packageID) {
    return enable(packageID, "");
  }

  public DesktopCompanionActionResult enable(String packageID, String packageVersion) {
    return action("enable", packageID, packageVersion);
  }

  public DesktopCompanionActionResult disable(String packageID) {
    return disable(packageID, "");
  }

  public DesktopCompanionActionResult disable(String packageID, String packageVersion) {
    return action("disable", packageID, packageVersion);
  }

  public DesktopCompanionActionResult start(String packageID) {
    return start(packageID, "");
  }

  public DesktopCompanionActionResult start(String packageID, String packageVersion) {
    return action("start", packageID, packageVersion);
  }

  public DesktopCompanionActionResult stop(String packageID) {
    return stop(packageID, "");
  }

  public DesktopCompanionActionResult stop(String packageID, String packageVersion) {
    return action("stop", packageID, packageVersion);
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private DesktopCompanionActionResult action(String action, String packageID, String packageVersion) {
    requireOpen();
    var input = input(packageID, packageVersion);
    try {
      byte[] raw =
          switch (action) {
            case "enable" -> transport.companionEnable(input.packageID, input.packageVersion);
            case "disable" -> transport.companionDisable(input.packageID, input.packageVersion);
            case "start" -> transport.companionStart(input.packageID, input.packageVersion);
            case "stop" -> transport.companionStop(input.packageID, input.packageVersion);
            default -> throw CompanionSupport.invalid("unsupported desktop companion action");
          };
      return DesktopCompanionActionResult.fromJSON(raw);
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw CompanionSupport.transport("desktop companion " + action + " failed", error);
    }
  }

  private Input input(String packageID, String packageVersion) {
    String cleanedID = packageID == null ? "" : packageID.trim();
    if (cleanedID.isEmpty()) {
      throw CompanionSupport.invalid("package_id is required");
    }
    return new Input(cleanedID, packageVersion == null ? "" : packageVersion.trim());
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("desktop_companion");
    }
  }

  private record Input(String packageID, String packageVersion) {}
}
