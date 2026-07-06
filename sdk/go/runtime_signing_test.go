package easynet

import (
	"context"
	"encoding/json"
	"testing"
)

func TestRuntimeSigningTransportSignsUnsignedInvokeDraft(t *testing.T) {
	provider := &memorySignatureProvider{}
	signer, err := NewSigner(signerHandle(""), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}

	var seen map[string]any
	transport, err := NewRuntimeSigningTransport(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seen); err != nil {
				t.Fatalf("decode signed draft: %v", err)
			}
			return []byte(`{
				"ok": true,
				"tuple": {
					"caller_ura": "easynet:///r/example/agent/alice.sdk",
					"callee_ura": "easynet:///r/example/device/dev-a",
					"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
					"subject_ura": "easynet:///r/example/device/dev-a",
					"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
					"causal_context": {"form": "none"},
					"args": {},
					"content_type": "application/json"
				},
				"terminal_state": "Completed",
				"output_content_type": "application/json",
				"output_base64": "e30=",
				"output_json": {},
				"elapsed_ms": 1,
				"receipt": null,
				"error": null
			}`), nil
		},
	}, signer)
	if err != nil {
		t.Fatalf("NewRuntimeSigningTransport: %v", err)
	}

	result, err := NewRuntimeClientMust(t, transport).Invoke(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if !result.OK() {
		t.Fatalf("result not ok: %#v", result)
	}
	signature, ok := seen["caller_signature"].(map[string]any)
	if !ok {
		t.Fatalf("caller_signature missing from forwarded draft: %#v", seen)
	}
	if signature["algorithm"] != "ed25519" || signature["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("unexpected signature: %#v", signature)
	}
	if provider.material.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0" {
		t.Fatalf("provider material descriptor = %q", provider.material.DescriptorRef())
	}
	if provider.material.CanonicalBytesBase64() == "" {
		t.Fatal("provider did not receive canonical bytes")
	}
}

func TestRuntimeSigningTransportPreservesPresignedDraft(t *testing.T) {
	provider := &memorySignatureProvider{}
	signer, err := NewSigner(signerHandle(""), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}
	presigned, err := NewInvocationBuilder().
		WithCallerURA("easynet:///r/example/agent/alice.sdk").
		WithCalleeURA("easynet:///r/example/device/dev-a").
		WithDescriptorRef("easynet:///r/example/ability/device.dev-a.observe.health@1.0.0").
		WithSubjectURA("easynet:///r/example/device/dev-a").
		WithNonceBase64("AQIDBAUGBwgJCgsMDQ4PEA==").
		WithCausalContext(map[string]any{"form": "none"}).
		WithJSONArgs(map[string]any{}).
		WithContentType("application/json").
		WithCallerSignature(InvocationSignature{Algorithm: "ed25519", SignatureBase64: "cHJlc2lnbmVk", KeyIDHint: "browser-key"}).
		Build()
	if err != nil {
		t.Fatalf("Build presigned: %v", err)
	}

	var seen map[string]any
	transport, err := NewRuntimeSigningTransport(RuntimeTransportFunc{
		InvokeFunc: func(ctx context.Context, draftJSON []byte) ([]byte, error) {
			if err := json.Unmarshal(draftJSON, &seen); err != nil {
				t.Fatalf("decode signed draft: %v", err)
			}
			return []byte(`{
				"ok": true,
				"tuple": {
					"caller_ura": "easynet:///r/example/agent/alice.sdk",
					"callee_ura": "easynet:///r/example/device/dev-a",
					"descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
					"subject_ura": "easynet:///r/example/device/dev-a",
					"nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
					"causal_context": {"form": "none"},
					"args": {},
					"content_type": "application/json"
				},
				"terminal_state": "Completed",
				"output_content_type": "application/json",
				"output_base64": "e30=",
				"output_json": {},
				"elapsed_ms": 1,
				"receipt": null,
				"error": null
			}`), nil
		},
	}, signer)
	if err != nil {
		t.Fatalf("NewRuntimeSigningTransport: %v", err)
	}

	if _, err := NewRuntimeClientMust(t, transport).Invoke(context.Background(), presigned); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	signature := seen["caller_signature"].(map[string]any)
	if signature["signature_base64"] != "cHJlc2lnbmVk" || signature["key_id_hint"] != "browser-key" {
		t.Fatalf("presigned signature not preserved: %#v", signature)
	}
	if provider.material.CanonicalBytesBase64() != "" {
		t.Fatal("provider was called for a presigned draft")
	}
}

func TestRuntimeSigningTransportSignsStreamAndBidiDrafts(t *testing.T) {
	provider := &memorySignatureProvider{}
	signer, err := NewSigner(signerHandle(""), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}

	var streamDraft map[string]any
	var bidiDraft map[string]any
	transport, err := NewRuntimeSigningTransport(RuntimeTransportFunc{
		OpenStreamFunc: func(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &streamDraft); err != nil {
				t.Fatalf("decode signed stream draft: %v", err)
			}
			return StreamTransportFunc{}, []byte(`{"stream_id":"stream-1","state":"Opening"}`), nil
		},
		OpenBidiFunc: func(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
			if err := json.Unmarshal(draftJSON, &bidiDraft); err != nil {
				t.Fatalf("decode signed bidi draft: %v", err)
			}
			return BidiTransportFunc{}, []byte(`{"session_id":"bidi-1","state":"Opening"}`), nil
		},
	}, signer)
	if err != nil {
		t.Fatalf("NewRuntimeSigningTransport: %v", err)
	}
	client := NewRuntimeClientMust(t, transport)

	stream, err := client.InvokeStream(context.Background(), completeDraftForRuntimeTest(t))
	if err != nil {
		t.Fatalf("InvokeStream: %v", err)
	}
	if stream.StreamID() != "stream-1" {
		t.Fatalf("stream id = %q", stream.StreamID())
	}
	assertRuntimeSigningSignature(t, streamDraft)

	session, err := client.OpenBidi(context.Background(), completeDraftForRuntimeTest(t), []BidiStreamDescriptor{
		{StreamID: 1, ContentType: "application/json", Ordering: "ordered"},
	})
	if err != nil {
		t.Fatalf("OpenBidi: %v", err)
	}
	if session.SessionID() != "bidi-1" {
		t.Fatalf("session id = %q", session.SessionID())
	}
	assertRuntimeSigningSignature(t, bidiDraft)
}

func TestRuntimeSigningTransportRejectsInvalidConstruction(t *testing.T) {
	provider := &memorySignatureProvider{}
	signer, err := NewSigner(signerHandle(""), provider)
	if err != nil {
		t.Fatalf("NewSigner: %v", err)
	}
	if _, err := NewRuntimeSigningTransport(nil, signer); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("NewRuntimeSigningTransport(nil) = %v, want %s", err, ErrInvalidArgument)
	}
	var transport *RuntimeSigningTransport
	if _, err := transport.Invoke(context.Background(), []byte(`{}`)); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("nil RuntimeSigningTransport.Invoke = %v, want %s", err, ErrInvalidArgument)
	}
}

func NewRuntimeClientMust(t *testing.T, transport RuntimeTransport) *RuntimeClient {
	t.Helper()
	client, err := NewRuntimeClient(transport)
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	return client
}

func assertRuntimeSigningSignature(t *testing.T, draft map[string]any) {
	t.Helper()
	signature, ok := draft["caller_signature"].(map[string]any)
	if !ok {
		t.Fatalf("caller_signature missing from forwarded draft: %#v", draft)
	}
	if signature["algorithm"] != "ed25519" || signature["signature_base64"] != "c2lnbmF0dXJl" {
		t.Fatalf("unexpected signature: %#v", signature)
	}
}
