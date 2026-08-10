package easynet

import (
	"context"
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

func TestCanonicalAddressingBuildsDescriptorAndSubjectWithoutIdentityProfile(t *testing.T) {
	addressing := NewCanonicalAddressing()
	ctx := context.Background()

	descriptorProjection, err := addressing.BuildDescriptorRef(
		ctx,
		CanonicalDescriptorRefBuildRequest{
			AbilityURA:        "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health",
			DescriptorVersion: "1.0.0",
			DescriptorHash:    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			Action:            "invoke",
		},
	)
	if err != nil {
		t.Fatalf("BuildDescriptorRef: %v", err)
	}
	descriptorRef := descriptorProjection.DescriptorRef
	if descriptorRef != "easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke" {
		t.Fatalf("descriptor_ref = %q", descriptorRef)
	}
	descriptorProjection, err = addressing.ProjectDescriptorRef(
		ctx,
		CanonicalDescriptorRefRequest{DescriptorRef: descriptorRef},
	)
	if err != nil {
		t.Fatalf("ProjectDescriptorRef: %v", err)
	}
	if descriptorProjection.Profile != uraProfileStrictV2 {
		t.Fatalf("descriptor profile = %q", descriptorProjection.Profile)
	}
	if descriptorProjection.DescriptorVersion != "1.0.0" ||
		descriptorProjection.DescriptorHash != "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" ||
		descriptorProjection.Action != "invoke" {
		t.Fatalf("descriptor proof facts = %#v", descriptorProjection)
	}

	subject, err := addressing.DescriptorBoundResourceSubjectURA(
		ctx,
		"easynet:///r/example/user/alice",
		"invoke/observe.health",
	)
	if err != nil {
		t.Fatalf("DescriptorBoundResourceSubjectURA: %v", err)
	}
	if subject != "easynet:///r/example/resource/user.alice/invoke/observe.health" {
		t.Fatalf("subject = %q", subject)
	}

	owner, err := addressing.OwnerURAForAbility(
		ctx,
		"easynet:///r/example/ability/system-agent.dev-a.runtime-health.observe.health",
	)
	if err != nil {
		t.Fatalf("OwnerURAForAbility: %v", err)
	}
	if owner != "easynet:///r/example/agent/device.dev-a.runtime-health" {
		t.Fatalf("owner = %q", owner)
	}
	ownerProjection, err := addressing.ProjectIdentity(
		ctx,
		URAProjectionRequest{URA: owner},
	)
	if err != nil {
		t.Fatalf("ProjectIdentity: %v", err)
	}
	if ownerProjection.Profile != uraProfileStrictV2 {
		t.Fatalf("owner profile = %q", ownerProjection.Profile)
	}
}

func TestCanonicalAddressingRejectsNonPublisherAndMalformedDescriptor(t *testing.T) {
	addressing := NewCanonicalAddressing()
	ctx := context.Background()

	if _, err := addressing.OwnerAbilityURA(
		ctx,
		"easynet:///r/example/user/alice",
		"observe.health",
	); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("user owner error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := addressing.ProjectDescriptorRef(
		ctx,
		CanonicalDescriptorRefRequest{DescriptorRef: "not-a-descriptor"},
	); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("descriptor error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := addressing.BuildURA(
		ctx,
		CanonicalURABuildRequest{Kind: "agent", Realm: "example", UserID: "alice", AgentID: "assistant"},
	); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("missing owner_kind error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestCanonicalAddressingSharedProjectionCorpus(t *testing.T) {
	t.Parallel()
	raw, err := os.ReadFile("../conformance/fixtures/canonical-addressing.v5.json")
	if err != nil {
		t.Fatalf("read shared addressing fixture: %v", err)
	}
	var fixture struct {
		Profile      string `json:"profile"`
		GrammarOwner string `json:"grammar_owner"`
		URACases     []struct {
			Name       string                   `json:"name"`
			Request    CanonicalURABuildRequest `json:"request"`
			URA        string                   `json:"ura"`
			Components map[string]any           `json:"components"`
		} `json:"ura_cases"`
		Descriptor struct {
			Raw        string         `json:"raw"`
			Components map[string]any `json:"components"`
		} `json:"descriptor"`
		InvalidURAs []string `json:"invalid_uras"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatalf("decode shared addressing fixture: %v", err)
	}
	addressing := NewCanonicalAddressing()
	ctx := context.Background()
	for _, testCase := range fixture.URACases {
		t.Run(testCase.Name, func(t *testing.T) {
			projection, err := addressing.BuildURA(ctx, testCase.Request)
			if err != nil {
				t.Fatalf("BuildURA: %v", err)
			}
			if projection.URA != testCase.URA || projection.Profile != fixture.Profile {
				t.Fatalf("projection identity = %#v", projection)
			}
			if !reflect.DeepEqual(projection.Components, testCase.Components) {
				t.Fatalf("components = %#v, want %#v", projection.Components, testCase.Components)
			}
			if projection.Metadata["grammar_owner"] != fixture.GrammarOwner {
				t.Fatalf("grammar_owner = %#v", projection.Metadata["grammar_owner"])
			}
		})
	}
	descriptor, err := addressing.ProjectDescriptorRef(
		ctx,
		CanonicalDescriptorRefRequest{DescriptorRef: fixture.Descriptor.Raw},
	)
	if err != nil {
		t.Fatalf("ProjectDescriptorRef: %v", err)
	}
	if !reflect.DeepEqual(descriptor.Components, fixture.Descriptor.Components) {
		t.Fatalf("descriptor components = %#v, want %#v", descriptor.Components, fixture.Descriptor.Components)
	}
	for _, invalid := range fixture.InvalidURAs {
		if _, err := addressing.ProjectIdentity(ctx, URAProjectionRequest{URA: invalid}); !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("invalid URA %q error = %v", invalid, err)
		}
	}
}
