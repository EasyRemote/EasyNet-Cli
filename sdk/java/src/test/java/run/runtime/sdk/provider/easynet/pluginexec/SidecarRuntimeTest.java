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
  }

  static void sidecarInvocationProjectsDaemonFrame() {
    SidecarInvocation invocation = SidecarInvocation.fromFrame(frame());

    check("call-1".equals(invocation.callId()), "call id");
    check("easynet:///r/hub/user/alice".equals(invocation.caller()), "caller");
    check("easynet:///r/hub/device/provider".equals(invocation.callee()), "callee");
    check("demo.echo".equals(invocation.ability()), "ability");
    check("easynet:///r/hub/resource/demo".equals(invocation.subject()), "subject");
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

  private static Map<String, Object> frame() {
    return Map.of(
        "type",
        "invoke",
        "call_id",
        "call-1",
        "invocation",
        Map.of(
            "caller",
            "easynet:///r/hub/user/alice",
            "callee",
            "easynet:///r/hub/device/provider",
            "ability",
            "demo.echo",
            "subject",
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
        {"type":"invoke","call_id":"call-1","invocation":{"caller":"easynet:///r/hub/user/alice","callee":"easynet:///r/hub/device/provider","ability":"demo.echo","subject":"easynet:///r/hub/resource/demo","invocation_nonce":[1,2,3,4],"causal_context":{"root":true},"args":{"message":"hello"}}}
        """;
  }

  private static void check(boolean condition, String label) {
    if (!condition) {
      throw new AssertionError(label);
    }
  }
}
