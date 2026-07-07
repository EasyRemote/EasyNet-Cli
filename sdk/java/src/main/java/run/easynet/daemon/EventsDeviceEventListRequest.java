package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record EventsDeviceEventListRequest(
    EventsCarrierBase base,
    EventFilter filter,
    String deviceURA,
    Integer limit,
    String cursor) {
  public EventsDeviceEventListRequest {
    if (base == null) {
      throw EventsSupport.invalid("events carrier base is required");
    }
    deviceURA = EventsSupport.optionalClean(deviceURA, "device_ura");
    cursor = EventsSupport.optionalClean(cursor, "cursor");
    if (limit != null && (limit < 1 || limit > EventsSupport.MAX_PAGE_SIZE)) {
      throw EventsSupport.invalid("limit exceeds bounds");
    }
  }

  byte[] toJSON() {
    EventFilter normalized =
        EventFilter.normalize(filter, Map.of("device_ura", deviceURA == null ? "" : deviceURA));
    LinkedHashMap<String, Object> out = base.toObject();
    Map<String, Object> filterObject = normalized.toObject();
    if (filter != null && !filterObject.isEmpty()) {
      out.put("filter", filterObject);
    }
    EventsSupport.putOptional(out, "device_ura", normalized.deviceURA());
    out.put("limit", limit == null ? EventsSupport.DEFAULT_PAGE_SIZE : limit);
    EventsSupport.putOptional(out, "cursor", cursor);
    return JsonValueWriter.object(out);
  }
}
