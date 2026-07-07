package run.easynet.daemon;

import java.util.LinkedHashMap;

public record MissionCancelRequest(MissionCarrierBase carrier, String missionID) {
  public MissionCancelRequest {
    if (carrier == null) {
      throw MissionSupport.invalid("carrier is required");
    }
    missionID = MissionSupport.missionID(missionID);
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    out.put("mission_id", missionID);
    return JsonValueWriter.object(out);
  }
}
