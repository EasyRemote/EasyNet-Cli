package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record SurfacePagePage(
    String profile,
    String kind,
    String itemKind,
    List<SurfacePageRecord> items,
    String nextCursor,
    int limit,
    String source,
    Map<String, Object> metadata) {
  public SurfacePagePage {
    if (!SurfaceSupport.PROFILE.equals(profile)
        || !"surface_page_page".equals(kind)
        || !"page_record".equals(itemKind)
        || !"pages_read_model".equals(source)) {
      throw SurfaceSupport.invalid("invalid surface page projection");
    }
    if (limit < 1 || limit > SurfaceSupport.MAX_PAGE_SIZE) {
      throw SurfaceSupport.invalid("surface page limit exceeds bounds");
    }
    items = items == null ? List.of() : List.copyOf(items);
    nextCursor = SurfaceSupport.optionalClean(nextCursor, "next_cursor");
    metadata = SurfaceSupport.copyObject(metadata);
  }

  public static SurfacePagePage fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "surface page page JSON");
    List<SurfacePageRecord> records = new ArrayList<>();
    for (Object item : SurfaceSupport.requiredList(fields, "items")) {
      Map<String, Object> itemObject = SurfaceSupport.optionalObject(item, "items");
      if (itemObject == null) {
        throw SurfaceSupport.invalid("items entry must be an object");
      }
      records.add(SurfacePageRecord.fromObject(itemObject));
    }
    return new SurfacePagePage(
        SurfaceSupport.requiredString(fields, "profile"),
        SurfaceSupport.requiredString(fields, "kind"),
        SurfaceSupport.requiredString(fields, "item_kind"),
        records,
        SurfaceSupport.optionalString(fields.get("next_cursor"), "next_cursor"),
        SurfaceSupport.requiredInteger(fields, "limit"),
        SurfaceSupport.requiredString(fields, "source"),
        SurfaceSupport.requiredObject(fields, "metadata"));
  }
}
