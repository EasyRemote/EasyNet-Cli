package run.easynet.daemon;

public record InvocationTuple(
    String caller,
    String callee,
    String descriptor,
    String subject,
    String nonce,
    String causalContext,
    String argsJson) {
  public InvocationTuple {
    caller = required(caller, "caller");
    callee = required(callee, "callee");
    descriptor = required(descriptor, "descriptor");
    subject = required(subject, "subject");
    nonce = required(nonce, "nonce");
    causalContext = required(causalContext, "causalContext");
    argsJson = required(argsJson, "argsJson");
  }

  private static String required(String value, String field) {
    if (value == null || value.isBlank()) {
      throw SDKError.validation("invocation", field + " is required");
    }
    return value;
  }
}
