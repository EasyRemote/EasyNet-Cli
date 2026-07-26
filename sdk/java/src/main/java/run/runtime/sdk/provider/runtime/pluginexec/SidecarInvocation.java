package run.runtime.sdk.provider.runtime.pluginexec;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

/** Handler-facing view of one runtime-admitted declarative exec sidecar call. */
public final class SidecarInvocation {
  private static final int CANONICAL_INVOCATION_NONCE_BYTES = 16;

  private final String callId;
  private final String callerURA;
  private final String calleeURA;
  private final String abilityURA;
  private final String subjectURA;
  private final List<Integer> invocationNonce;
  private final Map<String, Object> causalContext;
  private final Map<String, Object> args;
  private final String frameType;

  private SidecarInvocation(
      String callId,
      String callerURA,
      String calleeURA,
      String abilityURA,
      String subjectURA,
      List<Integer> invocationNonce,
      Map<String, Object> causalContext,
      Map<String, Object> args,
      String frameType) {
    this.callId = requireText(callId, "call_id");
    this.callerURA = requireText(callerURA, "caller_ura");
    this.calleeURA = requireText(calleeURA, "callee_ura");
    this.abilityURA = requireText(abilityURA, "ability_ura");
    this.subjectURA = requireText(subjectURA, "subject_ura");
    this.invocationNonce = List.copyOf(invocationNonce);
    this.causalContext = immutableObject(Objects.requireNonNull(causalContext, "causal_context"));
    this.args = immutableObject(Objects.requireNonNull(args, "args"));
    this.frameType = requireText(frameType, "type");
  }

  public static SidecarInvocation fromFrame(Map<String, Object> frame) {
    Objects.requireNonNull(frame, "frame");
    rejectUnknownRequestFields(frame);
    String frameType = requiredString(frame, "type");
    if (!"invoke".equals(frameType)) {
      throw new SidecarProtocolError("exec sidecar expected invoke frame, got " + frameType);
    }
    String callId = requiredString(frame, "call_id");
    Map<String, Object> invocation = requiredObject(frame, "invocation");
    rejectRetiredTupleFields(invocation);
    rejectUnknownInvocationFields(invocation);
    return new SidecarInvocation(
        callId,
        requiredString(invocation, "caller_ura"),
        requiredString(invocation, "callee_ura"),
        requiredString(invocation, "ability_ura"),
        requiredString(invocation, "subject_ura"),
        requiredNonce(invocation, "invocation_nonce"),
        requiredObject(invocation, "causal_context"),
        requiredObject(invocation, "args"),
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

  public Map<String, Object> causalContext() {
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

  private static void rejectRetiredTupleFields(Map<String, Object> object) {
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

  private static void rejectUnknownInvocationFields(Map<String, Object> object) {
    Set<String> allowed =
        Set.of(
            "caller_ura",
            "callee_ura",
            "ability_ura",
            "subject_ura",
            "invocation_nonce",
            "causal_context",
            "args");
    for (String field : object.keySet()) {
      if (!allowed.contains(field)) {
        throw new SidecarProtocolError(
            "sidecar frame field \""
                + field
                + "\" is not part of the canonical invocation frame");
      }
    }
  }

  private static void rejectUnknownRequestFields(Map<String, Object> object) {
    Set<String> allowed = Set.of("type", "call_id", "invocation");
    for (String field : object.keySet()) {
      if (!allowed.contains(field)) {
        throw new SidecarProtocolError(
            "sidecar request frame field \""
                + field
                + "\" is not part of the canonical request frame");
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
    if (raw.size() != CANONICAL_INVOCATION_NONCE_BYTES) {
      throw new SidecarProtocolError(
          "sidecar frame field \""
              + field
              + "\" must contain exactly "
              + CANONICAL_INVOCATION_NONCE_BYTES
              + " bytes");
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

  private static Map<String, Object> immutableObject(Map<String, Object> value) {
    Map<String, Object> projected = new LinkedHashMap<>();
    for (Map.Entry<String, Object> entry : value.entrySet()) {
      projected.put(entry.getKey(), immutableValue(entry.getValue()));
    }
    return Collections.unmodifiableMap(projected);
  }

  private static Object immutableValue(Object value) {
    if (value instanceof Map<?, ?> map) {
      return immutableObject(stringObject(map, "nested"));
    }
    if (value instanceof List<?> list) {
      List<Object> projected = new ArrayList<>(list.size());
      for (Object item : list) {
        projected.add(immutableValue(item));
      }
      return Collections.unmodifiableList(projected);
    }
    return value;
  }
}
