package run.runtime.sdk;

import java.util.List;
import java.util.Map;

public record SessionAuthority(
    String issuerURA,
    String sessionID,
    String sessionOwnerUserID,
    String creatorPrincipalID,
    String calleeURA,
    String subjectURA,
    String audience,
    List<String> scopes,
    List<String> allowedActions,
    List<String> allowedFollowupAbilities,
    long issuedAtMS,
    long expiresAtMS,
    String signatureBase64,
    String metadataValue) {
  public SessionAuthority {
    issuerURA = AuthoritySupport.requiredURA(issuerURA, "issuer_ura");
    sessionID = AuthoritySupport.requiredString(sessionID, "session_id");
    sessionOwnerUserID = AuthoritySupport.requiredPrincipalID(sessionOwnerUserID, "session_owner_user_id");
    creatorPrincipalID = AuthoritySupport.requiredPrincipalID(creatorPrincipalID, "creator_principal_id");
    calleeURA = AuthoritySupport.requiredURA(calleeURA, "callee_ura");
    subjectURA = AuthoritySupport.requiredURA(subjectURA, "subject_ura");
    audience = AuthoritySupport.requiredURA(audience, "audience");
    scopes = AuthoritySupport.requiredScopes(scopes);
    allowedActions = AuthoritySupport.requiredScopes(allowedActions);
    allowedFollowupAbilities = AuthoritySupport.requiredScopes(allowedFollowupAbilities);
    if (expiresAtMS <= issuedAtMS) {
      throw AuthoritySupport.invalid("session authority expires_at_ms must be greater than issued_at_ms");
    }
    AuthoritySupport.validateSessionAuthoritySubjectBinding(subjectURA, sessionOwnerUserID, sessionID);
    signatureBase64 = AuthoritySupport.requiredBase64(signatureBase64, "signature_base64");
    metadataValue = AuthoritySupport.requiredString(metadataValue, "metadata_value");
  }

  public static SessionAuthority fromMetadata(String value) {
    AuthoritySupport.DecodedAuthority decoded = AuthoritySupport.decodeAuthorityMetadata(value, "session authority");
    Map<String, Object> payload = decoded.payload();
    return new SessionAuthority(
        AuthoritySupport.requiredString(payload.get("issuer_ura"), "issuer_ura"),
        AuthoritySupport.requiredString(payload.get("session_id"), "session_id"),
        AuthoritySupport.requiredPrincipalID(
            AuthoritySupport.requiredString(payload.get("session_owner_user_id"), "session_owner_user_id"),
            "session_owner_user_id"),
        AuthoritySupport.requiredPrincipalID(
            AuthoritySupport.requiredString(payload.get("creator_principal_id"), "creator_principal_id"),
            "creator_principal_id"),
        AuthoritySupport.requiredString(payload.get("callee_ura"), "callee_ura"),
        AuthoritySupport.requiredString(payload.get("subject_ura"), "subject_ura"),
        AuthoritySupport.requiredString(payload.get("audience"), "audience"),
        AuthoritySupport.requiredStringList(payload.get("scopes"), "scopes"),
        AuthoritySupport.requiredStringList(payload.get("allowed_actions"), "allowed_actions"),
        AuthoritySupport.requiredStringList(payload.get("allowed_followup_abilities"), "allowed_followup_abilities"),
        AuthoritySupport.requiredLong(payload.get("issued_at_ms"), "issued_at_ms"),
        AuthoritySupport.requiredLong(payload.get("expires_at_ms"), "expires_at_ms"),
        decoded.signatureBase64(),
        value);
  }

  public AuthorityMetadata metadata() {
    return new AuthorityMetadata(
        AuthoritySupport.SESSION_AUTHORITY_KIND,
        AuthoritySupport.SESSION_AUTHORITY_METADATA_KEY,
        metadataValue);
  }
}
