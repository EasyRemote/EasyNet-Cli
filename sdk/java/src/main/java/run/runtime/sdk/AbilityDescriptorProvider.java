package run.runtime.sdk;

public interface AbilityDescriptorProvider {
  AbilityDescriptorPage list(AbilityDescriptorListRequest request);

  AbilityDescriptorProjection get(AbilityDescriptorGetRequest request);
}
