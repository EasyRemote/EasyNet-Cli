package run.easynet.daemon;

import java.util.Map;

public record HostStreamRequest(
    String function,
    Object args,
    String callID,
    String caller,
    Map<String, Object> metadata) {
  public HostStreamRequest {
    function = HostBindingSupport.required(function, "function");
    callID = HostBindingSupport.required(callID, "call_id");
    caller = HostBindingSupport.required(caller, "caller");
    metadata = HostBindingSupport.copyObject(metadata);
  }

  public static HostStreamRequest fromJSON(byte[] raw) {
    var fields = JsonValueReader.object(raw, "host stream request JSON");
    return new HostStreamRequest(
        HostBindingSupport.requiredString(fields, "function"),
        fields.get("args"),
        HostBindingSupport.requiredString(fields, "call_id"),
        HostBindingSupport.requiredString(fields, "caller"),
        HostBindingSupport.requiredObject(fields, "metadata"));
  }
}
