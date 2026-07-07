package run.easynet.daemon;

import java.util.List;
import java.util.Map;
import java.util.Objects;

public record DirectoryResolvedRef(
    String profile,
    String kind,
    String answerKind,
    String queryName,
    String canonicalName,
    String ownerURA,
    String abilityURA,
    String routeURA,
    Map<String, Object> nextHop,
    Map<String, Object> selectedRoute,
    List<Object> routeCandidates,
    List<Object> records,
    Map<String, Object> negative,
    String releaseProfile,
    Map<String, Object> authority,
    Map<String, Object> cachePolicy,
    Map<String, Object> metadata) {
  public DirectoryResolvedRef {
    if (!"directory_identity".equals(profile)) {
      throw DirectoryIdentitySupport.invalidField("profile", "must be directory_identity");
    }
    if (!"resolved_ref".equals(kind)) {
      throw DirectoryIdentitySupport.invalidField("kind", "must be resolved_ref");
    }
    routeCandidates = DirectoryIdentitySupport.copyList(routeCandidates);
    records = DirectoryIdentitySupport.copyList(records);
    metadata = DirectoryIdentitySupport.requiredCopiedObject(metadata, "metadata");
  }

  public static DirectoryResolvedRef fromJSON(byte[] raw) {
    Objects.requireNonNull(raw, "raw");
    Map<String, Object> fields = JsonValueReader.object(raw, "directory resolved-ref JSON");
    return new DirectoryResolvedRef(
        DirectoryIdentitySupport.requiredString(fields, "profile"),
        DirectoryIdentitySupport.requiredString(fields, "kind"),
        DirectoryIdentitySupport.requiredString(fields, "answer_kind"),
        DirectoryIdentitySupport.optionalString(fields.get("query_name"), "query_name"),
        DirectoryIdentitySupport.optionalString(fields.get("canonical_name"), "canonical_name"),
        DirectoryIdentitySupport.optionalString(fields.get("owner_ura"), "owner_ura"),
        DirectoryIdentitySupport.optionalString(fields.get("ability_ura"), "ability_ura"),
        DirectoryIdentitySupport.optionalString(fields.get("route_ura"), "route_ura"),
        DirectoryIdentitySupport.optionalObject(fields.get("next_hop"), "next_hop"),
        DirectoryIdentitySupport.optionalObject(fields.get("selected_route"), "selected_route"),
        DirectoryIdentitySupport.requiredList(fields, "route_candidates"),
        DirectoryIdentitySupport.requiredList(fields, "records"),
        DirectoryIdentitySupport.optionalObject(fields.get("negative"), "negative"),
        DirectoryIdentitySupport.optionalString(fields.get("release_profile"), "release_profile"),
        DirectoryIdentitySupport.optionalObject(fields.get("authority"), "authority"),
        DirectoryIdentitySupport.optionalObject(fields.get("cache_policy"), "cache_policy"),
        DirectoryIdentitySupport.requiredObject(fields, "metadata"));
  }
}
