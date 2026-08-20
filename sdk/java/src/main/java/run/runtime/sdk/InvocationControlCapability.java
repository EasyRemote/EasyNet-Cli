package run.runtime.sdk;

public final class InvocationControlCapability {
  private final long handleId;
  private final boolean runtimeBound;

  private InvocationControlCapability(long handleId, boolean runtimeBound) {
    if (handleId <= 0) {
      throw SDKError.validation("invocation_control", "control capability is required");
    }
    this.handleId = handleId;
    this.runtimeBound = runtimeBound;
  }

  static InvocationControlCapability fromHandleId(long handleId) {
    return runtimeBound(handleId);
  }

  static InvocationControlCapability runtimeBound(long handleId) {
    return new InvocationControlCapability(handleId, true);
  }

  static InvocationControlCapability snapshot(long handleId) {
    return new InvocationControlCapability(handleId, false);
  }

  long adapterHandleId() {
    if (!runtimeBound) {
      throw SDKError.validation(
          "invocation_control", "runtime-bound invocation control capability is required");
    }
    return handleId;
  }

  long rawHandleId() {
    return handleId;
  }
}
