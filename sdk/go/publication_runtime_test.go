package easynet

import (
	"context"
	"testing"
)

func TestPublicationRuntimeTransportBuildsDeployInvocation(t *testing.T) {
	identityTransport := newPublicationRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: publicationRuntimeDeployRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimePublicationClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimePublicationClient: %v", err)
	}

	draft, err := client.BuildDeployInvocation(context.Background(), baseAbilityDeployRequest())
	if err != nil {
		t.Fatalf("BuildDeployInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["node_id"] != "local" || args["resource_ref"] == nil {
		t.Fatalf("deploy args not preserved: %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != publicationProfile ||
		metadata["system_ability"] != publicationAbilityDeploy ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("metadata not normalized: %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 || identityTransport.seenBuildURA[0]["ability_name"] != publicationAbilityDeploy {
		t.Fatalf("ability URA was not delegated through identity client: %#v", identityTransport.seenBuildURA)
	}
}

func TestPublicationRuntimeTransportDeploysAndListsThroughRuntime(t *testing.T) {
	identity, err := NewIdentityClient(newPublicationRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: publicationRuntimeDeployRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimePublicationClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimePublicationClient: %v", err)
	}

	deploy, err := client.DeployAbility(context.Background(), baseAbilityDeployRequest())
	if err != nil {
		t.Fatalf("DeployAbility: %v", err)
	}
	if deploy.PublicName != "weather" || deploy.State != "enabled" {
		t.Fatalf("deploy = %#v", deploy)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0" {
		t.Fatalf("deploy descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}

	runtimeTransport.outputJSON = publicationRuntimeListRawJSON
	page, err := client.ListAbilities(context.Background(), basePublishedAbilityQuery())
	if err != nil {
		t.Fatalf("ListAbilities: %v", err)
	}
	if page.Limit != DefaultPublishedAbilityPageSize || len(page.Items) != 1 {
		t.Fatalf("page = %#v", page)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0" {
		t.Fatalf("list descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}

	ability, err := client.ShowAbilityWithRequest(context.Background(), baseShowAbilityRequest())
	if err != nil {
		t.Fatalf("ShowAbilityWithRequest: %v", err)
	}
	if ability.Descriptor["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0" {
		t.Fatalf("shown ability = %#v", ability)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0" {
		t.Fatalf("show descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}

	runtimeTransport.outputJSON = publicationRuntimeUnpublishRawJSON
	record, err := client.UnpublishAbilityWithRequest(context.Background(), baseUnpublishRequest())
	if err != nil {
		t.Fatalf("UnpublishAbilityWithRequest: %v", err)
	}
	if record.DescriptorRef != "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0" || record.Status == nil || *record.Status != "unpublished" {
		t.Fatalf("unpublish record = %#v", record)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0" {
		t.Fatalf("unpublish descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
}

func TestPublicationRuntimeTransportBuildsUnpublishInvocation(t *testing.T) {
	identity, err := NewIdentityClient(newPublicationRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: publicationRuntimeUnpublishRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimePublicationClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimePublicationClient: %v", err)
	}

	draft, err := client.BuildUnpublishInvocation(context.Background(), baseUnpublishRequest())
	if err != nil {
		t.Fatalf("BuildUnpublishInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0" {
		t.Fatalf("descriptor ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["ability_ura"] != "easynet:///r/example/ability/device.dev-a.er.weather" {
		t.Fatalf("unpublish args = %#v", args)
	}
}

func TestPublicationRuntimeTransportMarksUnsupportedExports(t *testing.T) {
	identity, err := NewIdentityClient(newPublicationRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimePublicationClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimePublicationClient: %v", err)
	}

	if _, err := client.ValidatePackage(context.Background(), "/tmp/pkg", ValidatePackageOptions{}); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("ValidatePackage error = %v, want %s", err, ErrNotImplemented)
	}
	if _, err := client.InstallPlugin(context.Background(), "file:///tmp/plugin", InstallOptions{}); !IsCode(err, ErrNotImplemented) {
		t.Fatalf("InstallPlugin error = %v, want %s", err, ErrNotImplemented)
	}
	if _, err := client.ShowAbility(context.Background(), "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"); err == nil {
		t.Fatalf("descriptor-only ShowAbility unexpectedly succeeded")
	}
}

func newPublicationRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			publicationAbilityDeploy:    "easynet:///r/example/ability/device.dev-a.ability.deploy",
			publicationAbilityList:      "easynet:///r/example/ability/device.dev-a.meta.list_abilities",
			publicationAbilityUnpublish: "easynet:///r/example/ability/device.dev-a.ability.unpublish",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.ability.deploy":      "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
			"easynet:///r/example/ability/device.dev-a.meta.list_abilities": "easynet:///r/example/ability/device.dev-a.meta.list_abilities@1.0.0",
			"easynet:///r/example/ability/device.dev-a.ability.unpublish":   "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0",
			"easynet:///r/example/ability/device.dev-a.er.weather":          "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
		},
		descriptorProjection: `{
			"kind":"descriptor_ref",
			"valid":true,
			"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
			"ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather",
			"descriptor_version":"1.0.0",
			"profile":"easynet-strict-v2",
			"components":{"owner_ura":"easynet:///r/example/device/dev-a"},
			"metadata":{"grammar_owner":"axon"}
		}`,
	}
}

const publicationRuntimeDeployRawJSON = `{
  "public_name": "weather",
  "namespace": "er",
  "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
  "node_id": "local",
  "install_id": "install-1",
  "state": "enabled"
}`

const publicationRuntimeListRawJSON = `{
  "profile": "publication",
  "kind": "published_ability_page",
  "item_kind": "published_ability",
  "items": [{
    "descriptor": {
      "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
      "descriptor_version": "1.0.0",
      "schema_hash": "sha256:abc",
      "owner_ura": "easynet:///r/example/device/dev-a"
    },
    "implementation": {"impl_id": "impl-1", "enabled": true},
    "metadata": {}
  }],
  "next_cursor": null,
  "limit": 50,
  "source": "read_model",
  "metadata": {}
}`

const publicationRuntimeUnpublishRawJSON = `{
  "ok": true,
  "ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather",
  "owner_ura": "easynet:///r/example/device/dev-a",
  "public_name": "weather",
  "content_hash": "sha256:def"
}`
