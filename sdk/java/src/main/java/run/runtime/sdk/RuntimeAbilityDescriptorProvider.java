package run.runtime.sdk;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

public final class RuntimeAbilityDescriptorProvider implements AbilityDescriptorProvider {
  private static final String LIST_ABILITY = "meta.list_abilities";
  private static final String ROWS_FIELD = "abilities";

  private final RuntimeAbilityClient ability;

  public RuntimeAbilityDescriptorProvider(RuntimeAbilityClient ability) {
    this.ability = Objects.requireNonNull(ability, "ability");
  }

  @Override
  public AbilityDescriptorPage list(AbilityDescriptorListRequest request) {
    Objects.requireNonNull(request, "request");
    Map<String, Object> args = new LinkedHashMap<>();
    if (!request.scope().isBlank()) {
      args.put("scope", request.scope());
    }
    if (!request.ownerURA().isBlank()) {
      args.put("owner_ura", request.ownerURA());
    }
    if (!request.abilityURA().isBlank()) {
      args.put("ability_ura", request.abilityURA());
    }
    Map<String, Object> output = ability.invokeCatalogueRead(request.call(), LIST_ABILITY, args);
    Object rows = output.get(ROWS_FIELD);
    if (!(rows instanceof List<?> rawRows)) {
      throw SDKError.validation(
          "ability_descriptor", "runtime descriptor catalog output must include descriptor rows");
    }
    List<AbilityDescriptorProjection> descriptors = new ArrayList<>();
    for (int index = 0; index < rawRows.size(); index++) {
      Object raw = rawRows.get(index);
      if (!(raw instanceof Map<?, ?> rawMap)) {
        throw SDKError.validation(
            "ability_descriptor", "ability descriptor row " + index + " must be an object");
      }
      descriptors.add(AbilityDescriptorProjection.fromRuntimeMap(copyStringMap(rawMap, index), index));
    }
    return new AbilityDescriptorPage(descriptors);
  }

  @Override
  public AbilityDescriptorProjection get(AbilityDescriptorGetRequest request) {
    Objects.requireNonNull(request, "request");
    AbilityDescriptorPage page =
        list(
            new AbilityDescriptorListRequest(
                request.call(), request.scope(), "", request.abilityURA()));
    List<AbilityDescriptorProjection> matches = new ArrayList<>();
    for (AbilityDescriptorProjection descriptor : page.descriptors()) {
      if (!descriptor.abilityURA().equals(request.abilityURA())) {
        throw SDKError.validation(
            "ability_descriptor", "runtime returned descriptor outside requested ability_ura");
      }
      if (!request.descriptorVersion().isBlank()
          && !descriptor.version().equals(request.descriptorVersion())) {
        continue;
      }
      if (!request.callMode().isBlank() && !descriptor.callMode().equals(request.callMode())) {
        continue;
      }
      matches.add(descriptor);
    }
    if (matches.isEmpty()) {
      throw new SDKError(
          ErrorCode.DESCRIPTOR_NOT_FOUND,
          "ability_descriptor",
          RetryHint.NEVER,
          false,
          "ability descriptor not found",
          "",
          "",
          "",
          Map.of("ability_ura", request.abilityURA()),
          null);
    }
    if (matches.size() > 1) {
      throw SDKError.validation(
          "ability_descriptor",
          "ability descriptor selection is ambiguous; specify descriptor_version or call_mode");
    }
    return matches.get(0);
  }

  private static Map<String, Object> copyStringMap(Map<?, ?> raw, int index) {
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : raw.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation(
            "ability_descriptor", "ability descriptor row " + index + " keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }
}
