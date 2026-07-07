package run.easynet.daemon;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class JsonValueReader {
  private final String input;
  private int index;

  private JsonValueReader(byte[] raw) {
    this.input = new String(raw, StandardCharsets.UTF_8);
  }

  static Map<String, Object> object(byte[] raw, String label) {
    Object value = value(raw, label);
    if (!(value instanceof Map<?, ?> decoded)) {
      throw invalid(label, "must be an object");
    }
    Map<String, Object> object = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : decoded.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw invalid(label, "object key must be a string");
      }
      object.put(key, entry.getValue());
    }
    return Collections.unmodifiableMap(object);
  }

  static Object value(byte[] raw, String label) {
    try {
      return new JsonValueReader(raw).readComplete();
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw invalid(label, "is invalid", error);
    }
  }

  private Object readComplete() {
    Object value = readValue();
    skipWhitespace();
    if (!atEnd()) {
      throw invalid("json", "contains trailing data");
    }
    return value;
  }

  private Object readValue() {
    skipWhitespace();
    if (atEnd()) {
      throw invalid("json", "is empty");
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
        throw invalid("json", "contains unsupported value");
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
        throw invalid("json", "object key must be a string");
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
        throw invalid("json", "contains unterminated escape");
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
        default -> throw invalid("json", "contains unsupported escape");
      }
    }
    throw invalid("json", "contains unterminated string");
  }

  private char readUnicodeEscape() {
    if (index + 4 > input.length()) {
      throw invalid("json", "contains truncated unicode escape");
    }
    String hex = input.substring(index, index + 4);
    index += 4;
    try {
      return (char) Integer.parseInt(hex, 16);
    } catch (NumberFormatException error) {
      throw invalid("json", "contains invalid unicode escape", error);
    }
  }

  private Object readNumber() {
    int start = index;
    consume('-');
    if (consume('0')) {
      // Leading zero consumed.
    } else {
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
      throw invalid("json", "contains invalid number", error);
    }
  }

  private void readDigits() {
    int start = index;
    while (!atEnd() && Character.isDigit(input.charAt(index))) {
      index++;
    }
    if (start == index) {
      throw invalid("json", "number requires digit");
    }
  }

  private Object readLiteral(String literal, Object value) {
    if (!input.startsWith(literal, index)) {
      throw invalid("json", "contains invalid literal");
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
      throw invalid("json", "expected '" + expected + "'");
    }
  }

  private boolean atEnd() {
    return index >= input.length();
  }

  private static SDKError invalid(String label, String message) {
    return invalid(label, message, null);
  }

  private static SDKError invalid(String label, String message, Throwable cause) {
    return new SDKError(
        ErrorCode.INVALID_ARGUMENT,
        "decode",
        RetryHint.NEVER,
        false,
        label + " " + message,
        "",
        "",
        "",
        Map.of(),
        cause);
  }
}
