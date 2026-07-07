package run.easynet.daemon;

public final class InvocationBuilder {
  private String caller;
  private String callee;
  private String descriptor;
  private String subject;
  private String nonce;
  private String causalContext;
  private String argsJson;

  public InvocationBuilder caller(String value) {
    caller = value;
    return this;
  }

  public InvocationBuilder callee(String value) {
    callee = value;
    return this;
  }

  public InvocationBuilder descriptor(String value) {
    descriptor = value;
    return this;
  }

  public InvocationBuilder subject(String value) {
    subject = value;
    return this;
  }

  public InvocationBuilder nonce(String value) {
    nonce = value;
    return this;
  }

  public InvocationBuilder causalContext(String value) {
    causalContext = value;
    return this;
  }

  public InvocationBuilder argsJson(String value) {
    argsJson = value;
    return this;
  }

  public InvocationDraft inspect() {
    return new InvocationDraft(
        new InvocationTuple(caller, callee, descriptor, subject, nonce, causalContext, argsJson));
  }
}
