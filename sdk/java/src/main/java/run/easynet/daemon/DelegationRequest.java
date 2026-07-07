package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record DelegationRequest(
    String issuerURA,
    String subjectURA,
    String callerURA,
    String audience,
    List<String> scopes,
    long issuedAtMS,
    long expiresAtMS,
    Map<String, Object> metadata) {
  public DelegationRequest {
    issuerURA = AuthoritySupport.requiredURA(issuerURA, "issuer_ura");
    subjectURA = AuthoritySupport.requiredURA(subjectURA, "subject_ura");
    callerURA = AuthoritySupport.requiredURA(callerURA, "caller_ura");
    audience = AuthoritySupport.requiredURA(audience, "audience");
    scopes = AuthoritySupport.requiredScopes(scopes);
    if (expiresAtMS <= issuedAtMS) {
      throw AuthoritySupport.invalid("delegation request expires_at_ms must be greater than issued_at_ms");
    }
    metadata = AuthoritySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("issuer_ura", issuerURA);
    out.put("subject_ura", subjectURA);
    out.put("caller_ura", callerURA);
    out.put("audience", audience);
    out.put("scopes", scopes);
    out.put("issued_at_ms", issuedAtMS);
    out.put("expires_at_ms", expiresAtMS);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
