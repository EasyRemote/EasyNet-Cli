package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record MissionEventPage(
    String profile,
    String kind,
    String missionID,
    long cursorSequence,
    long nextCursorSequence,
    boolean hasMore,
    long droppedCount,
    List<MissionEvent> events,
    Map<String, Object> metadata) {
  public MissionEventPage {
    if (!MissionSupport.PROFILE.equals(profile) || !"mission_event_page".equals(kind)) {
      throw MissionSupport.invalid("invalid mission event page projection");
    }
    missionID = MissionSupport.missionID(missionID);
    events = events == null ? List.of() : List.copyOf(events);
    metadata = MissionSupport.copyObject(metadata);
  }

  public static MissionEventPage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "mission event page JSON");
    List<MissionEvent> events = new ArrayList<>();
    for (Object item : MissionSupport.requiredList(fields, "events")) {
      Map<String, Object> event = MissionSupport.optionalObject(item, "events");
      if (event == null) {
        throw MissionSupport.invalid("events entry must be an object");
      }
      events.add(MissionEvent.fromObject(event));
    }
    return new MissionEventPage(
        MissionSupport.requiredString(fields, "profile"),
        MissionSupport.requiredString(fields, "kind"),
        MissionSupport.requiredString(fields, "mission_id"),
        MissionSupport.requiredLong(fields, "cursor_sequence"),
        MissionSupport.requiredLong(fields, "next_cursor_sequence"),
        MissionSupport.requiredBoolean(fields, "has_more"),
        MissionSupport.requiredLong(fields, "dropped_count"),
        events,
        MissionSupport.requiredObject(fields, "metadata"));
  }
}
