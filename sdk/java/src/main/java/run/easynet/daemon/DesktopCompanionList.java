package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record DesktopCompanionList(List<DesktopCompanionStatus> companions) {
  public DesktopCompanionList {
    companions = companions == null ? List.of() : List.copyOf(companions);
  }

  public static DesktopCompanionList fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "desktop companion list JSON");
    Object value = fields.get("companions");
    if (!(value instanceof List<?> items)) {
      throw CompanionSupport.invalid("companions must be an array");
    }
    ArrayList<DesktopCompanionStatus> statuses = new ArrayList<>();
    for (Object item : items) {
      if (!(item instanceof Map<?, ?> decoded)) {
        throw CompanionSupport.invalid("companions entries must be objects");
      }
      statuses.add(DesktopCompanionStatus.fromObject(companionStatusObject(decoded)));
    }
    return new DesktopCompanionList(statuses);
  }

  private static Map<String, Object> companionStatusObject(Map<?, ?> decoded) {
    return CompanionSupport.optionalObject(decoded, "companions entry");
  }
}
