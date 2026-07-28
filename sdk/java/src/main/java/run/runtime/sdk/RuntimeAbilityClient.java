package run.runtime.sdk;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;

public final class RuntimeAbilityClient {
  private final RuntimeClient runtime;

  public RuntimeAbilityClient(RuntimeClient runtime) {
    this.runtime = Objects.requireNonNull(runtime, "runtime");
  }

  public InvocationDraft build(RuntimeCallContext call, String abilityName, Object arguments) {
    return buildWithPolicy(call, abilityName, arguments, "rpc", RuntimeAbilityDispatchPolicy.publicAction());
  }

  public Map<String, Object> invoke(RuntimeCallContext call, String abilityName, Object arguments) {
    InvocationResult result = runtime.invoke(build(call, abilityName, arguments));
    return runtimeAbilityObjectOutput(result);
  }

  InvocationDraft buildCatalogueRead(RuntimeCallContext call, String abilityName, Object arguments) {
    return buildWithPolicy(
        call,
        abilityName,
        arguments,
        "rpc",
        RuntimeAbilityDispatchPolicy.catalogueRead());
  }

  Map<String, Object> invokeCatalogueRead(
      RuntimeCallContext call, String abilityName, Object arguments) {
    InvocationResult result = runtime.invoke(buildCatalogueRead(call, abilityName, arguments));
    return runtimeAbilityObjectOutput(result);
  }

  private InvocationDraft buildWithPolicy(
      RuntimeCallContext call,
      String abilityName,
      Object arguments,
      String callMode,
      RuntimeAbilityDispatchPolicy policy) {
    Objects.requireNonNull(call, "call");
    String ability = required(abilityName, "ability name");
    if (!policy.allowGovernanceRead()
        && !RuntimeAbilityProjection.runtimeGovernanceDescriptorProviderForAbility(ability).isBlank()) {
      throw SDKError.validation(
          "runtime",
          "runtime governance receipt/history/catalogue abilities must use RuntimeReceiptProvider or RuntimeAbilityDescriptorProvider");
    }
    String mode = required(callMode, "call_mode");
    String subjectURA = policy.subjectURA(call);
    String descriptorRef =
        runtime.resolveDescriptorRef(
            new RuntimeDescriptorRefRequest(
                call.calleeURA(),
                ability,
                mode,
                call.callerURA(),
                policy.descriptorResolutionSubjectURA(call, subjectURA),
                policy.descriptorProvider()));
    RuntimeAbilityProjection projection =
        RuntimeAbilityProjection.fromResolvedDescriptorRef(call.calleeURA(), descriptorRef);
    Map<String, Object> metadata = canonicalRuntimeCallMetadata(call, projection);
    InvocationBuilder builder =
        runtime
        .newInvocation()
        .caller(call.callerURA())
        .callee(call.calleeURA())
        .descriptor(descriptorRef)
        .subject(subjectURA)
        .nonce(call.nonceBase64())
        .causalContext(JsonValueWriter.write(call.causalContext()))
        .argsJson(JsonValueWriter.write(arguments == null ? Map.of() : arguments))
        .metadata(metadata);
    if (policy.allowGovernanceRead()) {
      builder.runtimeGovernanceRead();
    }
    return builder.inspect();
  }

  private static Map<String, Object> canonicalRuntimeCallMetadata(
      RuntimeCallContext call, RuntimeAbilityProjection projection) {
    Map<String, Object> out = new LinkedHashMap<>(call.metadata());
    out.put("ability_ura", projection.abilityURA());
    return Map.copyOf(out);
  }

  private static Map<String, Object> runtimeAbilityObjectOutput(InvocationResult result) {
    if (!result.ok()) {
      throw new SDKError(
          ErrorCode.ABILITY_FAILED,
          "runtime",
          RetryHint.NEVER,
          false,
          "runtime ability invocation failed",
          "",
          "",
          "",
          Map.of("terminal_state", result.terminalState().name()),
          result.error());
    }
    Object output = JsonValueReader.value(result.outputJson().getBytes(java.nio.charset.StandardCharsets.UTF_8), "ability output");
    if (!(output instanceof Map<?, ?> map)) {
      throw SDKError.validation("runtime", "runtime ability output must be an object");
    }
    Map<String, Object> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : map.entrySet()) {
      if (!(entry.getKey() instanceof String key)) {
        throw SDKError.validation("runtime", "runtime ability output keys must be strings");
      }
      out.put(key, entry.getValue());
    }
    return Map.copyOf(out);
  }

  private static String required(String value, String field) {
    String clean = value == null ? "" : value.trim();
    if (clean.isBlank()) {
      throw SDKError.validation("runtime", field + " is required");
    }
    return clean;
  }

  private record RuntimeAbilityDispatchPolicy(
      boolean allowGovernanceRead, String subjectPolicy, String descriptorProvider) {
    static RuntimeAbilityDispatchPolicy publicAction() {
      return new RuntimeAbilityDispatchPolicy(false, "descriptor_bound", "");
    }

    static RuntimeAbilityDispatchPolicy catalogueRead() {
      return new RuntimeAbilityDispatchPolicy(
          true, "runtime_owner", RuntimeDescriptorRefRequest.ABILITY_DESCRIPTOR_PROVIDER);
    }

    String subjectURA(RuntimeCallContext call) {
      if ("runtime_owner".equals(subjectPolicy)) {
        return call.calleeURA();
      }
      if ("descriptor_bound".equals(subjectPolicy)) {
        return call.subjectURA();
      }
      throw SDKError.validation("runtime", "runtime ability subject policy is unsupported");
    }

    String descriptorResolutionSubjectURA(RuntimeCallContext call, String selectedSubjectURA) {
      if (RuntimeDescriptorRefRequest.ABILITY_DESCRIPTOR_PROVIDER.equals(descriptorProvider)) {
        return selectedSubjectURA;
      }
      if ("runtime_owner".equals(subjectPolicy)) {
        return selectedSubjectURA;
      }
      return call.subjectURA();
    }
  }
}
