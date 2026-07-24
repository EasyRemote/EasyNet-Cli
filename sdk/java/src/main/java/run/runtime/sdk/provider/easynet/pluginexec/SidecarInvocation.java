package run.runtime.sdk.provider.easynet.pluginexec;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/** Handler-facing view of one daemon-admitted declarative exec sidecar call. */
public final class SidecarInvocation {
  private final String callId;
  private final String callerURA;
  private final String calleeURA;
  private final String abilityURA;
  private final String subjectURA;
  private final List<Integer> invocationNonce;
  private final Object causalContext;
  private final Map<String, Object> args;
  private final String frameType;

  private SidecarInvocation(
      String callId,
      String callerURA,
      String calleeURA,
      String abilityURA,
      String subjectURA,
      List<Integer> invocationNonce,
      Object causalContext,
      Map<String, Object> args,
      String frameType) {
    this.callId = requireText(callId, "call_id");
    this.callerURA = requireText(callerURA, "caller_ura");
    this.calleeURA = requireText(calleeURA, "callee_ura");
    this.abilityURA = requireText(abilityURA, "ability_ura");
    this.subjectURA = requireText(subjectURA, "subject_ura");
    this.invocationNonce = List.copyOf(invocationNonce);
    this.causalContext = causalContext == null ? Map.of() : causalContext;
    this.args = Collections.unmodifiableMap(new LinkedHashMap<>(args));
    this.frameType = requireText(frameType, "type");
  }

  public static SidecarInvocation fromFrame(Map<String, Object> frame) {
    Objects.requireNonNull(frame, "frame");
    String frameType = requiredString(frame, "type");
    if (!"invoke".equals(frameType)) {
      throw new SidecarProtocolError("exec sidecar expected invoke frame, got " + frameType);
    }
    String callId = requiredString(frame, "call_id");
    Map<String, Object> invocation = requiredObject(frame, "invocation");
    rejectLegacyTupleAliases(invocation);
    return new SidecarInvocation(
        callId,
        requiredString(invocation, "caller_ura"),
        requiredString(invocation, "callee_ura"),
        requiredString(invocation, "ability_ura"),
        requiredString(invocation, "subject_ura"),
        requiredNonce(invocation, "invocation_nonce"),
        invocation.get("causal_context"),
        optionalObject(invocation, "args"),
        frameType);
  }

  public String callId() {
    return callId;
  }

  public String callerURA() {
    return callerURA;
  }

  public String calleeURA() {
    return calleeURA;
  }

  public String abilityURA() {
    return abilityURA;
  }

  public String subjectURA() {
    return subjectURA;
  }

  public List<Integer> invocationNonce() {
    return invocationNonce;
  }

  public Object causalContext() {
    return causalContext;
  }

  public Map<String, Object> args() {
    return args;
  }

  public String frameType() {
    return frameType;
  }

  private static String requireText(String value, String field) {
    if (value == null || value.isEmpty()) {
      throw new SidecarProtocolError("sidecar frame field \"" + field + "\" must be a string");
    }
    return value;
  }

  private static String requiredString(Map<String, Object> object, String field) {
    Object value = object.get(field);
    if (!(value instanceof String text) || text.isEmpty()) {
      throw new SidecarProtocolError("sidecar frame field \"" + field + "\" must be a string");
    }
    return text;
  }

  private static Map<String, Object> requiredObject(Map<String, Object> object, String field) {
    Object value = object.get(field);
    if (!(value instanceof Map<?, ?> raw)) {
      throw new SidecarProtocolError("sidecar frame field \"" + field + "\" must be an object");
    }
    return stringObject(raw, field);
  }

  private static Map<String, Object> optionalObject(Map<String, Object> object, String field) {
    Object value = object.get(field);
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> raw)) {
      throw new SidecarProtocolError("sidecar frame field \"" + field + "\" must be an object");
    }
    return stringObject(raw, field);
  }

  private static void rejectLegacyTupleAliases(Map<String, Object> object) {
    for (Map.Entry<String, String> entry :
        Map.of(
                "caller", "caller_ura",
                "callee", "callee_ura",
                "ability", "ability_ura",
                "subject", "subject_ura")
            .entrySet()) {
      if (object.containsKey(entry.getKey())) {
        throw new SidecarProtocolError(
            "sidecar frame field \""
                + entry.getKey()
                + "\" is retired; use \""
                + entry.getValue()
                + "\"");
      }
    }
  }

  private static Map<String, Object> stringObject(Map<?, ?> raw, String field) {
    Map<String, Object> projected = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw new SidecarProtocolError(
            "sidecar frame field \"" + field + "\" object keys must be strings");
      }
      projected.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(projected);
  }

  private static List<Integer> requiredNonce(Map<String, Object> object, String field) {
    Object value = object.get(field);
    if (!(value instanceof List<?> raw) || raw.isEmpty()) {
      throw new SidecarProtocolError("sidecar frame field \"" + field + "\" must be a byte array");
    }
    List<Integer> nonce = new ArrayList<>(raw.size());
    for (Object item : raw) {
      int byteValue;
      if (item instanceof Integer integer) {
        byteValue = integer;
      } else if (item instanceof Long longValue
          && longValue >= Integer.MIN_VALUE
          && longValue <= Integer.MAX_VALUE) {
        byteValue = longValue.intValue();
      } else {
        throw new SidecarProtocolError(
            "sidecar frame field \"" + field + "\" must contain bytes");
      }
      if (byteValue < 0 || byteValue > 255) {
        throw new SidecarProtocolError(
            "sidecar frame field \"" + field + "\" must contain bytes");
      }
      nonce.add(byteValue);
    }
    return nonce;
  }
}
