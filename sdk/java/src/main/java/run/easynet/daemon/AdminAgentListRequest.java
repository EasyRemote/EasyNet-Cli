package run.easynet.daemon;

public record AdminAgentListRequest(AdminCarrierBase carrier) {
  public AdminAgentListRequest {
    if (carrier == null) {
      throw AdminSupport.invalid("carrier is required");
    }
  }

  byte[] toJSON() {
    return JsonValueWriter.object(carrier.toObject());
  }
}
