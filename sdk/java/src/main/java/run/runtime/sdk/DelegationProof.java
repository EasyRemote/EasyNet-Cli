package run.runtime.sdk;

import java.util.List;
import java.util.Map;

public record DelegationProof(
    String issuerURA,
    String subjectURA,
    String callerURA,
    String audience,
    List<String> scopes,
    long issuedAtMS,
    long expiresAtMS,
    String signatureBase64,
    String metadataValue) {
  public DelegationProof {
    issuerURA = AuthoritySupport.requiredURA(issuerURA, "issuer_ura");
    subjectURA = AuthoritySupport.requiredURA(subjectURA, "subject_ura");
    callerURA = AuthoritySupport.requiredURA(callerURA, "caller_ura");
    audience = AuthoritySupport.requiredURA(audience, "audience");
    scopes = AuthoritySupport.requiredScopes(scopes);
    if (expiresAtMS <= issuedAtMS) {
      throw AuthoritySupport.invalid("delegation authority expires_at_ms must be greater than issued_at_ms");
    }
    signatureBase64 = AuthoritySupport.requiredBase64(signatureBase64, "signature_base64");
    metadataValue = AuthoritySupport.requiredString(metadataValue, "metadata_value");
  }

  public static DelegationProof fromMetadata(String value) {
    AuthoritySupport.DecodedAuthority decoded =
        AuthoritySupport.decodeAuthorityMetadata(
            value, "delegation", AuthoritySupport.DELEGATION_AUTHORITY_PAYLOAD_FIELDS);
    Map<String, Object> payload = decoded.payload();
    return new DelegationProof(
        AuthoritySupport.requiredString(payload.get("issuer_ura"), "issuer_ura"),
        AuthoritySupport.requiredString(payload.get("subject_ura"), "subject_ura"),
        AuthoritySupport.requiredString(payload.get("caller_ura"), "caller_ura"),
        AuthoritySupport.requiredString(payload.get("audience"), "audience"),
        AuthoritySupport.requiredStringList(payload.get("scopes"), "scopes"),
        AuthoritySupport.requiredLong(payload.get("issued_at_ms"), "issued_at_ms"),
        AuthoritySupport.requiredLong(payload.get("expires_at_ms"), "expires_at_ms"),
        decoded.signatureBase64(),
        value);
  }

  public AuthorityMetadata metadata() {
    return new AuthorityMetadata(
        AuthoritySupport.DELEGATION_KIND, AuthoritySupport.DELEGATION_METADATA_KEY, metadataValue);
  }
}
