package run.runtime.sdk;

import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;

final class JsonValueWriter {
  private JsonValueWriter() {}

  static byte[] object(Map<String, Object> value) {
    return write(value).getBytes(StandardCharsets.UTF_8);
  }

  static String write(Object value) {
    if (value == null) {
      return "null";
    }
    if (value instanceof String string) {
      return quote(string);
    }
    if (value instanceof Boolean bool) {
      return bool.toString();
    }
    if (value instanceof Integer
        || value instanceof Long
        || value instanceof Short
        || value instanceof Byte) {
      return value.toString();
    }
    if (value instanceof Float floatValue) {
      if (!Float.isFinite(floatValue)) {
        throw SDKError.validation("json", "number must be finite");
      }
      return floatValue.toString();
    }
    if (value instanceof Double doubleValue) {
      if (!Double.isFinite(doubleValue)) {
        throw SDKError.validation("json", "number must be finite");
      }
      return doubleValue.toString();
    }
    if (value instanceof Map<?, ?> map) {
      StringBuilder builder = new StringBuilder("{");
      boolean first = true;
      for (Map.Entry<?, ?> entry : map.entrySet()) {
        if (!(entry.getKey() instanceof String key)) {
          throw SDKError.validation("json", "object keys must be strings");
        }
        if (!first) {
          builder.append(',');
        }
        first = false;
        builder.append(quote(key)).append(':').append(write(entry.getValue()));
      }
      return builder.append('}').toString();
    }
    if (value instanceof List<?> list) {
      StringBuilder builder = new StringBuilder("[");
      for (int i = 0; i < list.size(); i++) {
        if (i > 0) {
          builder.append(',');
        }
        builder.append(write(list.get(i)));
      }
      return builder.append(']').toString();
    }
    throw SDKError.validation("json", "unsupported value type " + value.getClass().getName());
  }

  private static String quote(String value) {
    StringBuilder builder = new StringBuilder(value.length() + 2);
    builder.append('"');
    for (int i = 0; i < value.length(); i++) {
      char current = value.charAt(i);
      switch (current) {
        case '"' -> builder.append("\\\"");
        case '\\' -> builder.append("\\\\");
        case '\b' -> builder.append("\\b");
        case '\f' -> builder.append("\\f");
        case '\n' -> builder.append("\\n");
        case '\r' -> builder.append("\\r");
        case '\t' -> builder.append("\\t");
        default -> {
          if (current < 0x20) {
            builder.append(String.format("\\u%04x", (int) current));
          } else {
            builder.append(current);
          }
        }
      }
    }
    return builder.append('"').toString();
  }
}
