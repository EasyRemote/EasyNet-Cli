package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public record SessionAuthorityRequest(
    String issuerURA,
    String subjectURA,
    String audience,
    List<String> scopes,
    long issuedAtMS,
    long expiresAtMS,
    Map<String, Object> metadata) {
  public SessionAuthorityRequest {
    issuerURA = AuthoritySupport.requiredURA(issuerURA, "issuer_ura");
    subjectURA = AuthoritySupport.requiredURA(subjectURA, "subject_ura");
    audience = AuthoritySupport.requiredURA(audience, "audience");
    scopes = AuthoritySupport.requiredScopes(scopes);
    if (expiresAtMS <= issuedAtMS) {
      throw AuthoritySupport.invalid("session authority request expires_at_ms must be greater than issued_at_ms");
    }
    metadata = AuthoritySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("issuer_ura", issuerURA);
    out.put("subject_ura", subjectURA);
    out.put("audience", audience);
    out.put("scopes", scopes);
    out.put("issued_at_ms", issuedAtMS);
    out.put("expires_at_ms", expiresAtMS);
    out.put("metadata", metadata);
    return JsonValueWriter.object(out);
  }
}
