package run.easynet.daemon;

public final class AuthorityClient implements AutoCloseable {
  private final AuthorityTransport transport;
  private boolean closed;

  public AuthorityClient(AuthorityTransport transport) {
    if (transport == null) {
      throw AuthoritySupport.invalid("authority transport is required");
    }
    this.transport = transport;
  }

  public DelegationProof mintDelegationProof(DelegationRequest request) {
    requireOpen();
    byte[] raw = transport.mintDelegationProof(request.toJSON());
    return DelegationProof.fromMetadata(
        AuthoritySupport.decodeAuthorityMetadataProjection(
            raw, AuthoritySupport.DELEGATION_METADATA_KEY, "delegation"));
  }

  public SessionAuthority mintSessionAuthority(SessionAuthorityRequest request) {
    requireOpen();
    byte[] raw = transport.mintSessionAuthority(request.toJSON());
    return SessionAuthority.fromMetadata(
        AuthoritySupport.decodeAuthorityMetadataProjection(
            raw, AuthoritySupport.SESSION_AUTHORITY_METADATA_KEY, "session authority"));
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("authority");
    }
  }
}
