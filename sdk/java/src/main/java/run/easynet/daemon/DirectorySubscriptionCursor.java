package run.easynet.daemon;

import java.util.LinkedHashMap;
import java.util.Map;

public record DirectorySubscriptionCursor(String stream, int sequence, String token) {
  public DirectorySubscriptionCursor {
    stream = DirectoryIdentitySupport.cleanRequired(stream, "stream");
    if (!stream.equals("directory")) {
      throw DirectoryIdentitySupport.invalidField("stream", "must be directory");
    }
    if (sequence < 0) {
      throw DirectoryIdentitySupport.invalidField("sequence", "must be non-negative");
    }
    token = DirectoryIdentitySupport.cleanRequired(token, "token");
    if (!token.equals(stream + ":" + sequence)) {
      throw DirectoryIdentitySupport.invalidField("token", "must match cursor sequence");
    }
  }

  public static DirectorySubscriptionCursor fromObject(Map<String, Object> fields) {
    return new DirectorySubscriptionCursor(
        DirectoryIdentitySupport.requiredString(fields, "stream"),
        DirectoryIdentitySupport.requiredInteger(fields, "sequence"),
        DirectoryIdentitySupport.requiredString(fields, "token"));
  }

  public Map<String, Object> toObject() {
    LinkedHashMap<String, Object> out = new LinkedHashMap<>();
    out.put("stream", stream);
    out.put("sequence", sequence);
    out.put("token", token);
    return out;
  }

  public String resumeToken() {
    return token;
  }
}
