package run.easynet.daemon;

import java.util.Objects;

public final class AdminClient implements AutoCloseable {
  private final AdminTransport transport;
  private boolean closed;

  public AdminClient(AdminTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public InvocationDraft buildAgentListInvocation(AdminAgentListRequest request) {
    return invocation(request.toJSON(), transport::buildAgentListInvocation, "admin agent-list invocation failed");
  }

  public InvocationDraft buildAgentStartInvocation(AdminAgentStartRequest request) {
    return invocation(request.toJSON(), transport::buildAgentStartInvocation, "admin agent-start invocation failed");
  }

  public InvocationDraft buildAgentStopInvocation(AdminAgentStopRequest request) {
    return invocation(request.toJSON(), transport::buildAgentStopInvocation, "admin agent-stop invocation failed");
  }

  public InvocationDraft buildAgentRefreshInvocation(AdminAgentRefreshRequest request) {
    return invocation(request.toJSON(), transport::buildAgentRefreshInvocation, "admin agent-refresh invocation failed");
  }

  public InvocationDraft buildSessionListInvocation(AdminSessionListRequest request) {
    return invocation(request.toJSON(), transport::buildSessionListInvocation, "admin session-list invocation failed");
  }

  public InvocationDraft buildRevokeDeviceInvocation(RevokeDeviceRequest request) {
    return invocation(request.toJSON(), transport::buildRevokeDeviceInvocation, "admin revoke-device invocation failed");
  }

  public GatewayStatus gatewayStatus(AdminGatewayStatusRequest request) {
    return GatewayStatus.fromJSON(raw(request.toJSON(), transport::gatewayStatus, "admin gateway status failed"));
  }

  public AdminAgentPage listAgents(AdminAgentListRequest request) {
    return AdminAgentPage.fromJSON(raw(request.toJSON(), transport::listAgents, "admin list agents failed"));
  }

  public AdminGatewayResult agentStart(AdminAgentStartRequest request) {
    return result(request.toJSON(), transport::agentStart, "admin agent start failed");
  }

  public AdminGatewayResult agentStop(AdminAgentStopRequest request) {
    return result(request.toJSON(), transport::agentStop, "admin agent stop failed");
  }

  public AdminGatewayResult agentRefresh(AdminAgentRefreshRequest request) {
    return result(request.toJSON(), transport::agentRefresh, "admin agent refresh failed");
  }

  public AdminGatewayResult joinHub(AdminJoinHubRequest request) {
    return result(request.toJSON(), transport::joinHub, "admin join hub failed");
  }

  public AdminGatewayResult leaveHub(AdminLeaveHubRequest request) {
    return result(request.toJSON(), transport::leaveHub, "admin leave hub failed");
  }

  public PairingPreflight pairingPreflight(PairingPreflightRequest request) {
    return PairingPreflight.fromJSON(
        raw(request.toJSON(), transport::pairingPreflight, "admin pairing preflight failed"));
  }

  public DeviceCredential validatePairing(ValidatePairingRequest request) {
    return DeviceCredential.fromJSON(raw(request.toJSON(), transport::validatePairing, "admin validate pairing failed"));
  }

  public DeviceCredentialVerification verifyDeviceCredential(VerifyDeviceCredentialRequest request) {
    return DeviceCredentialVerification.fromJSON(
        raw(request.toJSON(), transport::verifyDeviceCredential, "admin verify device credential failed"));
  }

  public PairingToken createPairing(CreatePairingRequest request) {
    return PairingToken.fromJSON(raw(request.toJSON(), transport::createPairing, "admin create pairing failed"));
  }

  public AdminGatewayResult revokeDevice(RevokeDeviceRequest request) {
    return result(request.toJSON(), transport::revokeDevice, "admin revoke device failed");
  }

  public DeviceSession createDeviceSession(CreateDeviceSessionRequest request) {
    return DeviceSession.fromJSON(
        raw(request.toJSON(), transport::createDeviceSession, "admin create device session failed"));
  }

  public DeviceSessionPage listDeviceSessions(AdminSessionListRequest request) {
    return DeviceSessionPage.fromJSON(
        raw(request.toJSON(), transport::listDeviceSessions, "admin list device sessions failed"));
  }

  public AdminGatewayResult deleteDeviceSession(DeleteDeviceSessionRequest request) {
    return result(request.toJSON(), transport::deleteDeviceSession, "admin delete device session failed");
  }

  public GatewayStatus projectGatewayStatus(byte[] raw) {
    return GatewayStatus.fromJSON(raw);
  }

  public AdminAgentPage projectAgentRecords(byte[] raw) {
    return AdminAgentPage.fromJSON(raw);
  }

  public AdminGatewayResult projectAgentLifecycleResult(byte[] raw) {
    return AdminGatewayResult.fromJSON(raw);
  }

  public PairingPreflight projectPairingPreflight(byte[] raw) {
    return PairingPreflight.fromJSON(raw);
  }

  public PairingToken projectPairingToken(byte[] raw) {
    return PairingToken.fromJSON(raw);
  }

  public DeviceCredential projectDeviceCredential(byte[] raw) {
    return DeviceCredential.fromJSON(raw);
  }

  public DeviceSession projectDeviceSession(byte[] raw) {
    return DeviceSession.fromJSON(raw);
  }

  public DeviceSessionPage projectDeviceSessionPage(byte[] raw) {
    return DeviceSessionPage.fromJSON(raw);
  }

  public AdminGatewayResult projectDeviceAdminResult(byte[] raw) {
    return AdminGatewayResult.fromJSON(raw);
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private InvocationDraft invocation(byte[] requestJSON, AdminBytesOperation operation, String label) {
    return InvocationDraft.fromWireObject(JsonValueReader.object(raw(requestJSON, operation, label), "admin invocation JSON"));
  }

  private AdminGatewayResult result(byte[] requestJSON, AdminBytesOperation operation, String label) {
    return AdminGatewayResult.fromJSON(raw(requestJSON, operation, label));
  }

  private byte[] raw(byte[] requestJSON, AdminBytesOperation operation, String label) {
    requireOpen();
    try {
      return operation.call(requestJSON);
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw AdminSupport.transport(label, error);
    }
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed(AdminSupport.PROFILE);
    }
  }

  @FunctionalInterface
  private interface AdminBytesOperation {
    byte[] call(byte[] requestJSON);
  }
}
