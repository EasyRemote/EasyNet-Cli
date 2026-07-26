package run.runtime.sdk.provider.runtime.pluginexec;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.StringReader;
import java.io.StringWriter;
import java.util.Map;

public final class SidecarRuntimeTest {
  public static void main(String[] args) throws Exception {
    new SidecarRuntimeTest().testSidecarRuntimeHelper();
  }

  public void testSidecarRuntimeHelper() throws Exception {
    sidecarInvocationProjectsDaemonFrame();
    serveWritesResultFrame();
    serveWritesArrayResultFrame();
    serveWritesErrorFrameForHandlerFailure();
    sidecarInvocationRejectsNonInvokeFrame();
    sidecarInvocationRejectsNonCanonicalTupleAliases();
    sidecarInvocationRejectsUnknownInvocationFields();
    sidecarInvocationRejectsUnknownRequestFields();
    sidecarInvocationRejectsMissingCanonicalInvocationObjects();
    sidecarInvocationRejectsNonCanonicalNonceLength();
  }

  static void sidecarInvocationProjectsDaemonFrame() {
    SidecarInvocation invocation = SidecarInvocation.fromFrame(frame());

    check("call-1".equals(invocation.callId()), "call id");
    check("easynet:///r/hub/user/alice".equals(invocation.callerURA()), "caller_ura");
    check("easynet:///r/hub/device/provider".equals(invocation.calleeURA()), "callee_ura");
    check("demo.echo".equals(invocation.abilityURA()), "ability_ura");
    check("easynet:///r/hub/resource/demo".equals(invocation.subjectURA()), "subject_ura");
    check(invocation.invocationNonce().equals(canonicalNonce()), "nonce");
    check("none".equals(invocation.causalContext().get("form")), "causal_context");
    check("hello".equals(invocation.args().get("message")), "args");
    try {
      invocation.causalContext().put("form", "mutated");
      throw new AssertionError("causal_context projection must be immutable");
    } catch (UnsupportedOperationException expected) {
      // expected
    }
    java.util.Map<String, Object> mutableFrame = frameWithMutableArgs();
    SidecarInvocation owned = SidecarInvocation.fromFrame(mutableFrame);
    @SuppressWarnings("unchecked")
    java.util.Map<String, Object> sourceInvocation =
        (java.util.Map<String, Object>) mutableFrame.get("invocation");
    @SuppressWarnings("unchecked")
    java.util.Map<String, Object> sourceArgs =
        (java.util.Map<String, Object>) sourceInvocation.get("args");
    @SuppressWarnings("unchecked")
    java.util.Map<String, Object> sourceNested =
        (java.util.Map<String, Object>) sourceArgs.get("nested");
    sourceNested.put("value", "mutated-after-projection");
    @SuppressWarnings("unchecked")
    java.util.Map<String, Object> projectedNested =
        (java.util.Map<String, Object>) owned.args().get("nested");
    check("owned".equals(projectedNested.get("value")), "args projection owns nested maps");
    try {
      projectedNested.put("value", "handler-mutation");
      throw new AssertionError("nested args projection must be immutable");
    } catch (UnsupportedOperationException expected) {
      // expected
    }
  }

  static void serveWritesResultFrame() throws Exception {
    StringWriter output = new StringWriter();

    SidecarRuntime.serve(
        new BufferedReader(new StringReader(frameJSON())),
        new BufferedWriter(output),
        invocation ->
            Map.of(
                "ok", true,
                "message", invocation.args().get("message"),
                "nonce_len", invocation.invocationNonce().size()));

    String response = output.toString();
    check(response.contains("\"type\":\"result\""), "result type");
    check(response.contains("\"call_id\":\"call-1\""), "call id");
    check(response.contains("\"message\":\"hello\""), "message");
    check(response.contains("\"nonce_len\":16"), "nonce len");
  }

  static void serveWritesArrayResultFrame() throws Exception {
    StringWriter output = new StringWriter();

    SidecarRuntime.serve(
        new BufferedReader(new StringReader(frameJSON())),
        new BufferedWriter(output),
        invocation ->
            Map.of(
                "channels", new String[] {"audio", "video"},
                "samples", new int[] {1, 2, 3}));

    String response = output.toString();
    check(response.contains("\"type\":\"result\""), "array result type");
    check(response.contains("\"channels\":[\"audio\",\"video\"]"), "string array result");
    check(response.contains("\"samples\":[1,2,3]"), "primitive array result");
  }

  static void serveWritesErrorFrameForHandlerFailure() throws Exception {
    StringWriter output = new StringWriter();

    SidecarRuntime.serve(
        new BufferedReader(new StringReader(frameJSON())),
        new BufferedWriter(output),
        invocation -> {
          throw new IllegalStateException("boom");
        });

    String response = output.toString();
    check(response.contains("\"type\":\"error\""), "error type");
    check(response.contains("\"call_id\":\"call-1\""), "call id");
    check(response.contains("\"message\":\"boom\""), "message");
  }

  static void sidecarInvocationRejectsNonInvokeFrame() {
    Map<String, Object> bad =
        Map.of("type", "stream_open", "call_id", "call-1", "invocation", Map.of());
    try {
      SidecarInvocation.fromFrame(bad);
      throw new AssertionError("non-invoke frame must fail");
    } catch (SidecarProtocolError expected) {
      // expected
    }
  }

  static void sidecarInvocationRejectsNonCanonicalTupleAliases() {
    java.util.Map<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("caller", "easynet:///r/hub/user/bob");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", canonicalNonce());
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("non-canonical tuple aliases must fail");
    } catch (SidecarProtocolError expected) {
      check(
          expected.getMessage().contains("canonical invocation frame"),
          "non-canonical alias error");
    }
  }

  static void sidecarInvocationRejectsUnknownInvocationFields() {
    java.util.Map<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", canonicalNonce());
    invocation.put("descriptor_ref", "retired-provider-leak");
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("unknown invocation fields must fail");
    } catch (SidecarProtocolError expected) {
      check(expected.getMessage().contains("canonical invocation frame"), "unknown field error");
    }
  }

  static void sidecarInvocationRejectsUnknownRequestFields() {
    java.util.Map<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", canonicalNonce());
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("retired_mode", "json");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("unknown request fields must fail");
    } catch (SidecarProtocolError expected) {
      check(expected.getMessage().contains("canonical request frame"), "unknown request field error");
    }
  }

  static void sidecarInvocationRejectsMissingCanonicalInvocationObjects() {
    for (String field : java.util.List.of("causal_context", "args")) {
      java.util.Map<String, Object> invocation = canonicalInvocation();
      invocation.remove(field);
      java.util.Map<String, Object> incomplete = new java.util.LinkedHashMap<>();
      incomplete.put("type", "invoke");
      incomplete.put("call_id", "call-1");
      incomplete.put("invocation", invocation);
      try {
        SidecarInvocation.fromFrame(incomplete);
        throw new AssertionError("missing canonical invocation object must fail: " + field);
      } catch (SidecarProtocolError expected) {
        check(expected.getMessage().contains("object"), "missing " + field + " error");
      }
      invocation = canonicalInvocation();
      invocation.put(field, null);
      java.util.Map<String, Object> nullField = new java.util.LinkedHashMap<>();
      nullField.put("type", "invoke");
      nullField.put("call_id", "call-1");
      nullField.put("invocation", invocation);
      try {
        SidecarInvocation.fromFrame(nullField);
        throw new AssertionError("null canonical invocation object must fail: " + field);
      } catch (SidecarProtocolError expected) {
        check(expected.getMessage().contains("object"), "null " + field + " error");
      }
    }
  }

  static void sidecarInvocationRejectsNonCanonicalNonceLength() {
    java.util.Map<String, Object> invocation = canonicalInvocation();
    invocation.put("invocation_nonce", java.util.List.of(1, 2, 3, 4));
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("non-canonical invocation nonce length must fail");
    } catch (SidecarProtocolError expected) {
      check(expected.getMessage().contains("exactly 16 bytes"), "nonce length error");
    }
  }

  private static Map<String, Object> frame() {
    return Map.of(
        "type",
        "invoke",
        "call_id",
        "call-1",
        "invocation",
        canonicalInvocation());
  }

  private static java.util.Map<String, Object> frameWithMutableArgs() {
    java.util.Map<String, Object> args = new java.util.LinkedHashMap<>();
    args.put("message", "hello");
    args.put("nested", new java.util.LinkedHashMap<>(Map.of("value", "owned")));
    java.util.Map<String, Object> invocation = canonicalInvocation();
    invocation.put("args", args);
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("invocation", invocation);
    return frame;
  }

  private static java.util.LinkedHashMap<String, Object> canonicalInvocation() {
    java.util.LinkedHashMap<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", canonicalNonce());
    invocation.put("causal_context", Map.of("form", "none"));
    invocation.put("args", Map.of("message", "hello"));
    return invocation;
  }

  private static String frameJSON() {
    return """
        {"type":"invoke","call_id":"call-1","invocation":{"caller_ura":"easynet:///r/hub/user/alice","callee_ura":"easynet:///r/hub/device/provider","ability_ura":"demo.echo","subject_ura":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16],"causal_context":{"form":"none"},"args":{"message":"hello"}}}
        """;
  }

  private static java.util.List<Integer> canonicalNonce() {
    return java.util.List.of(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
  }

  private static void check(boolean condition, String label) {
    if (!condition) {
      throw new AssertionError(label);
    }
  }
}
