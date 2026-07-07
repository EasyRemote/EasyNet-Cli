package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record DirectorySubscription(
    String profile,
    String kind,
    String stream,
    String state,
    DirectorySubscriptionCursor cursor,
    String resumeToken,
    int dropCount,
    List<Event> events,
    Map<String, Object> metadata) {
  public static final int MAX_BUFFERED_EVENTS = 1024;

  public DirectorySubscription {
    profile = DirectoryIdentitySupport.cleanRequired(profile, "profile");
    kind = DirectoryIdentitySupport.cleanRequired(kind, "kind");
    stream = DirectoryIdentitySupport.cleanRequired(stream, "stream");
    if (!profile.equals("directory_identity")
        || !kind.equals("directory_subscription")
        || !stream.equals("directory")) {
      throw DirectoryIdentitySupport.invalidField("directory_subscription", "projection mismatch");
    }
    state = DirectoryIdentitySupport.cleanRequired(state, "state");
    if (!List.of("Opening", "CatchingUp", "Live", "Resuming", "Closed", "Failed")
        .contains(state)) {
      throw DirectoryIdentitySupport.invalidField("state", "is not supported");
    }
    if (cursor == null) {
      throw DirectoryIdentitySupport.invalidField("cursor", "must be an object");
    }
    resumeToken = DirectoryIdentitySupport.cleanRequired(resumeToken, "resume_token");
    if (!resumeToken.equals(cursor.resumeToken())) {
      throw DirectoryIdentitySupport.invalidField("resume_token", "must match cursor");
    }
    if (dropCount < 0) {
      throw DirectoryIdentitySupport.invalidField("drop_count", "must be non-negative");
    }
    events = events == null ? List.of() : List.copyOf(events);
    if (events.size() > MAX_BUFFERED_EVENTS) {
      throw DirectoryIdentitySupport.invalidField("events", "exceeds bounded capacity");
    }
    metadata = DirectoryIdentitySupport.copyObject(metadata);
  }

  public static DirectorySubscription fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "directory subscription JSON");
    List<Event> events = new ArrayList<>();
    for (Object event : DirectoryIdentitySupport.requiredList(fields, "events")) {
      events.add(Event.fromObject(DirectoryIdentitySupport.optionalObject(event, "events")));
    }
    return new DirectorySubscription(
        DirectoryIdentitySupport.requiredString(fields, "profile"),
        DirectoryIdentitySupport.requiredString(fields, "kind"),
        DirectoryIdentitySupport.requiredString(fields, "stream"),
        DirectoryIdentitySupport.requiredString(fields, "state"),
        DirectorySubscriptionCursor.fromObject(
            DirectoryIdentitySupport.requiredObject(fields, "cursor")),
        DirectoryIdentitySupport.requiredString(fields, "resume_token"),
        DirectoryIdentitySupport.requiredInteger(fields, "drop_count"),
        events,
        DirectoryIdentitySupport.requiredObject(fields, "metadata"));
  }

  public record Event(
      String profile,
      String stream,
      String kind,
      String eventID,
      String phase,
      String itemKind,
      Map<String, Object> item,
      DirectorySubscriptionCursor cursor,
      String resumeToken,
      boolean terminal,
      Map<String, Object> metadata) {
    public Event {
      profile = DirectoryIdentitySupport.cleanRequired(profile, "profile");
      stream = DirectoryIdentitySupport.cleanRequired(stream, "stream");
      kind = DirectoryIdentitySupport.cleanRequired(kind, "kind");
      eventID = DirectoryIdentitySupport.cleanRequired(eventID, "event_id");
      phase = DirectoryIdentitySupport.cleanRequired(phase, "phase");
      if (!profile.equals("directory_identity") || !stream.equals("directory")) {
        throw DirectoryIdentitySupport.invalidField("directory_event", "projection mismatch");
      }
      itemKind = DirectoryIdentitySupport.optionalClean(itemKind, "item_kind");
      item = DirectoryIdentitySupport.copyObject(item);
      if (cursor == null) {
        throw DirectoryIdentitySupport.invalidField("cursor", "must be an object");
      }
      resumeToken = DirectoryIdentitySupport.cleanRequired(resumeToken, "resume_token");
      if (!resumeToken.equals(cursor.resumeToken())) {
        throw DirectoryIdentitySupport.invalidField("resume_token", "must match cursor");
      }
      metadata = DirectoryIdentitySupport.copyObject(metadata);
    }

    static Event fromObject(Map<String, Object> fields) {
      return new Event(
          DirectoryIdentitySupport.requiredString(fields, "profile"),
          DirectoryIdentitySupport.requiredString(fields, "stream"),
          DirectoryIdentitySupport.requiredString(fields, "kind"),
          DirectoryIdentitySupport.requiredString(fields, "event_id"),
          DirectoryIdentitySupport.requiredString(fields, "phase"),
          DirectoryIdentitySupport.optionalString(fields.get("item_kind"), "item_kind"),
          DirectoryIdentitySupport.optionalObject(fields.get("item"), "item"),
          DirectorySubscriptionCursor.fromObject(
              DirectoryIdentitySupport.requiredObject(fields, "cursor")),
          DirectoryIdentitySupport.requiredString(fields, "resume_token"),
          DirectoryIdentitySupport.requiredBoolean(fields, "terminal"),
          DirectoryIdentitySupport.requiredObject(fields, "metadata"));
    }
  }
}
