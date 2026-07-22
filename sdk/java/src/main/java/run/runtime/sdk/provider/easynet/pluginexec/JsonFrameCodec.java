package run.runtime.sdk.provider.easynet.pluginexec;

import java.io.BufferedReader;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class JsonFrameCodec {
  private final String input;
  private int index;

  private JsonFrameCodec(String input) {
    this.input = input;
  }

  static Map<String, Object> readObjectLine(BufferedReader input) throws IOException {
    String line = input.readLine();
    if (line == null) {
      throw new SidecarProtocolError("missing sidecar request frame");
    }
    Object decoded;
    try {
      decoded = new JsonFrameCodec(line).readComplete();
    } catch (SidecarProtocolError error) {
      throw error;
    } catch (RuntimeException error) {
      throw new SidecarProtocolError("invalid sidecar request JSON: " + error.getMessage(), error);
    }
    if (!(decoded instanceof Map<?, ?> raw)) {
      throw new SidecarProtocolError("sidecar request frame must be an object");
    }
    Map<String, Object> object = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw new SidecarProtocolError("sidecar request object keys must be strings");
      }
      object.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(object);
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
        throw new SidecarProtocolError("sidecar response number must be finite");
      }
      return floatValue.toString();
    }
    if (value instanceof Double doubleValue) {
      if (!Double.isFinite(doubleValue)) {
        throw new SidecarProtocolError("sidecar response number must be finite");
      }
      return doubleValue.toString();
    }
    if (value instanceof Map<?, ?> map) {
      StringBuilder builder = new StringBuilder("{");
      boolean first = true;
      for (Map.Entry<?, ?> entry : map.entrySet()) {
        if (!(entry.getKey() instanceof String key)) {
          throw new SidecarProtocolError("sidecar response object keys must be strings");
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
    throw new SidecarProtocolError(
        "sidecar response contains unsupported value type " + value.getClass().getName());
  }

  private Object readComplete() {
    Object value = readValue();
    skipWhitespace();
    if (!atEnd()) {
      throw new SidecarProtocolError("json contains trailing data");
    }
    return value;
  }

  private Object readValue() {
    skipWhitespace();
    if (atEnd()) {
      throw new SidecarProtocolError("json is empty");
    }
    char current = input.charAt(index);
    return switch (current) {
      case '{' -> readObject();
      case '[' -> readArray();
      case '"' -> readString();
      case 't' -> readLiteral("true", Boolean.TRUE);
      case 'f' -> readLiteral("false", Boolean.FALSE);
      case 'n' -> readLiteral("null", null);
      default -> {
        if (current == '-' || Character.isDigit(current)) {
          yield readNumber();
        }
        throw new SidecarProtocolError("json contains unsupported value");
      }
    };
  }

  private Map<String, Object> readObject() {
    expect('{');
    Map<String, Object> object = new LinkedHashMap<>();
    skipWhitespace();
    if (consume('}')) {
      return Collections.unmodifiableMap(object);
    }
    while (true) {
      skipWhitespace();
      if (atEnd() || input.charAt(index) != '"') {
        throw new SidecarProtocolError("json object key must be a string");
      }
      String key = readString();
      skipWhitespace();
      expect(':');
      object.put(key, readValue());
      skipWhitespace();
      if (consume('}')) {
        return Collections.unmodifiableMap(object);
      }
      expect(',');
    }
  }

  private List<Object> readArray() {
    expect('[');
    List<Object> values = new ArrayList<>();
    skipWhitespace();
    if (consume(']')) {
      return Collections.unmodifiableList(values);
    }
    while (true) {
      values.add(readValue());
      skipWhitespace();
      if (consume(']')) {
        return Collections.unmodifiableList(values);
      }
      expect(',');
    }
  }

  private String readString() {
    expect('"');
    StringBuilder builder = new StringBuilder();
    while (!atEnd()) {
      char current = input.charAt(index++);
      if (current == '"') {
        return builder.toString();
      }
      if (current != '\\') {
        builder.append(current);
        continue;
      }
      if (atEnd()) {
        throw new SidecarProtocolError("json contains unterminated escape");
      }
      char escaped = input.charAt(index++);
      switch (escaped) {
        case '"' -> builder.append('"');
        case '\\' -> builder.append('\\');
        case '/' -> builder.append('/');
        case 'b' -> builder.append('\b');
        case 'f' -> builder.append('\f');
        case 'n' -> builder.append('\n');
        case 'r' -> builder.append('\r');
        case 't' -> builder.append('\t');
        case 'u' -> builder.append(readUnicodeEscape());
        default -> throw new SidecarProtocolError("json contains unsupported escape");
      }
    }
    throw new SidecarProtocolError("json contains unterminated string");
  }

  private char readUnicodeEscape() {
    if (index + 4 > input.length()) {
      throw new SidecarProtocolError("json contains truncated unicode escape");
    }
    String hex = input.substring(index, index + 4);
    index += 4;
    try {
      return (char) Integer.parseInt(hex, 16);
    } catch (NumberFormatException error) {
      throw new SidecarProtocolError("json contains invalid unicode escape", error);
    }
  }

  private Object readNumber() {
    int start = index;
    consume('-');
    if (!consume('0')) {
      readDigits();
    }
    boolean fractional = false;
    if (consume('.')) {
      fractional = true;
      readDigits();
    }
    if (!atEnd() && (input.charAt(index) == 'e' || input.charAt(index) == 'E')) {
      fractional = true;
      index++;
      if (!atEnd() && (input.charAt(index) == '+' || input.charAt(index) == '-')) {
        index++;
      }
      readDigits();
    }
    String number = input.substring(start, index);
    try {
      if (fractional) {
        return Double.valueOf(number);
      }
      return Long.valueOf(number);
    } catch (NumberFormatException error) {
      throw new SidecarProtocolError("json contains invalid number", error);
    }
  }

  private void readDigits() {
    int start = index;
    while (!atEnd() && Character.isDigit(input.charAt(index))) {
      index++;
    }
    if (start == index) {
      throw new SidecarProtocolError("json number requires digit");
    }
  }

  private Object readLiteral(String literal, Object value) {
    if (!input.startsWith(literal, index)) {
      throw new SidecarProtocolError("json contains invalid literal");
    }
    index += literal.length();
    return value;
  }

  private void skipWhitespace() {
    while (!atEnd()) {
      char current = input.charAt(index);
      if (current != ' ' && current != '\n' && current != '\r' && current != '\t') {
        return;
      }
      index++;
    }
  }

  private boolean consume(char expected) {
    if (!atEnd() && input.charAt(index) == expected) {
      index++;
      return true;
    }
    return false;
  }

  private void expect(char expected) {
    if (!consume(expected)) {
      throw new SidecarProtocolError("json expected '" + expected + "'");
    }
  }

  private boolean atEnd() {
    return index >= input.length();
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
