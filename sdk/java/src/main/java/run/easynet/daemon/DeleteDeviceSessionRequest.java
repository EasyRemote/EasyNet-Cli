package run.easynet.daemon;

import java.util.LinkedHashMap;

public record DeleteDeviceSessionRequest(AdminCarrierBase carrier, String sessionID, String reason) {
  public DeleteDeviceSessionRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
    sessionID = AdminSupport.identifier(sessionID, "session_id");
    reason = AdminSupport.optionalReason(reason);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("session_id", sessionID);
    AdminSupport.putOptional(out, "reason", reason);
    return JsonValueWriter.object(out);
  }
}
