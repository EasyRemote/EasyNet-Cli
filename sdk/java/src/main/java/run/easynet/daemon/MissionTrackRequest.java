package run.easynet.daemon;

import java.util.LinkedHashMap;

public record MissionTrackRequest(MissionCarrierBase carrier, String missionID) {
  public MissionTrackRequest {
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
