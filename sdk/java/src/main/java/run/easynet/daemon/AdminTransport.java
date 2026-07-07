package run.easynet.daemon;

public interface AdminTransport extends AutoCloseable {
  default byte[] buildAgentListInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-list invocation transport is required");
  }

  default byte[] buildAgentStartInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-start invocation transport is required");
  }

  default byte[] buildAgentStopInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-stop invocation transport is required");
  }

  default byte[] buildAgentRefreshInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-refresh invocation transport is required");
  }

  default byte[] buildSessionListInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin session-list invocation transport is required");
  }

  default byte[] buildRevokeDeviceInvocation(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin revoke-device invocation transport is required");
  }

  default byte[] gatewayStatus(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin gateway-status transport is required");
  }

  default byte[] listAgents(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin list-agents transport is required");
  }

  default byte[] agentStart(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-start transport is required");
  }

  default byte[] agentStop(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-stop transport is required");
  }

  default byte[] agentRefresh(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin agent-refresh transport is required");
  }

  default byte[] listDeviceSessions(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin list-device-sessions transport is required");
  }

  default byte[] joinHub(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin join-hub transport is required");
  }

  default byte[] leaveHub(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin leave-hub transport is required");
  }

  default byte[] pairingPreflight(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin pairing-preflight transport is required");
  }

  default byte[] validatePairing(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin validate-pairing transport is required");
  }

  default byte[] verifyDeviceCredential(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin verify-device-credential transport is required");
  }

  default byte[] createPairing(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin create-pairing transport is required");
  }

  default byte[] revokeDevice(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin revoke-device transport is required");
  }

  default byte[] createDeviceSession(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin create-device-session transport is required");
  }

  default byte[] deleteDeviceSession(byte[] requestJSON) {
    throw AdminSupport.unsupported("admin delete-device-session transport is required");
  }

  @Override
  default void close() {}
}
