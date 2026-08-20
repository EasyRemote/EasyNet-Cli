package run.runtime.sdk;

import java.util.Objects;

public final class AbilityDescriptorClient {
  private final AbilityDescriptorProvider provider;

  public AbilityDescriptorClient(AbilityDescriptorProvider provider) {
    this.provider = Objects.requireNonNull(provider, "provider");
  }

  public AbilityDescriptorPage list(AbilityDescriptorListRequest request) {
    return provider.list(Objects.requireNonNull(request, "request"));
  }

  public AbilityDescriptorProjection get(AbilityDescriptorGetRequest request) {
    return provider.get(Objects.requireNonNull(request, "request"));
  }
}
