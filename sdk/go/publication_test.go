package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

type memoryPublicationTransport struct {
	resourceRefJSON         string
	packageValidationJSON   string
	deployResultJSON        string
	deployInvocationJSON    string
	pluginInstallJSON       string
	listJSON                string
	showJSON                string
	enableJSON              string
	disableJSON             string
	unpublishInvocationJSON string
	unpublishJSON           string
	seenRequest             map[string]any
	closeCalls              int
}

func (m *memoryPublicationTransport) remember(requestJSON []byte) {
	_ = json.Unmarshal(requestJSON, &m.seenRequest)
}

func (m *memoryPublicationTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.resourceRefJSON), nil
}

func (m *memoryPublicationTransport) ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.packageValidationJSON), nil
}

func (m *memoryPublicationTransport) DeployAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.deployResultJSON), nil
}

func (m *memoryPublicationTransport) BuildDeployInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.deployInvocationJSON), nil
}

func (m *memoryPublicationTransport) InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.pluginInstallJSON), nil
}

func (m *memoryPublicationTransport) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.listJSON), nil
}

func (m *memoryPublicationTransport) ShowAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.showJSON), nil
}

func (m *memoryPublicationTransport) EnableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.enableJSON), nil
}

func (m *memoryPublicationTransport) DisableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.disableJSON), nil
}

func (m *memoryPublicationTransport) BuildUnpublishInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.unpublishInvocationJSON), nil
}

func (m *memoryPublicationTransport) UnpublishAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	m.remember(requestJSON)
	return []byte(m.unpublishJSON), nil
}

func (m *memoryPublicationTransport) Close(ctx context.Context) error {
	m.closeCalls++
	return nil
}

func newMemoryPublicationTransport() *memoryPublicationTransport {
	return &memoryPublicationTransport{
		resourceRefJSON:         resourceRefFixtureJSON,
		packageValidationJSON:   packageValidationFixtureJSON,
		deployResultJSON:        `{"public_name":"weather","namespace":"er","ability_ura":"easynet:///r/example/ability/device.dev-a.er.weather","node_id":"local","install_id":"install-1","state":"enabled"}`,
		deployInvocationJSON:    deployInvocationFixtureJSON,
		pluginInstallJSON:       `{"profile":"publication","kind":"plugin_install","source":"file:///tmp/plugin","install_id":"install-1","status":"installed","metadata":{}}`,
		listJSON:                `{"profile":"publication","kind":"published_ability_page","item_kind":"published_ability","items":[{"descriptor":{"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0","descriptor_version":"1.0.0","schema_hash":"sha256:abc","owner_ura":"easynet:///r/example/device/dev-a"},"implementation":{"impl_id":"impl-1","impl_hash":"sha256:def","runtime_env":"python","enabled":true},"metadata":{}}],"next_cursor":null,"limit":50,"source":"read_model","metadata":{}}`,
		showJSON:                `{"descriptor":{"descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0","descriptor_version":"1.0.0","schema_hash":"sha256:abc","owner_ura":"easynet:///r/example/device/dev-a"},"implementation":{"impl_id":"impl-1","impl_hash":"sha256:def","runtime_env":"python","enabled":true},"metadata":{}}`,
		enableJSON:              `{"profile":"publication","kind":"ability_impl_enabled","metadata":{}}`,
		disableJSON:             `{"profile":"publication","kind":"ability_impl_disabled","metadata":{}}`,
		unpublishInvocationJSON: unpublishInvocationFixtureJSON,
		unpublishJSON:           `{"profile":"publication","kind":"ability_unpublished","descriptor_ref":"easynet:///r/example/ability/device.dev-a.er.weather@1.0.0","metadata":{}}`,
	}
}

func baseAbilityDeployRequest() AbilityDeployRequest {
	return AbilityDeployRequest{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		ResourceRef: ResourceRef{
			ResourceURA:   "easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
			OwnerURA:      "easynet:///r/example/device/dev-a",
			Namespace:     "fs",
			DisplayPath:   "tmp/easynet-weather-package",
			Capability:    "read",
			ExpiresUnixMS: 4102444800000,
			Revision:      "fs-local-mapping-v1",
		},
		NodeID:   "local",
		Metadata: map[string]any{"request_id": "publication-deploy-1"},
	}
}

func basePublishedAbilityQuery() PublishedAbilityQuery {
	return PublishedAbilityQuery{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
	}
}

func baseShowAbilityRequest() ShowAbilityRequest {
	return ShowAbilityRequest{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		DescriptorRef:     "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
	}
}

func baseUnpublishRequest() UnpublishAbilityRequest {
	return UnpublishAbilityRequest{
		CallerURA:         "easynet:///r/example/agent/alice.sdk",
		CalleeURA:         "easynet:///r/example/device/dev-a",
		SubjectURA:        "easynet:///r/example/device/dev-a",
		DescriptorVersion: "1.0.0",
		NonceBase64:       "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext:     map[string]any{"form": "none"},
		AbilityURA:        "easynet:///r/example/ability/device.dev-a.er.weather",
	}
}

func TestPublicationBuildResourceRefAndValidatePackage(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	ref, err := client.BuildLocalResourceRef(context.Background(), LocalResourceRefRequest{Path: "/tmp/easynet-weather-package", Capability: "read"})
	if err != nil {
		t.Fatalf("BuildLocalResourceRef: %v", err)
	}
	if ref.ResourceURA == "" || ref.OwnerURA == "" {
		t.Fatalf("unexpected resource ref: %#v", ref)
	}

	validation, err := client.ValidatePackage(context.Background(), "/tmp/easynet-weather-package", ValidatePackageOptions{})
	if err != nil {
		t.Fatalf("ValidatePackage: %v", err)
	}
	if !validation.Valid || validation.Manifest.WireKey != "er.weather" {
		t.Fatalf("unexpected validation: %#v", validation)
	}
	if transport.seenRequest["package_path"] != "/tmp/easynet-weather-package" {
		t.Fatalf("package path not forwarded: %#v", transport.seenRequest)
	}
}

func TestPublicationDeployAndBuildDeployInvocation(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	result, err := client.DeployAbility(context.Background(), baseAbilityDeployRequest())
	if err != nil {
		t.Fatalf("DeployAbility: %v", err)
	}
	if result.AbilityURA == "" || result.State != "enabled" {
		t.Fatalf("unexpected deploy result: %#v", result)
	}

	draft, err := client.BuildDeployInvocation(context.Background(), baseAbilityDeployRequest())
	if err != nil {
		t.Fatalf("BuildDeployInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0" ||
		draft.CallerURA() == "" || draft.SubjectURA() == "" || !draft.HasJSONArgs() {
		t.Fatalf("deploy invocation lost tuple fields: %#v", draft)
	}
	args, ok := draft.JSONArgs().(map[string]any)
	if !ok || args["node_id"] != "local" {
		t.Fatalf("deploy args not preserved: %#v", draft.JSONArgs())
	}
}

func TestPublicationRejectsIncompleteDeployCarrier(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	req := baseAbilityDeployRequest()
	req.SubjectURA = ""
	if _, err := client.BuildDeployInvocation(context.Background(), req); err == nil {
		t.Fatalf("incomplete deploy carrier accepted")
	}
}

func TestPublicationListShowEnableDisableAndUnpublish(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	page, err := client.ListAbilities(context.Background(), basePublishedAbilityQuery())
	if err != nil {
		t.Fatalf("ListAbilities: %v", err)
	}
	if page.Limit != DefaultPublishedAbilityPageSize || len(page.Items) != 1 {
		t.Fatalf("unexpected page: %#v", page)
	}
	if transport.seenRequest["limit"] != float64(DefaultPublishedAbilityPageSize) {
		t.Fatalf("default limit not forwarded: %#v", transport.seenRequest)
	}

	ability, err := client.ShowAbility(context.Background(), DescriptorRef("easynet:///r/example/ability/device.dev-a.er.weather@1.0.0"))
	if err != nil {
		t.Fatalf("ShowAbility: %v", err)
	}
	if ability.Descriptor["descriptor_version"] != "1.0.0" {
		t.Fatalf("unexpected ability: %#v", ability)
	}

	id := AbilityImplID{ImplID: "impl-1", AbilityURA: "easynet:///r/example/ability/device.dev-a.er.weather"}
	if err := client.EnableAbilityImpl(context.Background(), id); err != nil {
		t.Fatalf("EnableAbilityImpl: %v", err)
	}
	if err := client.DisableAbilityImpl(context.Background(), id); err != nil {
		t.Fatalf("DisableAbilityImpl: %v", err)
	}
	ability, err = client.ShowAbilityWithRequest(context.Background(), baseShowAbilityRequest())
	if err != nil {
		t.Fatalf("ShowAbilityWithRequest: %v", err)
	}
	if descriptor := ability.Descriptor["descriptor_ref"]; descriptor != "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0" {
		t.Fatalf("show with request descriptor = %#v", descriptor)
	}
	if err := client.UnpublishAbility(context.Background(), DescriptorRef("easynet:///r/example/ability/device.dev-a.er.weather@1.0.0")); err != nil {
		t.Fatalf("UnpublishAbility: %v", err)
	}
	record, err := client.UnpublishAbilityWithRequest(context.Background(), baseUnpublishRequest())
	if err != nil {
		t.Fatalf("UnpublishAbilityWithRequest: %v", err)
	}
	if record.Kind != "ability_unpublished" {
		t.Fatalf("unpublish record = %#v", record)
	}
}

func TestPublicationBuildUnpublishInvocation(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	draft, err := client.BuildUnpublishInvocation(context.Background(), baseUnpublishRequest())
	if err != nil {
		t.Fatalf("BuildUnpublishInvocation: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0" ||
		draft.CalleeURA() == "" || !draft.HasJSONArgs() {
		t.Fatalf("unpublish invocation lost tuple fields: %#v", draft)
	}

	req := baseUnpublishRequest()
	req.AbilityURA = ""
	if _, err := client.BuildUnpublishInvocation(context.Background(), req); err == nil {
		t.Fatalf("unpublish carrier without ability_ura accepted")
	}
}

func TestPublicationClientCloseDelegatesOnceAndFailsClosed(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := client.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if transport.closeCalls != 1 {
		t.Fatalf("close calls = %d, want 1", transport.closeCalls)
	}
	_, err = client.BuildLocalResourceRef(context.Background(), LocalResourceRefRequest{Path: "/tmp/easynet-weather-package", Capability: "read"})
	if err == nil {
		t.Fatalf("BuildLocalResourceRef after close succeeded")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called after close: %#v", transport.seenRequest)
	}
}

func TestPublicationProfileErrorsIncludeSourceRefs(t *testing.T) {
	transport := newMemoryPublicationTransport()
	client, err := NewPublicationClient(transport)
	if err != nil {
		t.Fatalf("NewPublicationClient: %v", err)
	}

	_, err = client.BuildLocalResourceRef(context.Background(), LocalResourceRefRequest{Path: "relative", Capability: "read"})
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
	details := sdkErrorDetails(t, err)
	if details["profile"] != publicationProfile {
		t.Fatalf("profile detail = %#v, want %s", details["profile"], publicationProfile)
	}
	if details["source_ref"] != "go_sdk.profile.publication" {
		t.Fatalf("source_ref detail = %#v", details["source_ref"])
	}
	if transport.seenRequest != nil {
		t.Fatalf("transport called for invalid request: %#v", transport.seenRequest)
	}
}

func TestPublicationTransportSDKErrorGetsProfileSourceRefs(t *testing.T) {
	err := wrapPublicationTransportError("publication failed", &SDKError{
		Code:    ErrTimeout,
		Stage:   "transport",
		Retry:   RetrySafe,
		Message: "deadline elapsed",
		Details: map[string]any{"reason": "deadline"},
	})

	if !IsCode(err, ErrTimeout) {
		t.Fatalf("error code = %v, want %s", err, ErrTimeout)
	}
	details := sdkErrorDetails(t, err)
	if details["reason"] != "deadline" {
		t.Fatalf("reason detail not preserved: %#v", details)
	}
	if details["profile"] != publicationProfile {
		t.Fatalf("profile detail = %#v, want %s", details["profile"], publicationProfile)
	}
	if details["source_ref"] != "go_sdk.profile.publication" {
		t.Fatalf("source_ref detail = %#v", details["source_ref"])
	}
}

func sdkErrorDetails(t *testing.T, err error) map[string]any {
	t.Helper()
	sdkErr, ok := err.(*SDKError)
	if !ok {
		t.Fatalf("err = %T, want *SDKError", err)
	}
	if sdkErr.Details == nil {
		t.Fatalf("SDKError.Details is nil")
	}
	return sdkErr.Details
}

const resourceRefFixtureJSON = `{
  "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
  "owner_ura": "easynet:///r/example/device/dev-a",
  "namespace": "fs",
  "display_path": "tmp/easynet-weather-package",
  "capability": "read",
  "expires_unix_ms": 4102444800000,
  "revision": "fs-local-mapping-v1"
}`

const packageValidationFixtureJSON = `{
  "profile": "publication",
  "kind": "package_validation",
  "valid": true,
  "package_path": "/tmp/easynet-weather-package",
  "manifest_path": "/tmp/easynet-weather-package/ability.json",
  "manifest_hash": "sha256:09c6bb09967428f12db1c5f0d0ae726c448dabf01bf7cea8476f4eabdf613bd1",
  "manifest": {
    "name": "weather",
    "namespace": "er",
    "wire_key": "er.weather",
    "descriptor_version": "1.0.0",
    "description": "Weather stream",
    "exec_kind": "host_stream",
    "timeout_seconds": null,
    "input_schema": {"type": "object", "properties": {}},
    "output_schema": null
  },
  "errors": [],
  "metadata": {"profile": "publication", "frame_contract_owner": "daemon_sdk"}
}`

const deployInvocationFixtureJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.ability.deploy@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {
    "resource_ref": {
      "resource_ura": "easynet:///r/example/resource/device.dev-a/fs/tmp/easynet-weather-package",
      "owner_ura": "easynet:///r/example/device/dev-a",
      "namespace": "fs",
      "display_path": "tmp/easynet-weather-package",
      "capability": "read",
      "expires_unix_ms": 4102444800000,
      "revision": "fs-local-mapping-v1"
    },
    "node_id": "local"
  },
  "content_type": "application/json",
  "metadata": {
    "request_id": "publication-deploy-1",
    "profile": "publication",
    "system_ability": "ability.deploy",
    "carrier_owner": "daemon_sdk"
  }
}`

const unpublishInvocationFixtureJSON = `{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.ability.unpublish@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {"ability_ura": "easynet:///r/example/ability/device.dev-a.er.weather"},
  "content_type": "application/json",
  "metadata": {
    "profile": "publication",
    "system_ability": "ability.unpublish",
    "carrier_owner": "daemon_sdk"
  }
}`
