package run.easynet.daemon;

public interface HostStreamLifecycleProvider {
  HostStreamReadiness checkReadiness(HostStreamBinding binding);

  HostStreamCleanup cleanup(HostStreamBinding binding);
}
