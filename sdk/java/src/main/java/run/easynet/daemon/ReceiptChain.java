package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record ReceiptChain(List<ReceiptRef> receipts, Map<String, Object> metadata) {
  public ReceiptChain {
    if (receipts == null || receipts.isEmpty()) {
      throw ReceiptSupport.invalid("receipt chain requires at least one receipt");
    }
    receipts = List.copyOf(receipts);
    metadata = metadata == null ? Map.of() : Map.copyOf(metadata);
  }

  public static ReceiptChain of(List<ReceiptRef> receipts) {
    return new ReceiptChain(receipts, Map.of());
  }

  public byte[] toJSON() {
    List<Object> encoded = new ArrayList<>(receipts.size());
    for (ReceiptRef receipt : receipts) {
      encoded.add(JsonValueReader.object(receipt.toJSON(), "receipt ref JSON"));
    }
    return JsonValueWriter.object(Map.of("receipts", encoded, "metadata", metadata));
  }
}
