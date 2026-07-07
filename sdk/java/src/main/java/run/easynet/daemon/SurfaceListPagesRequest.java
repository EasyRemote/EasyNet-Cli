package run.easynet.daemon;

import java.util.LinkedHashMap;

public record SurfaceListPagesRequest(SurfaceCarrierBase base, Integer limit, String cursor) {
  public SurfaceListPagesRequest {
    if (base == null) {
      throw SurfaceSupport.invalid("surface carrier base is required");
    }
    if (limit != null && (limit < 1 || limit > SurfaceSupport.MAX_PAGE_SIZE)) {
      throw SurfaceSupport.invalid("surface page limit exceeds bounds");
    }
    cursor = SurfaceSupport.optionalClean(cursor, "cursor");
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = base.toObject();
    if (limit != null) {
      out.put("limit", limit);
    }
    SurfaceSupport.putOptional(out, "cursor", cursor);
    return JsonValueWriter.object(out);
  }
}
