package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DirectoryQueryBase(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    Integer limit,
    String cursor,
    Map<String, Object> metadata) {
  public static final int DEFAULT_PAGE_SIZE = DirectoryClient.DEFAULT_DIRECTORY_PAGE_SIZE;
  public static final int MAX_PAGE_SIZE = DirectoryClient.MAX_DIRECTORY_PAGE_SIZE;

  public DirectoryQueryBase {
    callerURA = DirectoryIdentitySupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = DirectoryIdentitySupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = DirectoryIdentitySupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion =
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = DirectoryIdentitySupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = DirectoryIdentitySupport.requiredCopiedObject(causalContext, "causal_context");
    limit = DirectoryIdentitySupport.normalizeLimit(limit);
    cursor = DirectoryIdentitySupport.optionalClean(cursor, "cursor");
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  byte[] toJSON() {
    return JsonValueWriter.object(toObject());
  }

  LinkedHashMap<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("caller_ura", callerURA);
    out.put("callee_ura", calleeURA);
    out.put("subject_ura", subjectURA);
    out.put("descriptor_version", descriptorVersion);
    out.put("nonce_base64", nonceBase64);
    out.put("causal_context", causalContext);
    out.put("limit", limit);
    if (cursor != null) {
      out.put("cursor", cursor);
    }
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return out;
  }
}
