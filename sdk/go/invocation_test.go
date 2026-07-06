package easynet

import (
	"encoding/base64"
	"encoding/json"
	"testing"
)

func TestInvocationBuilderBuildsCompleteTuple(t *testing.T) {
	draft, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithMetadata(map[string]any{}).
		Build()
	if err != nil {
		t.Fatalf("Build: %v", err)
	}

	raw, err := json.Marshal(draft)
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	var value map[string]any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatalf("Unmarshal marshaled draft: %v", err)
	}
	if _, ok := value["args"]; !ok {
		t.Fatalf("args missing from marshaled draft: %s", raw)
	}
	if _, ok := value["arguments_base64"]; ok {
		t.Fatalf("arguments_base64 present with args: %s", raw)
	}
}

func TestNewInvocationNonceBase64ReturnsSixteenBytes(t *testing.T) {
	nonce, err := NewInvocationNonceBase64()
	if err != nil {
		t.Fatalf("NewInvocationNonceBase64: %v", err)
	}
	raw, err := base64.StdEncoding.DecodeString(nonce)
	if err != nil {
		t.Fatalf("decode nonce: %v", err)
	}
	if len(raw) != 16 {
		t.Fatalf("nonce length = %d, want 16", len(raw))
	}
}

func TestInvocationBuilderInspectDoesNotConsumeAndBuildConsumes(t *testing.T) {
	builder := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json")

	if _, err := builder.Inspect(); err != nil {
		t.Fatalf("Inspect: %v", err)
	}
	if _, err := builder.Inspect(); err != nil {
		t.Fatalf("second Inspect: %v", err)
	}
	if _, err := builder.Build(); err != nil {
		t.Fatalf("Build: %v", err)
	}
	if _, err := builder.Inspect(); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("Inspect after Build error = %v, want %s", err, ErrInvalidHandle)
	}
	if _, err := builder.Build(); !IsCode(err, ErrInvalidHandle) {
		t.Fatalf("Build after Build error = %v, want %s", err, ErrInvalidHandle)
	}
}

func TestInvocationDraftFromJSONDecodesFixtureShape(t *testing.T) {
	draft, err := NewInvocationDraftFromJSON([]byte(`{
		"caller_ura": "easynet:///r/example/agent/alice.sdk",
		"callee_ura": "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"subject_ura": "easynet:///r/example/device/dev-a",
		"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": {"form": "none"},
		"args": {},
		"content_type": "application/json",
		"metadata": {}
	}`))
	if err != nil {
		t.Fatalf("NewInvocationDraftFromJSON: %v", err)
	}
	if draft.CallerURA() != "easynet:///r/example/agent/alice.sdk" {
		t.Fatalf("caller = %q", draft.CallerURA())
	}
	if !draft.HasJSONArgs() {
		t.Fatalf("draft did not preserve JSON args carrier")
	}
}

func TestInvocationBuilderRejectsMissingTupleField(t *testing.T) {
	_, err := NewInvocationBuilder().
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		Build()
	if err == nil {
		t.Fatalf("Build succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationBuilderRejectsDualArgumentCarriers(t *testing.T) {
	_, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithArgumentsBase64("e30=").
		WithContentType("application/json").
		Build()
	if err == nil {
		t.Fatalf("Build succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationBuilderRejectsMalformedNonce(t *testing.T) {
	for _, nonce := range []string{"not base64", "AQIDBA=="} {
		_, err := NewInvocationBuilder().
			WithCallerURA("easynet:///r/example/agent/alice.sdk").
			WithCalleeURA("easynet:///r/example/device/dev-a").
			WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
			WithSubjectURA("easynet:///r/example/device/dev-a").
			WithNonceBase64(nonce).
			WithCausalContext(map[string]any{"form": "none"}).
			WithJSONArgs(map[string]any{}).
			WithContentType("application/json").
			Build()
		if err == nil {
			t.Fatalf("Build succeeded for nonce %q, want invalid argument", nonce)
		}
		if !IsCode(err, ErrInvalidArgument) {
			t.Fatalf("error code for nonce %q = %v, want %s", nonce, err, ErrInvalidArgument)
		}
	}
}

func TestInvocationBuilderRejectsMalformedRawPayload(t *testing.T) {
	_, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithArgumentsBase64("not base64").
		WithContentType("application/octet-stream").
		Build()
	if err == nil {
		t.Fatal("Build succeeded for malformed raw payload, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestInvocationDraftFromJSONRejectsUnknownField(t *testing.T) {
	_, err := NewInvocationDraftFromJSON([]byte(`{
		"caller_ura": "easynet:///r/example/agent/alice.sdk",
		"callee_ura": "easynet:///r/example/device/dev-a",
		"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
		"subject_ura": "easynet:///r/example/device/dev-a",
		"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
		"causal_context": {"form": "none"},
		"args": {},
		"content_type": "application/json",
		"unexpected": true
	}`))
	if err == nil {
		t.Fatalf("NewInvocationDraftFromJSON succeeded, want invalid argument")
	}
	if !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("error code = %v, want %s", err, ErrInvalidArgument)
	}
}
