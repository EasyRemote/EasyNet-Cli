package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record DeviceEventPage(
    String profile,
    String stream,
    String itemKind,
    List<EventFrame> items,
    String nextCursor,
    boolean hasMore,
    int limit,
    Map<String, Object> metadata) {
  public DeviceEventPage {
    profile = EventsSupport.cleanRequired(profile, "profile");
    stream = EventsSupport.requiredStream(stream, "stream");
    itemKind = EventsSupport.cleanRequired(itemKind, "item_kind");
    if (!profile.equals(EventsSupport.PROFILE) || !stream.equals("device")) {
      throw EventsSupport.invalid("invalid device event page projection");
    }
    items = items == null ? List.of() : List.copyOf(items);
    for (EventFrame item : items) {
      if (!item.stream().equals("device")) {
        throw EventsSupport.invalid("device event page item stream mismatch");
      }
    }
    nextCursor = EventsSupport.optionalClean(nextCursor, "next_cursor");
    if (limit < 1 || limit > EventsSupport.MAX_PAGE_SIZE) {
      throw EventsSupport.invalid("limit exceeds bounds");
    }
    metadata = EventsSupport.copyObject(metadata);
  }

  public static DeviceEventPage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "device event page JSON");
    List<EventFrame> events = new ArrayList<>();
    for (Object item : EventsSupport.requiredList(fields, "items")) {
      events.add(EventFrame.fromObject(EventsSupport.optionalObject(item, "items")));
    }
    return new DeviceEventPage(
        EventsSupport.requiredString(fields, "profile"),
        EventsSupport.requiredString(fields, "stream"),
        EventsSupport.requiredString(fields, "item_kind"),
        events,
        EventsSupport.optionalString(fields.get("next_cursor"), "next_cursor"),
        EventsSupport.requiredBoolean(fields, "has_more"),
        EventsSupport.requiredInteger(fields, "limit"),
        EventsSupport.requiredObject(fields, "metadata"));
  }
}
