package run.easynet.daemon;

public record InvocationDraft(InvocationTuple tuple) {
  public InvocationDraft {
    if (tuple == null) {
      throw SDKError.validation("invocation", "tuple is required");
    }
  }

  public InvocationTuple inspectTuple() {
    return tuple;
  }
}
