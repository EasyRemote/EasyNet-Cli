package run.runtime.sdk;

import java.util.Map;

public record FeatureSet(
    int abiVersion,
    String sdkVersion,
    Map<String, String> profiles,
    Map<String, Boolean> symbols) {
  public FeatureSet {
    if (abiVersion <= 0) {
      throw SDKError.validation("feature_discovery", "abiVersion must be positive");
    }
    if (sdkVersion == null || sdkVersion.isBlank()) {
      throw SDKError.validation("feature_discovery", "sdkVersion is required");
    }
    profiles = profiles == null ? Map.of() : Map.copyOf(profiles);
    symbols = symbols == null ? Map.of() : Map.copyOf(symbols);
  }
}
