package run.runtime.sdk.provider.easynet.pluginexec;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.StringReader;
import java.io.StringWriter;
import java.util.Map;

public final class SidecarRuntimeTest {
  public void testSidecarRuntimeHelper() throws Exception {
    sidecarInvocationProjectsDaemonFrame();
    serveWritesResultFrame();
    serveWritesErrorFrameForHandlerFailure();
    sidecarInvocationRejectsNonInvokeFrame();
    sidecarInvocationRejectsRetiredTupleAliases();
    sidecarInvocationRejectsUnknownInvocationFields();
    sidecarInvocationRejectsUnknownRequestFields();
  }

  static void sidecarInvocationProjectsDaemonFrame() {
    SidecarInvocation invocation = SidecarInvocation.fromFrame(frame());

    check("call-1".equals(invocation.callId()), "call id");
    check("easynet:///r/hub/user/alice".equals(invocation.callerURA()), "caller_ura");
    check("easynet:///r/hub/device/provider".equals(invocation.calleeURA()), "callee_ura");
    check("demo.echo".equals(invocation.abilityURA()), "ability_ura");
    check("easynet:///r/hub/resource/demo".equals(invocation.subjectURA()), "subject_ura");
    check(invocation.invocationNonce().equals(java.util.List.of(1, 2, 3, 4)), "nonce");
    check("hello".equals(invocation.args().get("message")), "args");
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
    check(response.contains("\"nonce_len\":4"), "nonce len");
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

  static void sidecarInvocationRejectsRetiredTupleAliases() {
    java.util.Map<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("caller", "easynet:///r/hub/user/bob");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", java.util.List.of(1, 2, 3, 4));
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("retired tuple aliases must fail");
    } catch (SidecarProtocolError expected) {
      check(expected.getMessage().contains("retired"), "retired alias error");
    }
  }

  static void sidecarInvocationRejectsUnknownInvocationFields() {
    java.util.Map<String, Object> invocation = new java.util.LinkedHashMap<>();
    invocation.put("caller_ura", "easynet:///r/hub/user/alice");
    invocation.put("callee_ura", "easynet:///r/hub/device/provider");
    invocation.put("ability_ura", "demo.echo");
    invocation.put("subject_ura", "easynet:///r/hub/resource/demo");
    invocation.put("invocation_nonce", java.util.List.of(1, 2, 3, 4));
    invocation.put("descriptor_ref", "legacy-provider-leak");
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
    invocation.put("invocation_nonce", java.util.List.of(1, 2, 3, 4));
    java.util.Map<String, Object> frame = new java.util.LinkedHashMap<>();
    frame.put("type", "invoke");
    frame.put("call_id", "call-1");
    frame.put("legacy_mode", "json");
    frame.put("invocation", invocation);
    try {
      SidecarInvocation.fromFrame(frame);
      throw new AssertionError("unknown request fields must fail");
    } catch (SidecarProtocolError expected) {
      check(expected.getMessage().contains("canonical request frame"), "unknown request field error");
    }
  }

  private static Map<String, Object> frame() {
    return Map.of(
        "type",
        "invoke",
        "call_id",
        "call-1",
        "invocation",
        Map.of(
            "caller_ura",
            "easynet:///r/hub/user/alice",
            "callee_ura",
            "easynet:///r/hub/device/provider",
            "ability_ura",
            "demo.echo",
            "subject_ura",
            "easynet:///r/hub/resource/demo",
            "invocation_nonce",
            java.util.List.of(1, 2, 3, 4),
            "causal_context",
            Map.of("root", true),
            "args",
            Map.of("message", "hello")));
  }

  private static String frameJSON() {
    return """
        {"type":"invoke","call_id":"call-1","invocation":{"caller_ura":"easynet:///r/hub/user/alice","callee_ura":"easynet:///r/hub/device/provider","ability_ura":"demo.echo","subject_ura":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4],"causal_context":{"root":true},"args":{"message":"hello"}}}
        """;
  }

  private static void check(boolean condition, String label) {
    if (!condition) {
      throw new AssertionError(label);
    }
  }
}
