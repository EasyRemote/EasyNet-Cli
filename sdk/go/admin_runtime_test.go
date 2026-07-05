package easynet

import (
	"context"
	"encoding/json"
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

func TestAdminRuntimeTransportGatewayStatusUsesExplicitProvider(t *testing.T) {
	identity, err := NewIdentityClient(newAdminRuntimeIdentityTransport())
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtime, err := NewRuntimeClient(&compatibilityRuntimeInvokeTransport{outputJSON: adminRuntimeAgentStartRawJSON})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	var seen map[string]any
	client, err := NewRuntimeAdminClientWithGatewayStatus(runtime, identity, AdminGatewayStatusProviderFunc(func(ctx context.Context, requestJSON []byte) ([]byte, error) {
		if err := json.Unmarshal(requestJSON, &seen); err != nil {
			return nil, err
		}
		return []byte(adminGatewayStatusJSON), nil
	}))
	if err != nil {
		t.Fatalf("NewRuntimeAdminClientWithGatewayStatus: %v", err)
	}

	requirePublic := false
	status, err := client.GatewayStatus(context.Background(), AdminGatewayStatusRequest{
		RequirePublicListener: &requirePublic,
		Metadata:              map[string]any{"caller": "test"},
	})
	if err != nil {
		t.Fatalf("GatewayStatus: %v", err)
	}
	if !status.ControlReady || !status.RuntimeReady || status.Metadata["source"] != "daemon_lifecycle_status" {
		t.Fatalf("gateway status = %#v", status)
	}
	if seen["require_public_listener"] != false || seen["metadata"].(map[string]any)["caller"] != "test" {
		t.Fatalf("provider request = %#v", seen)
	}
}

func TestAdminRuntimeTransportGatewayStatusFailsClosedWithoutProvider(t *testing.T) {
	identity, err := NewIdentityClient(newAdminRuntimeIdentityTransport())
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

	if _, err := client.GatewayStatus(context.Background(), AdminGatewayStatusRequest{}); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("GatewayStatus error = %v, want %s", err, ErrInvalidArgument)
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

func TestAdminRuntimeTransportRunsHubAndPairingLifecycleThroughRuntime(t *testing.T) {
	identityTransport := newAdminRuntimeIdentityTransport()
	identity, err := NewIdentityClient(identityTransport)
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	runtimeTransport := &compatibilityRuntimeInvokeTransport{outputJSON: adminRuntimeJoinHubRawJSON}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAdminClient(runtime, identity)
	if err != nil {
		t.Fatalf("NewRuntimeAdminClient: %v", err)
	}

	join, err := client.JoinHub(context.Background(), AdminJoinHubRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("JoinHub: %v", err)
	}
	if join.Operation != adminAbilityHubJoin || join.DeviceURA == nil || *join.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("join result = %#v", join)
	}
	joinArgs := runtimeTransport.seenDraft["args"].(map[string]any)
	if joinArgs["hub_ura"] != "easynet:///r/example/hub/main" || joinArgs["device_ura"] != "easynet:///r/example/device/dev-a" {
		t.Fatalf("hub.join args = %#v", joinArgs)
	}

	runtimeTransport.outputJSON = adminRuntimePairingPreflightRawJSON
	preflight, err := client.PairingPreflight(context.Background(), PairingPreflightRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		RequestedScopes:  []string{"invoke", "events"},
	})
	if err != nil {
		t.Fatalf("PairingPreflight: %v", err)
	}
	if !preflight.PairingRequired || preflight.TrustReady || len(preflight.Scopes) != 2 {
		t.Fatalf("preflight = %#v", preflight)
	}
	preflightArgs := runtimeTransport.seenDraft["args"].(map[string]any)
	if preflightArgs["hub_ura"] != "easynet:///r/example/hub/main" ||
		preflightArgs["device_ura"] != "easynet:///r/example/device/dev-a" ||
		len(preflightArgs["requested_scopes"].([]any)) != 2 {
		t.Fatalf("pairing.preflight args = %#v", preflightArgs)
	}

	runtimeTransport.outputJSON = adminRuntimeCreatePairingRawJSON
	token, err := client.CreatePairing(context.Background(), CreatePairingRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		ExpiresUnixMS:    1893456000000,
		Scopes:           []string{"invoke", "events"},
	})
	if err != nil {
		t.Fatalf("CreatePairing: %v", err)
	}
	if token.TokenID != "pair-token-1" || token.Token != "pair-token-value" || len(token.Scopes) != 2 {
		t.Fatalf("pairing token = %#v", token)
	}

	runtimeTransport.outputJSON = adminRuntimeValidatePairingRawJSON
	credential, err := client.ValidatePairing(context.Background(), ValidatePairingRequest{
		AdminCarrierBase: adminBaseForTest(),
		Token:            "pair-token-value",
		DeviceURA:        "easynet:///r/example/device/dev-a",
	})
	if err != nil {
		t.Fatalf("ValidatePairing: %v", err)
	}
	if credential.CredentialID != "cred-dev-a" || credential.HubURA != "easynet:///r/example/hub/main" {
		t.Fatalf("credential = %#v", credential)
	}

	runtimeTransport.outputJSON = adminRuntimeVerifyCredentialRawJSON
	verification, err := client.VerifyDeviceCredential(context.Background(), VerifyDeviceCredentialRequest{
		AdminCarrierBase: adminBaseForTest(),
		CredentialID:     "cred-dev-a",
		DeviceURA:        "easynet:///r/example/device/dev-a",
		HubURA:           "easynet:///r/example/hub/main",
	})
	if err != nil {
		t.Fatalf("VerifyDeviceCredential: %v", err)
	}
	if !verification.Verified || verification.Method != "daemon-trust-store" {
		t.Fatalf("verification = %#v", verification)
	}

	runtimeTransport.outputJSON = adminRuntimeLeaveHubRawJSON
	leave, err := client.LeaveHub(context.Background(), AdminLeaveHubRequest{
		AdminCarrierBase: adminBaseForTest(),
		HubURA:           "easynet:///r/example/hub/main",
		Reason:           "operator rotation",
	})
	if err != nil {
		t.Fatalf("LeaveHub: %v", err)
	}
	if leave.Operation != adminAbilityHubLeave || leave.DeviceURA == nil || *leave.DeviceURA != "easynet:///r/example/device/dev-a" {
		t.Fatalf("leave result = %#v", leave)
	}
	leaveArgs := runtimeTransport.seenDraft["args"].(map[string]any)
	if leaveArgs["hub_ura"] != "easynet:///r/example/hub/main" || leaveArgs["reason"] != "operator rotation" {
		t.Fatalf("hub.leave args = %#v", leaveArgs)
	}

	wantAbilities := []string{
		adminAbilityHubJoin,
		adminAbilityPairingPreflight,
		adminAbilityPairingCreate,
		adminAbilityPairingValidate,
		adminAbilityCredentialVerify,
		adminAbilityHubLeave,
	}
	if len(identityTransport.seenBuildURA) != len(wantAbilities) {
		t.Fatalf("identity lookups = %#v", identityTransport.seenBuildURA)
	}
	for index, ability := range wantAbilities {
		if identityTransport.seenBuildURA[index]["ability_name"] != ability {
			t.Fatalf("identity lookup %d = %#v, want %s", index, identityTransport.seenBuildURA[index], ability)
		}
	}
}

func newAdminRuntimeIdentityTransport() *compatibilityRuntimeIdentityTransport {
	return &compatibilityRuntimeIdentityTransport{
		abilityByName: map[string]string{
			adminAbilityAgentList:        "easynet:///r/example/ability/device.dev-a.agent.list",
			adminAbilityAgentStart:       "easynet:///r/example/ability/device.dev-a.agent.start",
			adminAbilityAgentStop:        "easynet:///r/example/ability/device.dev-a.agent.stop",
			adminAbilityAgentRefresh:     "easynet:///r/example/ability/device.dev-a.agent.refresh",
			adminAbilitySessionList:      "easynet:///r/example/ability/device.dev-a.session.list",
			adminAbilitySessionCreate:    "easynet:///r/example/ability/device.dev-a.session.create",
			adminAbilitySessionDelete:    "easynet:///r/example/ability/device.dev-a.session.delete",
			adminAbilityHubJoin:          "easynet:///r/example/ability/device.dev-a.hub.join",
			adminAbilityHubLeave:         "easynet:///r/example/ability/device.dev-a.hub.leave",
			adminAbilityPairingPreflight: "easynet:///r/example/ability/device.dev-a.pairing.preflight",
			adminAbilityPairingValidate:  "easynet:///r/example/ability/device.dev-a.pairing.validate",
			adminAbilityCredentialVerify: "easynet:///r/example/ability/device.dev-a.credential.verify",
			adminAbilityPairingCreate:    "easynet:///r/example/ability/device.dev-a.pairing.create",
			adminAbilityRevokeDevice:     "easynet:///r/example/ability/device.dev-a.federation.revoke",
		},
		descriptorByAbility: map[string]string{
			"easynet:///r/example/ability/device.dev-a.agent.list":        "easynet:///r/example/ability/device.dev-a.agent.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.start":       "easynet:///r/example/ability/device.dev-a.agent.start@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.stop":        "easynet:///r/example/ability/device.dev-a.agent.stop@1.0.0",
			"easynet:///r/example/ability/device.dev-a.agent.refresh":     "easynet:///r/example/ability/device.dev-a.agent.refresh@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.list":      "easynet:///r/example/ability/device.dev-a.session.list@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.create":    "easynet:///r/example/ability/device.dev-a.session.create@1.0.0",
			"easynet:///r/example/ability/device.dev-a.session.delete":    "easynet:///r/example/ability/device.dev-a.session.delete@1.0.0",
			"easynet:///r/example/ability/device.dev-a.hub.join":          "easynet:///r/example/ability/device.dev-a.hub.join@1.0.0",
			"easynet:///r/example/ability/device.dev-a.hub.leave":         "easynet:///r/example/ability/device.dev-a.hub.leave@1.0.0",
			"easynet:///r/example/ability/device.dev-a.pairing.preflight": "easynet:///r/example/ability/device.dev-a.pairing.preflight@1.0.0",
			"easynet:///r/example/ability/device.dev-a.pairing.validate":  "easynet:///r/example/ability/device.dev-a.pairing.validate@1.0.0",
			"easynet:///r/example/ability/device.dev-a.credential.verify": "easynet:///r/example/ability/device.dev-a.credential.verify@1.0.0",
			"easynet:///r/example/ability/device.dev-a.pairing.create":    "easynet:///r/example/ability/device.dev-a.pairing.create@1.0.0",
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

const adminRuntimeJoinHubRawJSON = `{
	"ack": true,
	"device_ura": "easynet:///r/example/device/dev-a"
}`

const adminRuntimeLeaveHubRawJSON = `{
	"ack": true,
	"device_ura": "easynet:///r/example/device/dev-a"
}`

const adminRuntimePairingPreflightRawJSON = `{
	"state": "requires_pairing",
	"hub_ura": "easynet:///r/example/hub/main",
	"device_ura": "easynet:///r/example/device/dev-a",
	"pairing_required": true,
	"trust_ready": false,
	"scopes": ["invoke", "events"]
}`

const adminRuntimeCreatePairingRawJSON = `{
	"token_id": "pair-token-1",
	"token": "pair-token-value",
	"hub_ura": "easynet:///r/example/hub/main",
	"device_ura": "easynet:///r/example/device/dev-a",
	"state": "issued",
	"expires_unix_ms": 1893456000000,
	"scopes": ["invoke", "events"]
}`

const adminRuntimeValidatePairingRawJSON = `{
	"credential_id": "cred-dev-a",
	"device_ura": "easynet:///r/example/device/dev-a",
	"hub_ura": "easynet:///r/example/hub/main",
	"state": "active",
	"issued_unix_ms": 1767225600000,
	"expires_unix_ms": 1893456000000,
	"scopes": ["invoke", "events"]
}`

const adminRuntimeVerifyCredentialRawJSON = `{
	"verified": true,
	"credential_id": "cred-dev-a",
	"device_ura": "easynet:///r/example/device/dev-a",
	"hub_ura": "easynet:///r/example/hub/main",
	"method": "daemon-trust-store"
}`
