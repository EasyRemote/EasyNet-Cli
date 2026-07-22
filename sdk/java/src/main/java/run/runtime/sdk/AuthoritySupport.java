package run.runtime.sdk;

import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class AuthoritySupport {
  static final String DELEGATION_METADATA_KEY = "x-runtime-delegation";
  static final String SESSION_AUTHORITY_METADATA_KEY = "x-runtime-session-authority";
  static final String DELEGATION_KIND = "delegation";
  static final String SESSION_AUTHORITY_KIND = "session_authority";

  private AuthoritySupport() {}

  static void validateAuthorityMetadata(Map<String, Object> metadata) {
    if (metadata == null || metadata.isEmpty()) {
      return;
    }
    boolean hasDelegation = hasMetadataValue(metadata, DELEGATION_METADATA_KEY);
    boolean hasSession = hasMetadataValue(metadata, SESSION_AUTHORITY_METADATA_KEY);
    if (hasDelegation && hasSession) {
      throw invalid("invocation authority metadata is ambiguous");
    }
  }

  static String decodeAuthorityMetadataProjection(byte[] raw, String metadataKey, String label) {
    Map<String, Object> object = JsonValueReader.object(raw, label + " metadata projection");
    Object direct = object.get("metadata_value");
    if (direct instanceof String value && !value.isBlank()) {
      return value;
    }
    Object metadata = object.get("metadata");
    if (metadata instanceof Map<?, ?> map) {
      Object value = map.get(metadataKey);
      if (value instanceof String string && !string.isBlank()) {
        return string;
      }
    }
    throw invalid(label + " metadata projection missing metadata_value");
  }

  static DecodedAuthority decodeAuthorityMetadata(String value, String label) {
    String cleaned = requiredString(value, "metadata_value");
    byte[] decoded;
    try {
      decoded = Base64.getDecoder().decode(cleaned);
    } catch (IllegalArgumentException error) {
      throw invalid(label + " metadata must be base64 JSON");
    }
    Map<String, Object> wire = JsonValueReader.object(decoded, label + " authority metadata");
    Map<String, Object> payload = requiredObject(wire.get("payload"), "payload");
    String signature = requiredBase64(requiredString(wire.get("signature"), "signature"), "signature");
    return new DecodedAuthority(payload, signature);
  }

  static SDKError invalid(String message) {
    return SDKError.validation("authority", message);
  }

  static String requiredString(Object value, String field) {
    if (!(value instanceof String string) || string.isBlank() || !string.equals(string.trim())) {
      throw invalid(field + " is required");
    }
    return string;
  }

  static String requiredURA(String value, String field) {
    String cleaned = requiredString(value, field);
    rejectAllZero(cleaned, field);
    if (!cleaned.startsWith("easynet:///r/")) {
      throw invalid(field + " must be a URA");
    }
    return cleaned;
  }

  static String requiredPrincipalID(String value, String field) {
    String cleaned = requiredString(value, field);
    rejectAllZero(cleaned, field);
    return cleaned;
  }

  private static void rejectAllZero(String value, String field) {
    if (value.trim().toLowerCase().contains("00000000-0000-0000-0000-000000000000")) {
      throw invalid(field + " must not be all-zero");
    }
  }

  static String requiredBase64(String value, String field) {
    String cleaned = requiredString(value, field);
    try {
      Base64.getDecoder().decode(cleaned);
    } catch (IllegalArgumentException error) {
      throw invalid(field + " must be base64");
    }
    return cleaned;
  }

  static long requiredLong(Object value, String field) {
    if (value instanceof Long longValue && longValue >= 0) {
      return longValue;
    }
    if (value instanceof Integer integerValue && integerValue >= 0) {
      return integerValue.longValue();
    }
    throw invalid(field + " must be a non-negative integer");
  }

  static List<String> requiredStringList(Object value, String field) {
    if (!(value instanceof List<?> raw) || raw.isEmpty()) {
      throw invalid(field + " must be a non-empty string array");
    }
    List<String> out = new ArrayList<>();
    for (Object item : raw) {
      out.add(requiredString(item, field));
    }
    return List.copyOf(out);
  }

  static List<String> requiredScopes(List<String> scopes) {
    if (scopes == null || scopes.isEmpty()) {
      throw invalid("authority scopes are required");
    }
    List<String> out = new ArrayList<>();
    for (String scope : scopes) {
      out.add(requiredString(scope, "scope"));
    }
    return List.copyOf(out);
  }

  static Map<String, Object> requiredObject(Object value, String field) {
    if (!(value instanceof Map<?, ?> raw)) {
      throw invalid(field + " must be an object");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(field + " keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  static Map<String, Object> copyObject(Map<String, Object> value) {
    if (value == null || value.isEmpty()) {
      return Map.of();
    }
    return Map.copyOf(new LinkedHashMap<>(value));
  }

  private static boolean hasMetadataValue(Map<String, Object> metadata, String key) {
    Object value = metadata.get(key);
    return value instanceof String string && !string.isBlank();
  }

  record DecodedAuthority(Map<String, Object> payload, String signatureBase64) {}
}
