package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DirectorySubscriptionRequest(
    String callerURA,
    String calleeURA,
    String subjectURA,
    String descriptorVersion,
    String nonceBase64,
    Map<String, Object> causalContext,
    String stream,
    String realm,
    String ownerURA,
    String deviceURA,
    String agentURA,
    String abilityURA,
    String itemKind,
    DirectorySubscriptionCursor resumeCursor,
    Integer heartbeatIntervalMS,
    Map<String, Object> metadata) {
  public DirectorySubscriptionRequest {
    callerURA = DirectoryIdentitySupport.cleanRequired(callerURA, "caller_ura");
    calleeURA = DirectoryIdentitySupport.cleanRequired(calleeURA, "callee_ura");
    subjectURA = DirectoryIdentitySupport.cleanRequired(subjectURA, "subject_ura");
    descriptorVersion =
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version");
    nonceBase64 = DirectoryIdentitySupport.cleanRequired(nonceBase64, "nonce_base64");
    causalContext = DirectoryIdentitySupport.requiredCopiedObject(causalContext, "causal_context");
    stream = stream == null ? "directory" : DirectoryIdentitySupport.cleanRequired(stream, "stream");
    if (!stream.equals("directory")) {
      throw DirectoryIdentitySupport.invalidField("stream", "must be directory");
    }
    realm = DirectoryIdentitySupport.optionalClean(realm, "realm");
    ownerURA = DirectoryIdentitySupport.optionalClean(ownerURA, "owner_ura");
    deviceURA = DirectoryIdentitySupport.optionalClean(deviceURA, "device_ura");
    agentURA = DirectoryIdentitySupport.optionalClean(agentURA, "agent_ura");
    abilityURA = DirectoryIdentitySupport.optionalClean(abilityURA, "ability_ura");
    itemKind = DirectoryIdentitySupport.optionalClean(itemKind, "item_kind");
    if (heartbeatIntervalMS != null && heartbeatIntervalMS < 0) {
      throw DirectoryIdentitySupport.invalidField(
          "heartbeat_interval_ms", "must be non-negative");
    }
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
    out.put("stream", stream);
    putOptional(out, "realm", realm);
    putOptional(out, "owner_ura", ownerURA);
    putOptional(out, "device_ura", deviceURA);
    putOptional(out, "agent_ura", agentURA);
    putOptional(out, "ability_ura", abilityURA);
    putOptional(out, "item_kind", itemKind);
    if (resumeCursor != null) {
      out.put("resume_cursor", resumeCursor.toObject());
    }
    if (heartbeatIntervalMS != null) {
      out.put("heartbeat_interval_ms", heartbeatIntervalMS);
    }
    if (!metadata.isEmpty()) {
      out.put("metadata", metadata);
    }
    return out;
  }

  private static void putOptional(Map<String, Object> out, String name, String value) {
    if (value != null) {
      out.put(name, value);
    }
  }
}
