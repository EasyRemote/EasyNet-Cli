package run.easynet.daemon;

public record CompatibilityListModelsRequest(CompatibilityCarrierBase base) {
  public CompatibilityListModelsRequest {
    if (base == null) {
      throw CompatibilitySupport.invalid("complete compatibility invocation carrier is required");
    }
  }

  public static CompatibilityListModelsRequest fromJSON(byte[] raw) {
    return new CompatibilityListModelsRequest(CompatibilityCarrierBase.fromObject(JsonValueReader.object(raw, "compatibility list models request JSON")));
  }

  byte[] toJSON() {
    return JsonValueWriter.object(base.toObject());
  }
}
