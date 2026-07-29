package run.runtime.sdk;

import java.util.Base64;

final class InvocationNonce {
  private InvocationNonce() {}

  static String requiredBase64(String value) {
    String clean = value == null ? "" : value.trim();
    if (clean.isBlank()) {
      throw SDKError.validation("invocation", "nonce_base64 is required");
    }
    byte[] decoded;
    try {
      decoded = Base64.getDecoder().decode(clean);
    } catch (IllegalArgumentException error) {
      throw SDKError.validation("invocation", "nonce_base64 must be canonical base64");
    }
    if (decoded.length != 16) {
      throw SDKError.validation("invocation", "nonce_base64 must decode to 16 bytes");
    }
    return clean;
  }
}
