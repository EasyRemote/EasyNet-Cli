package run.easynet.daemon;

public interface AuthorityTransport extends AutoCloseable {
  byte[] mintDelegationProof(byte[] requestJSON);

  byte[] mintSessionAuthority(byte[] requestJSON);

  @Override
  default void close() {}
}
