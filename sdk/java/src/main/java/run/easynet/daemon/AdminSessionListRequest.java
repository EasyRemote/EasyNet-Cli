package run.easynet.daemon;

import java.util.LinkedHashMap;

public record AdminSessionListRequest(AdminCarrierBase carrier, Boolean includeTerminated) {
  public AdminSessionListRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
  }

  byte[] toJSON() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>(carrier.toObject());
    AdminSupport.putOptional(out, "include_terminated", includeTerminated);
    return JsonValueWriter.object(out);
  }
}
