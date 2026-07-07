package run.easynet.daemon;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;

final class HostBindingSupport {
  static final String PROFILE = "host_binding";
  static final String FRAME_SCHEMA = "host-stream-frame.schema.json";
  static final String HASH_ALGORITHM = "sha256(prev_hash || seq_be || canonical_json(value))";
  static final String EMPTY_OUTPUT_HASH =
      "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

  private HostBindingSupport() {}

  static String required(String value, String field) {
    if (value == null || value.isEmpty() || !value.equals(value.trim())) {
      throw invalid(field + " is required");
    }
    return value;
  }

  static String optional(String value, String field) {
    if (value == null || value.isEmpty()) {
      return "";
    }
    if (!value.equals(value.trim())) {
      throw invalid(field + " must not contain surrounding whitespace");
    }
    return value;
  }

  static String endpoint(String value) {
    String cleaned = required(value, "endpoint");
    if (!cleaned.startsWith("/") && !cleaned.startsWith("unix:///")) {
      throw invalid("host stream endpoint must be absolute");
    }
    return cleaned;
  }

  static String frameSchema(String value) {
    String cleaned = required(value, "frame_schema");
    if (!FRAME_SCHEMA.equals(cleaned)) {
      throw invalid("frame_schema must be host-stream-frame.schema.json");
    }
    return cleaned;
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Collections.unmodifiableMap(new LinkedHashMap<>(value));
  }

  static Map<String, Object> requiredObject(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(name + " must be an object");
    }
    LinkedHashMap<String, Object> copied = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(name + " keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(copied);
  }

  static Map<String, Object> optionalObject(Object value, String name) {
    if (value == null) {
      return Map.of();
    }
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(name + " must be an object or null");
    }
    LinkedHashMap<String, Object> copied = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(name + " keys must be strings");
      }
      copied.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(copied);
  }

  static String requiredString(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof String string && !string.isEmpty()) {
      return string;
    }
    throw invalid(name + " must be a non-empty string");
  }

  static String optionalString(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (value instanceof String string) {
      return string;
    }
    throw invalid(name + " must be a string or null");
  }

  static boolean requiredBoolean(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Boolean bool) {
      return bool;
    }
    throw invalid(name + " must be a boolean");
  }

  static long requiredLong(Map<String, Object> fields, String name) {
    Object value = fields.get(name);
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer");
  }

  static Long optionalLong(Object value, String name) {
    if (value == null) {
      return null;
    }
    if (value instanceof Number number) {
      double doubleValue = number.doubleValue();
      long longValue = number.longValue();
      if (longValue >= 0 && doubleValue == (double) longValue) {
        return longValue;
      }
    }
    throw invalid(name + " must be a non-negative integer or null");
  }

  static void validateHashState(String algorithm, String outputHash, long frames, Long lastSeq) {
    if (!HASH_ALGORITHM.equals(algorithm) || !isOutputHash(outputHash) || frames < 0) {
      throw invalid("invalid host stream hash state projection");
    }
    if (frames == 0) {
      if (lastSeq != null) {
        throw invalid("host stream hash state cannot have last_seq when frames is zero");
      }
      return;
    }
    if (lastSeq == null || lastSeq.longValue() != frames - 1) {
      throw invalid("host stream hash state last_seq must match frames");
    }
  }

  static void validateHashFold(HostStreamHashState state, long seq) {
    validateHashState(state.algorithm(), state.outputHash(), state.frames(), state.lastSeq());
    if (seq != state.frames()) {
      throw invalid("host stream hash sequence gap");
    }
  }

  static HostStreamHashState foldOutputHash(HostStreamHashState state, long seq, Object value) {
    validateHashFold(state, seq);
    String canonicalJSON = JsonValueWriter.write(value);
    byte[] previous = decodeHash(state.outputHash());
    byte[] sequence = ByteBuffer.allocate(Long.BYTES).putLong(seq).array();
    try {
      MessageDigest digest = MessageDigest.getInstance("SHA-256");
      digest.update(previous);
      digest.update(sequence);
      digest.update(canonicalJSON.getBytes(StandardCharsets.UTF_8));
      return new HostStreamHashState(
          HASH_ALGORITHM,
          "sha256:" + hex(digest.digest()),
          state.frames() + 1,
          seq,
          canonicalJSON);
    } catch (NoSuchAlgorithmException error) {
      throw transport("sha256 digest is not available", error);
    }
  }

  static boolean isOutputHash(String value) {
    return value != null && value.matches("sha256:[0-9a-f]{64}");
  }

  private static byte[] decodeHash(String value) {
    if (!isOutputHash(value)) {
      throw invalid("output_hash must be a sha256 digest");
    }
    String hex = value.substring("sha256:".length());
    byte[] out = new byte[32];
    for (int i = 0; i < out.length; i++) {
      out[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
    }
    return out;
  }

  private static String hex(byte[] value) {
    StringBuilder builder = new StringBuilder(value.length * 2);
    for (byte item : value) {
      builder.append(String.format("%02x", item & 0xff));
    }
    return builder.toString();
  }

  static SDKError invalid(String message) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        PROFILE,
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        null);
  }

  static SDKError unsupported(String message) {
    return new SDKError(
        ErrorCode.NOT_IMPLEMENTED,
        "transport",
        RetryHint.NEVER,
        false,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        null);
  }

  static SDKError transport(String message, Throwable cause) {
    if (cause instanceof SDKError sdkError) {
      return sdkError;
    }
    return new SDKError(
        ErrorCode.TRANSPORT,
        "transport",
        RetryHint.SAFE,
        true,
        message,
        "",
        "",
        "",
        Map.of("profile", PROFILE),
        cause);
  }
}
