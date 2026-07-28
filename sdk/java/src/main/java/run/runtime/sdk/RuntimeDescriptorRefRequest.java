package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;

public record RuntimeDescriptorRefRequest(
    String calleeURA,
    String ability,
    String callMode,
    String callerURA,
    String subjectURA,
    String provider) {
  static final String ABILITY_DESCRIPTOR_PROVIDER = "ability_descriptor";
  static final String RECEIPT_HISTORY_PROVIDER = "receipt_history";

  public RuntimeDescriptorRefRequest {
    calleeURA = required(calleeURA, "callee_ura");
    ability = required(ability, "ability");
    callMode = required(callMode, "call_mode");
    callerURA = optionalPrincipal(callerURA, "caller_ura");
    subjectURA = optionalPrincipal(subjectURA, "subject_ura");
    provider = provider == null ? "" : provider.trim();
    validateProvider(ability, provider);
    validateProviderSubject(calleeURA, callerURA, subjectURA, provider);
  }

  Map<String, Object> toWireObject() {
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("callee_ura", calleeURA);
    out.put("ability", ability);
    out.put("call_mode", callMode);
    if (!callerURA.isBlank()) {
      out.put("caller_ura", callerURA);
    }
    if (!subjectURA.isBlank()) {
      out.put("subject_ura", subjectURA);
    }
    if (!provider.isBlank()) {
      out.put("provider", provider);
    }
    return out;
  }

  private static void validateProvider(String ability, String provider) {
    String expected = RuntimeAbilityProjection.runtimeGovernanceDescriptorProviderForAbility(ability);
    if (expected.isBlank()) {
      if (!provider.isBlank()) {
        throw SDKError.validation(
            "runtime",
            "descriptor_ref provider "
                + provider
                + " cannot resolve non-governance ability "
                + ability);
      }
      return;
    }
    if (provider.isBlank()) {
      throw SDKError.validation(
          "runtime",
          "runtime governance read ability "
              + ability
              + " requires descriptor_ref provider "
              + expected);
    }
    if (!provider.equals(expected)) {
      throw SDKError.validation(
          "runtime",
          "descriptor_ref provider "
              + provider
              + " cannot resolve ability "
              + ability
              + "; use provider "
              + expected);
    }
  }

  private static void validateProviderSubject(
      String calleeURA, String callerURA, String subjectURA, String provider) {
    if (provider.isBlank()) {
      return;
    }
    if (callerURA.isBlank() || subjectURA.isBlank()) {
      throw SDKError.validation(
          "runtime",
          "descriptor_ref provider requests require caller_ura and subject_ura");
    }
    if (ABILITY_DESCRIPTOR_PROVIDER.equals(provider)) {
      String authority = RuntimeAbilityProjection.authorityURAForRealmOf(calleeURA);
      if (!subjectURA.equals(authority)) {
        throw SDKError.validation(
            "runtime",
            "ability_descriptor provider descriptor resolution subject must be the callee realm Authority");
      }
    }
  }

  private static String required(String value, String field) {
    String clean = value == null ? "" : value.trim();
    if (clean.isBlank()) {
      throw SDKError.validation("runtime", field + " is required");
    }
    return clean;
  }

  private static String optionalPrincipal(String value, String field) {
    String clean = value == null ? "" : value.trim();
    if (!clean.isBlank() && RuntimePrincipals.containsAllZeroPrincipal(clean)) {
      throw SDKError.validation("runtime", field + " must not be all-zero");
    }
    return clean;
  }
}
