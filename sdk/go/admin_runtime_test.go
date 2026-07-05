package easynet

import (
	"context"
	"testing"
)

func TestAdminRuntimeTransportBuildsAgentStartInvocation(t *testing.T) {
	identityTransport := newAdminRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: adminRuntimeAgentStartRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAdminClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}

	draft, err := client.BuildAgentStartInvocation(context.Background(), AdminAgentStartRequest{
		AdminCarrierBase: adminBaseForTest(),
		Name:             "codex",
		AgentType:        "codex",
		Model:            "gpt-5",
		Label:            "primary",
	})
	if err != nil {
		t.Fatalf("BuildAgentStartInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0" {
		t.Fatalf("descriptor_ref = %q", draft.DescriptorRef())
	}
	args := draft.JSONArgs().(map[string]any)
	if args["name"] != "codex" || args["agent_type"] != "codex" || args["model"] != "gpt-5" {
		t.Fatalf("agent.start args = %#v", args)
	}
	metadata := draft.Metadata()
	if metadata["profile"] != adminGatewayProfile ||
		metadata["system_ability"] != adminAbilityAgentStart ||
		metadata["carrier_owner"] != "daemon_sdk" {
		t.Fatalf("metadata = %#v", metadata)
	}
	if len(identityTransport.seenBuildURA) != 1 ||
		identityTransport.seenBuildURA[0]["ability_name"] != adminAbilityAgentStart {
		t.Fatalf("identity lookup not delegated: %#v", identityTransport.seenBuildURA)
	}
}

func TestAdminRuntimeTransportRevokesDeviceThroughRuntime(t *testing.T) {
	identity, err := NewIdentityClient(newAdminRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: adminRuntimeRevokeRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAdminClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}

	draft, err := client.BuildRevokeDeviceInvocation(context.Background(), RevokeDeviceRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		Reason:           "operator-initiated device removal",
	})
	if err != nil {
		t.Fatalf("BuildRevokeDeviceInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", draft.DescriptorRef())
	}

	result, err := client.RevokeDevice(context.Background(), RevokeDeviceRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		Reason:           "operator-initiated device removal",
	})
	if err != nil {
		t.Fatalf("RevokeDevice: %v", err)
	}
	if result.Profile != adminGatewayProfile || result.State != "ok" || result.DeviceURA == nil ||
		*result.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("revoke result = %#v", result)
	}
	args := runtimeTransport.seenDraft["args"].(map[string]any)
	if args["agent_ura"] != "easynet:///r/example/device/dev-a" ||
		args["reason"] != "operator-initiated device removal" {
		t.Fatalf("revoke args = %#v", args)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
}

func newAdminRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			adminAbilityAgentList:    "easynet:///r/example/ability/device.dev-a.agent.list",
			adminAbilityAgentStart:   "easynet:///r/example/ability/device.dev-a.agent.start",
			adminAbilityAgentStop:    "easynet:///r/example/ability/device.dev-a.agent.stop",
			adminAbilityAgentRefresh: "easynet:///r/example/ability/device.dev-a.agent.refresh",
			adminAbilitySessionList:  "easynet:///r/example/ability/device.dev-a.session.list",
			adminAbilityRevokeDevice: "easynet:///r/example/ability/device.dev-a.federation.revoke",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.agent.list":        "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.start":       "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.stop":        "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.refresh":     "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.list":      "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.federation.revoke": "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0",
		},
		descriptorProjection: identityDescriptorProjectionJSON,
	}
}

const adminRuntimeAgentStartRawJSON = `{
	"agent_ura": "easynet:///r/example/agent/codex",
	"runtime_registered": 1,
	"runtime_failed": 0
}`

const adminRuntimeRevokeRawJSON = `{
	"ack": true,
	"was_active": true
}`
