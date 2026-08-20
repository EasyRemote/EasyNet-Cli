#!/usr/bin/env bash

set -euo pipefail

ROOT="${CHECK_ABILITY_IDENTITY_OWNER_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-ability-identity-owner-boundary: %s\n' "$*" >&2
  exit 1
}

OWNER="src/daemon/ability/catalog/ownership.rs"
DESCRIPTOR="src/daemon/ability/descriptors/surface.rs"
DEVICE_PROFILE="src/daemon/ability/catalog/profiles/device.rs"
OWNER_PROJECTION="src/daemon/federation/read_model/owner_projection.rs"
AGENT_LIST="src/daemon/ability/builtins/agents/list.rs"
LOCAL_INVOKE="src/support/platform/local_invoke.rs"
REMOTE_INVOKE="src/daemon/invocation/routing/remote_invoke.rs"
CLI_CATALOGUE="src/cli/daemon_client/ability_catalog.rs"

for file in "$OWNER" "$DESCRIPTOR" "$DEVICE_PROFILE" "$OWNER_PROJECTION" "$AGENT_LIST" "$LOCAL_INVOKE" "$REMOTE_INVOKE" "$CLI_CATALOGUE"; do
  [[ -f "$file" ]] || fail "missing $file"
done

grep -Fq 'easynet:///r/acme/user/alice' "$DESCRIPTOR" \
  || fail "AbilityDescriptor must retain a regression case rejecting User owners"
grep -Fq 'descriptor_constructor_rejects_non_actor_and_device_owner_locators' "$DESCRIPTOR" \
  || fail "AbilityDescriptor User-owner rejection test is missing"
validator="$(sed -n '/pub fn validate_owner_ura/,/^    }/p' "$DESCRIPTOR")"
if grep -Fq 'URAKind::Device' <<<"$validator"; then
  fail "AbilityDescriptor must not accept Device as an owner/callee"
fi
grep -Fq 'easynet:///r/acme/device/dev-1' "$DESCRIPTOR" \
  || fail "AbilityDescriptor Device-owner rejection evidence is missing"
grep -Fq 'SystemAgent, Service, or realm Authority' "$DESCRIPTOR" \
  || fail "AbilityDescriptor owner docs must name Service as an ordinary public callee"
grep -Fq 'service/<principal-id>.<service-id>' "$DESCRIPTOR" \
  || fail "AbilityDescriptor owner examples must include principal-scoped Service URAs"
grep -Fq 'Vec::new()' "$DEVICE_PROFILE" \
  || fail "DeviceProfileProjection live descriptor inventory must remain empty"
if grep -Fq '.rebind_owner_ura(owner_ura)' "$DEVICE_PROFILE"; then
  fail "DeviceProfileProjection must not manufacture Device-owned descriptors"
fi
grep -Fq 'DeviceProfileProjection migration cursor must not carry AbilityDescriptor rows' "$OWNER_PROJECTION" \
  || fail "federation receive admission must reject descriptor rows on a Device migration cursor"
grep -Fq 'integrity_rejects_device_profile_projection_descriptor_rows' "$OWNER_PROJECTION" \
  || fail "Device migration cursor receive-admission regression test is missing"

grep -Fq '.registered_agents()' "$AGENT_LIST" \
  || fail "agent.list must enumerate the Agent aggregate, not account identities"
grep -Fq 'snapshot.hosted_llm_agent_ura(name)' "$AGENT_LIST" \
  || fail "agent.list must project only explicit hosted-Agent URAs"
grep -Fq 'missing hosted-Agent identity must not synthesize the User account as an Agent' "$AGENT_LIST" \
  || fail "agent.list needs an explicit no-account-synthesis regression assertion"

grep -Fq 'fn device_sponsored_system_agent_owner_for_public_ability(' "$OWNER" \
  || fail "device-native owner projection must expose a typed SystemAgent owner domain object"
grep -Fq 'super::catalog_metadata::unique_system_agent_owner_for_public_ability(public_ability)' "$OWNER" \
  || fail "device-native owner projection must come from registry ownership"
grep -Fq 'is_declared_daemon_native_system_agent_id(&system_agent_id)' "$OWNER" \
  || fail "device-native owner projection must validate the SystemAgent inventory"
grep -Fq 'META_LIST_ABILITIES' "$OWNER" \
  || fail "meta.list_abilities ownership regression evidence is missing"
grep -Fq 'RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID' "$OWNER" \
  || fail "meta.list_abilities must map to runtime-introspection"

grep -Fq 'LocalRuntimeCatalogueReadIssuer::list_abilities' "$CLI_CATALOGUE" \
  || fail "debug ability list must enter through the typed catalogue issuer"
grep -Fq 'LocalAbilityTarget::for_device_sponsored_system_ability(ability, execution_host_ura)' "$LOCAL_INVOKE" \
  || fail "local catalogue dispatch must separate Device host from SystemAgent callee"
grep -Fq 'fn local_catalogue_target_is_runtime_introspection_system_agent()' "$LOCAL_INVOKE" \
  || fail "local debug catalogue target regression test is missing"
grep -Fq 'runtime_introspection_owner_for_execution_target(execution_target_ura)?' "$REMOTE_INVOKE" \
  || fail "remote catalogue reads must use the same runtime-introspection owner projection"

echo "check-ability-identity-owner-boundary: ok"
