package run.runtime.sdk;

import java.util.List;

public record AbilityDescriptorPage(List<AbilityDescriptorProjection> descriptors) {
  public AbilityDescriptorPage {
    descriptors = descriptors == null ? List.of() : List.copyOf(descriptors);
  }
}
