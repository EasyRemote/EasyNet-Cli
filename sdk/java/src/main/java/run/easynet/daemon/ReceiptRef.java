package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

public record ReceiptRef(
    String receiptURA,
    String receiptHashHex,
    String invocationID,
    String prevReceiptHashHex,
    int index,
    Map<String, Object> metadata) {
  public ReceiptRef {
    receiptURA = ReceiptSupport.required(receiptURA, "receipt_ura");
    receiptHashHex = ReceiptSupport.normalizeHash(receiptHashHex, "receipt_hash_hex");
    invocationID = invocationID == null ? "" : invocationID;
    prevReceiptHashHex =
        prevReceiptHashHex == null || prevReceiptHashHex.isEmpty()
            ? ""
            : ReceiptSupport.normalizeHash(prevReceiptHashHex, "prev_receipt_hash_hex");
    if (index < -1) {
      throw ReceiptSupport.invalid("index must be non-negative");
    }
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
  }

  public static ReceiptRef fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "receipt ref JSON");
    return new ReceiptRef(
        ReceiptSupport.requiredJSON(fields, "receipt_ura"),
        ReceiptSupport.requiredJSON(fields, "receipt_hash_hex"),
        ReceiptSupport.optionalJSON(fields, "invocation_id"),
        ReceiptSupport.optionalJSON(fields, "prev_receipt_hash_hex"),
        ReceiptSupport.optionalIndex(fields, "index"),
        ReceiptSupport.optionalObject(fields, "metadata"));
  }

  public static ReceiptRef fromSummary(ReceiptSummary summary) {
    if (summary == null || summary.receiptURA().isEmpty()) {
      throw ReceiptSupport.invalid("receipt_ura is required");
    }
    throw ReceiptSupport.invalid("receipt_hash_hex is required");
  }

  public byte[] toJSON() {
    LinkedHashMap<String, Object> value = new LinkedHashMap<>();
    value.put("receipt_ura", receiptURA);
    value.put("receipt_hash_hex", receiptHashHex);
    if (!invocationID.isEmpty()) {
      value.put("invocation_id", invocationID);
    }
    if (!prevReceiptHashHex.isEmpty()) {
      value.put("prev_receipt_hash_hex", prevReceiptHashHex);
    }
    if (index >= 0) {
      value.put("index", index);
    }
    if (!metadata.isEmpty()) {
      value.put("metadata", metadata);
    }
    return JsonValueWriter.object(value);
  }
}
