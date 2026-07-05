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
		Reason:           "operator/key rotation",
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
		Reason:           "operator/key rotation",
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
		args["reason"] != "operator/key rotation" {
		t.Fatalf("revoke args = %#v", args)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.federation.revoke@1.0.0" {
		t.Fatalf("descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
}

func TestAdminRuntimeTransportCreatesAndDeletesDeviceSessionThroughRuntime(t *testing.T) {
	identityTransport := newAdminRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: adminRuntimeCreateSessionRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAdminClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}

	session, err := client.CreateDeviceSession(context.Background(), CreateDeviceSessionRequest{
		AdminCarrierBase: adminBaseForTest(),
		DeviceURA:        "easynet:///r/example/device/dev-a",
		HubURA:           "easynet:///r/example/hub/main",
		SessionKind:      "remote_desktop",
		ExpiresUnixMS:    1893456000000,
	})
	if err != nil {
		t.Fatalf("CreateDeviceSession: %v", err)
	}
	if session.SessionID != "dev-session-1" || session.DeviceURA != "easynet:///r/example/device/dev-a" ||
		session.HubURA != "easynet:///r/example/hub/main" || session.SessionKind != "remote_desktop" {
		t.Fatalf("device session = %#v", session)
	}
	createArgs := runtimeTransport.seenDraft["args"].(map[string]any)
	if createArgs["device_ura"] != "easynet:///r/example/device/dev-a" ||
		createArgs["hub_ura"] != "easynet:///r/example/hub/main" ||
		createArgs["session_kind"] != "remote_desktop" ||
		createArgs["expires_unix_ms"].(float64) != 1893456000000 {
		t.Fatalf("session.create args = %#v", createArgs)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.session.create@1.0.0" {
		t.Fatalf("create descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}

	runtimeTransport.outputJSON = adminRuntimeDeleteSessionRawJSON
	deleted, err := client.DeleteDeviceSession(context.Background(), DeleteDeviceSessionRequest{
		AdminCarrierBase: adminBaseForTest(),
		SessionID:        "dev-session-1",
		Reason:           "operator closed",
	})
	if err != nil {
		t.Fatalf("DeleteDeviceSession: %v", err)
	}
	if deleted.Operation != adminAbilitySessionDelete || deleted.State != "ok" ||
		deleted.DeviceURA == nil || *deleted.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("delete result = %#v", deleted)
	}
	deleteArgs := runtimeTransport.seenDraft["args"].(map[string]any)
	if deleteArgs["session_id"] != "dev-session-1" || deleteArgs["reason"] != "operator closed" {
		t.Fatalf("session.delete args = %#v", deleteArgs)
	}
	if runtimeTransport.seenDraft["descriptor_ref"] != "easynet:///r/example/ability/device.dev-a.session.delete@1.0.0" {
		t.Fatalf("delete descriptor_ref = %#v", runtimeTransport.seenDraft["descriptor_ref"])
	}
	if len(identityTransport.seenBuildURA) != 2 ||
		identityTransport.seenBuildURA[0]["ability_name"] != adminAbilitySessionCreate ||
		identityTransport.seenBuildURA[1]["ability_name"] != adminAbilitySessionDelete {
		t.Fatalf("identity lookups not delegated: %#v", identityTransport.seenBuildURA)
	}
}

func newAdminRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			adminAbilityAgentList:     "easynet:///r/example/ability/device.dev-a.agent.list",
			adminAbilityAgentStart:    "easynet:///r/example/ability/device.dev-a.agent.start",
			adminAbilityAgentStop:     "easynet:///r/example/ability/device.dev-a.agent.stop",
			adminAbilityAgentRefresh:  "easynet:///r/example/ability/device.dev-a.agent.refresh",
			adminAbilitySessionList:   "easynet:///r/example/ability/device.dev-a.session.list",
			adminAbilitySessionCreate: "easynet:///r/example/ability/device.dev-a.session.create",
			adminAbilitySessionDelete: "easynet:///r/example/ability/device.dev-a.session.delete",
			adminAbilityRevokeDevice:  "easynet:///r/example/ability/device.dev-a.federation.revoke",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.agent.list":        "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.start":       "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.stop":        "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.refresh":     "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.list":      "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.create":    "easynet:///r/example/ability/device.dev-a.session.create@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.delete":    "easynet:///r/example/ability/device.dev-a.session.delete@1.0.0",
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

const adminRuntimeCreateSessionRawJSON = `{
	"session_id": "dev-session-1",
	"device_ura": "easynet:///r/example/device/dev-a",
	"hub_ura": "easynet:///r/example/hub/main",
	"state": "active",
	"session_kind": "remote_desktop",
	"created_unix_ms": 1767225600000,
	"expires_unix_ms": 1893456000000
}`

const adminRuntimeDeleteSessionRawJSON = `{
	"ack": true,
	"device_ura": "easynet:///r/example/device/dev-a"
}`
