package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class IdentityClient implements AutoCloseable {
  private final IdentityTransport transport;
  private boolean closed;

  public IdentityClient(IdentityTransport transport) {
    this.transport = Objects.requireNonNull(transport, "transport");
  }

  public IdentityProjection projectDescriptorRef(DescriptorRefRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return IdentityProjection.fromJSON(transport.projectDescriptorRef(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("identity descriptor_ref projection transport failed", error);
    }
  }

  public IdentityProjection buildDescriptorRef(DescriptorRefBuildRequest request) {
    requireOpen();
    Objects.requireNonNull(request, "request");
    try {
      return IdentityProjection.fromJSON(transport.buildDescriptorRef(request.toJSON()));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("identity descriptor_ref build transport failed", error);
    }
  }

  public String canonicalAbilityDescriptorRef(String value) {
    IdentityProjection projection = projectDescriptorRef(new DescriptorRefRequest(value, Map.of()));
    return DirectoryIdentitySupport.cleanRequired(projection.descriptorRef(), "descriptor_ref");
  }

  public String canonicalAbilityDescriptorRef(String abilityURA, String descriptorVersion) {
    IdentityProjection projection =
        buildDescriptorRef(new DescriptorRefBuildRequest(abilityURA, descriptorVersion, Map.of()));
    return DirectoryIdentitySupport.cleanRequired(projection.descriptorRef(), "descriptor_ref");
  }

  public String abilityURAFromDescriptorRef(String descriptorRef) {
    IdentityProjection projection =
        projectDescriptorRef(new DescriptorRefRequest(descriptorRef, Map.of()));
    return DirectoryIdentitySupport.cleanRequired(projection.abilityURA(), "ability_ura");
  }

  public String ownerAbilityURA(String ownerURA, String abilityName) {
    requireOpen();
    String cleanOwner = DirectoryIdentitySupport.cleanRequired(ownerURA, "owner_ura");
    String cleanAbility = DirectoryIdentitySupport.cleanRequired(abilityName, "ability_name");
    try {
      Map<String, Object> response =
          JsonValueReader.object(
              transport.ownerAbilityURA(
                  JsonValueWriter.object(
                      Map.of("owner_ura", cleanOwner, "ability_name", cleanAbility))),
              "identity owner ability JSON");
      String abilityURA = DirectoryIdentitySupport.optionalString(response.get("ability_ura"), "ability_ura");
      if (abilityURA == null) {
        abilityURA = DirectoryIdentitySupport.optionalString(response.get("ura"), "ura");
      }
      return DirectoryIdentitySupport.cleanRequired(abilityURA, "ability_ura");
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("identity owner ability transport failed", error);
    }
  }

  public IdentityProjection buildURA(Map<String, Object> request) {
    requireOpen();
    Map<String, Object> cleanRequest =
        DirectoryIdentitySupport.requiredCopiedObject(request, "build_ura request");
    try {
      return IdentityProjection.fromJSON(
          transport.buildURA(JsonValueWriter.object(cleanRequest)));
    } catch (SDKError error) {
      throw error;
    } catch (RuntimeException error) {
      throw transportFailure("identity URA build transport failed", error);
    }
  }

  public String resourceURA(String ownerURA, String path) {
    String cleanOwner = DirectoryIdentitySupport.cleanRequired(ownerURA, "owner_ura");
    String cleanPath = DirectoryIdentitySupport.cleanRequired(path, "path");
    IdentityProjection projection =
        buildURA(Map.of("kind", "resource", "owner_ura", cleanOwner, "path", cleanPath));
    String resourceURA =
        DirectoryIdentitySupport.optionalString(projection.resourceURA(), "resource_ura");
    if (resourceURA == null) {
      resourceURA = DirectoryIdentitySupport.optionalString(projection.ura(), "ura");
    }
    return DirectoryIdentitySupport.cleanRequired(resourceURA, "resource_ura");
  }

  public String descriptorBoundResourceSubjectURA(String ownerURA, String path) {
    return resourceURA(ownerURA, path);
  }

  public String ownerAbilityDescriptorRef(
      String ownerURA, String abilityName, String descriptorVersion) {
    return canonicalAbilityDescriptorRef(
        ownerAbilityURA(ownerURA, abilityName),
        DirectoryIdentitySupport.cleanRequired(descriptorVersion, "descriptor_version"));
  }

  @Override
  public void close() {
    if (closed) {
      return;
    }
    closed = true;
    transport.close();
  }

  private void requireOpen() {
    if (closed) {
      throw SDKError.closed("identity");
    }
  }

  private static SDKError transportFailure(String message, RuntimeException cause) {
    return new SDKError(
        ErrorCode.TRANSPORT,
        "transport",
        RetryHint.SAFE,
        true,
        message,
        "",
        "",
        "",
        Map.of(),
        cause);
  }
}
