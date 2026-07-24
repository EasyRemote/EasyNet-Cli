package run.runtime.sdk.provider.runtime.pluginexec;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

/** Provider-scoped runtime helper for runtime declarative exec sidecars. */
public final class SidecarRuntime {
  private SidecarRuntime() {}

  /** Run one declarative exec plugin invocation using process stdin/stdout. */
  public static void serve(SidecarHandler handler) throws IOException {
    try (BufferedReader input =
            new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        BufferedWriter output =
            new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8))) {
      serve(input, output, handler);
    }
  }

  /** Run one declarative exec plugin invocation over explicit streams. */
  public static void serve(BufferedReader input, BufferedWriter output, SidecarHandler handler)
      throws IOException {
    String callId = "";
    try {
      Map<String, Object> frame = JsonFrameCodec.readObjectLine(input);
      callId = optionalCallId(frame);
      SidecarInvocation invocation = SidecarInvocation.fromFrame(frame);
      Object value = handler.handle(invocation);
      writeFrame(output, resultFrame(invocation.callId(), value));
    } catch (Exception error) {
      writeFrame(output, errorFrame(callId, error.getMessage()));
    }
  }

  private static String optionalCallId(Map<String, Object> frame) {
    Object value = frame.get("call_id");
    return value instanceof String text ? text : "";
  }

  private static Map<String, Object> resultFrame(String callId, Object value) {
    Map<String, Object> frame = new LinkedHashMap<>();
    frame.put("type", "result");
    frame.put("call_id", callId);
    frame.put("value", value);
    return frame;
  }

  private static Map<String, Object> errorFrame(String callId, String message) {
    Map<String, Object> frame = new LinkedHashMap<>();
    frame.put("type", "error");
    frame.put("call_id", callId);
    frame.put("message", message == null ? "" : message);
    return frame;
  }

  private static void writeFrame(BufferedWriter output, Map<String, Object> frame)
      throws IOException {
    output.write(JsonFrameCodec.write(frame));
    output.newLine();
    output.flush();
  }
}
