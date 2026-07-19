package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record SessionAuthorityRequest(
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
    Map<String, Object> metadata) {
  public SessionAuthorityRequest {
    issuerURA = AuthoritySupport.requiredURA(issuerURA, "issuer_ura");
    sessionID = AuthoritySupport.requiredString(sessionID, "session_id");
    sessionOwnerUserID = AuthoritySupport.requiredPrincipalID(sessionOwnerUserID, "session_owner_user_id");
    creatorPrincipalID = AuthoritySupport.requiredString(creatorPrincipalID, "creator_principal_id");
    calleeURA = AuthoritySupport.requiredURA(calleeURA, "callee_ura");
    subjectURA = AuthoritySupport.requiredURA(subjectURA, "subject_ura");
    audience = AuthoritySupport.requiredURA(audience, "audience");
    scopes = AuthoritySupport.requiredScopes(scopes);
    allowedActions = AuthoritySupport.requiredScopes(allowedActions);
    allowedFollowupAbilities = AuthoritySupport.requiredScopes(allowedFollowupAbilities);
    if (expiresAtMS <= issuedAtMS) {
      throw AuthoritySupport.invalid("session authority request expires_at_ms must be greater than issued_at_ms");
    }
    metadata = AuthoritySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("issuer_ura", issuerURA);
    out.put("session_id", sessionID);
    out.put("session_owner_user_id", sessionOwnerUserID);
    out.put("creator_principal_id", creatorPrincipalID);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("audience", audience);
    out.put("scopes", scopes);
    out.put("allowed_actions", allowedActions);
    out.put("allowed_followup_abilities", allowedFollowupAbilities);
    out.put("issued_at_ms", issuedAtMS);
    out.put("expires_at_ms", expiresAtMS);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
