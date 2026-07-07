package run.easynet.daemon;

public interface CompanionTransport extends AutoCloseable {
  byte[] companionList();

  byte[] companionStatus(String packageID, String packageVersion);

  byte[] companionEnable(String packageID, String packageVersion);

  byte[] companionDisable(String packageID, String packageVersion);

  byte[] companionStart(String packageID, String packageVersion);

  byte[] companionStop(String packageID, String packageVersion);

  @Override
  default void close() {}
}
