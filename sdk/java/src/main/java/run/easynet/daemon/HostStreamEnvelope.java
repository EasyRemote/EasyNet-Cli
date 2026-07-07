package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record HostStreamEnvelope(HostStreamEnvelopeRequest request) {
  public HostStreamEnvelope {
    if (request == null) {
      throw HostBindingSupport.invalid("request is required");
    }
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(Map.of("request", request.toObject()));
  }

  public record HostStreamEnvelopeRequest(
      String function, Object args, String callID, String caller) {
    public HostStreamEnvelopeRequest {
      function = HostBindingSupport.required(function, "fn");
      callID = HostBindingSupport.required(callID, "call_id");
      caller = HostBindingSupport.required(caller, "caller");
    }

    Map<String, Object> toObject() {
      LinkedHashMap<String, Object> object = new LinkedHashMap<>();
      object.put("fn", function);
      object.put("args", args);
      object.put("call_id", callID);
      object.put("caller", caller);
      return object;
    }
  }
}
