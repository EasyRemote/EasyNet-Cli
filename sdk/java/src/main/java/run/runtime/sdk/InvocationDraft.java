package run.runtime.sdk;

import java.util.Map;

public record InvocationDraft(InvocationTuple tuple) {
  public InvocationDraft {
    if (tuple == null) {
      throw SDKError.validation("invocation", "tuple is required");
    }
  }

  public InvocationTuple inspectTuple() {
    return tuple;
  }

  public byte[] toJSON() {
    return JsonValueWriter.object(tuple.toWireObject());
  }

  static InvocationDraft fromWireObject(Map<String, Object> fields) {
    return new InvocationDraft(InvocationTuple.fromWireObject(fields));
  }
}
