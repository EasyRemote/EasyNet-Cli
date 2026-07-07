package run.easynet.daemon;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public record MissionStatus(
    String profile,
    String kind,
    String missionID,
    String state,
    boolean terminal,
    int partialFailures,
    boolean cancelled,
    String parentInvocationID,
    String parentReceiptURA,
    Map<String, Object> parentInvocation,
    List<MissionChildInvocation> childInvocations,
    List<MissionChildReceipt> childReceipts,
    List<MissionOutputRef> outputRefs,
    Map<String, Object> error,
    Map<String, Object> metadata) {
  public MissionStatus {
    if (!MissionSupport.PROFILE.equals(profile) || !"mission_status".equals(kind)) {
      throw MissionSupport.invalid("invalid mission status projection");
    }
    missionID = MissionSupport.missionID(missionID);
    state = MissionSupport.required(state, "state");
    if (partialFailures < 0) {
      throw MissionSupport.invalid("partial_failures must be non-negative");
    }
    parentInvocationID = parentInvocationID == null ? "" : parentInvocationID;
    parentReceiptURA = parentReceiptURA == null ? "" : parentReceiptURA;
    parentInvocation = MissionSupport.copyObject(parentInvocation);
    childInvocations = childInvocations == null ? List.of() : List.copyOf(childInvocations);
    childReceipts = childReceipts == null ? List.of() : List.copyOf(childReceipts);
    outputRefs = outputRefs == null ? List.of() : List.copyOf(outputRefs);
    error = MissionSupport.copyObject(error);
    metadata = MissionSupport.copyObject(metadata);
  }

  public static MissionStatus fromJSON(byte[] raw) {
    Map<String, Object> fields = JsonValueReader.object(raw, "mission status JSON");
    List<MissionChildInvocation> childInvocations = new ArrayList<>();
    for (Object item : MissionSupport.requiredList(fields, "child_invocations")) {
      Map<String, Object> child = MissionSupport.optionalObject(item, "child_invocations");
      if (child == null) {
        throw MissionSupport.invalid("child_invocations entry must be an object");
      }
      childInvocations.add(MissionChildInvocation.fromObject(child));
    }
    List<MissionChildReceipt> childReceipts = new ArrayList<>();
    for (Object item : MissionSupport.requiredList(fields, "child_receipts")) {
      Map<String, Object> receipt = MissionSupport.optionalObject(item, "child_receipts");
      if (receipt == null) {
        throw MissionSupport.invalid("child_receipts entry must be an object");
      }
      childReceipts.add(MissionChildReceipt.fromObject(receipt));
    }
    List<MissionOutputRef> outputRefs = new ArrayList<>();
    for (Object item : MissionSupport.requiredList(fields, "output_refs")) {
      Map<String, Object> output = MissionSupport.optionalObject(item, "output_refs");
      if (output == null) {
        throw MissionSupport.invalid("output_refs entry must be an object");
      }
      outputRefs.add(MissionOutputRef.fromObject(output));
    }
    return new MissionStatus(
        MissionSupport.requiredString(fields, "profile"),
        MissionSupport.requiredString(fields, "kind"),
        MissionSupport.requiredString(fields, "mission_id"),
        MissionSupport.requiredString(fields, "state"),
        MissionSupport.requiredBoolean(fields, "terminal"),
        MissionSupport.requiredInteger(fields, "partial_failures"),
        MissionSupport.requiredBoolean(fields, "cancelled"),
        MissionSupport.optionalString(fields, "parent_invocation_id"),
        MissionSupport.optionalString(fields, "parent_receipt_ura"),
        MissionSupport.requiredObject(fields, "parent_invocation"),
        childInvocations,
        childReceipts,
        outputRefs,
        MissionSupport.optionalObject(fields.get("error"), "error"),
        MissionSupport.requiredObject(fields, "metadata"));
  }
}
