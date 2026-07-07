package run.easynet.daemon;

import java.util.LinkedHashMap;

public record MissionEventsRequest(MissionCarrierBase carrier, String missionID, long cursorSequence, int limit) {
  public MissionEventsRequest {
    if (carrier == null) {
      throw MissionSupport.invalid("carrier is required");
    }
    missionID = MissionSupport.missionID(missionID);
    if (cursorSequence < 0) {
      throw MissionSupport.invalid("cursor_sequence must be non-negative");
    }
    if (limit < 0 || limit > MissionSupport.MAX_EVENTS_LIMIT) {
      throw MissionSupport.invalid("limit must be between 0 and " + MissionSupport.MAX_EVENTS_LIMIT);
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("mission_id", missionID);
    out.put("cursor_sequence", cursorSequence);
    if (limit > 0) {
      out.put("limit", limit);
    }
    return JsonValueWriter.object(out);
  }
}
