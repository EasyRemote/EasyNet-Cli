package run.runtime.sdk.provider.runtime.pluginexec;

/** Implements one declarative exec sidecar invocation. */
@FunctionalInterface
public interface SidecarHandler {
  Object handle(SidecarInvocation invocation) throws Exception;
}
