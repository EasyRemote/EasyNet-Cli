package easynet

import (
	"context"
	"testing"
)

func TestNativeRuntimeHandleClosesClientAndProvider(t *testing.T) {
	health := nativeRuntimeTestHealth(t)
	transportClosed := false
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(context.Context, []byte) ([]byte, error) {
			return nil, nil
		},
		CloseFunc: func(context.Context) error {
			transportClosed = true
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	addressing := NewCanonicalAddressing()
	providerClosed := false
	handle, err := newNativeRuntimeHandleWithAddressing(runtime, health, addressing, func(context.Context) error {
		providerClosed = true
		return nil
	})
	if err != nil {
		t.Fatalf("newNativeRuntimeHandle: %v", err)
	}

	client, err := handle.Client()
	if err != nil {
		t.Fatalf("Client: %v", err)
	}
	if client != runtime {
		t.Fatalf("Client returned different runtime")
	}
	healthClient, err := handle.Health()
	if err != nil {
		t.Fatalf("Health: %v", err)
	}
	if healthClient != health {
		t.Fatalf("Health returned different client")
	}
	addressingProvider, err := handle.Addressing()
	if err != nil {
		t.Fatalf("Addressing: %v", err)
	}
	if addressingProvider != addressing {
		t.Fatalf("Addressing returned different provider")
	}
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transportClosed || !providerClosed {
		t.Fatalf("close did not release client/provider: client=%v provider=%v", transportClosed, providerClosed)
	}
	if _, err := handle.Client(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Client after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := handle.Health(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Health after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := handle.Addressing(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Addressing after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
}

func TestNativeRuntimeHandleAlwaysProvidesCanonicalAddressing(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(context.Context, []byte) ([]byte, error) { return nil, nil },
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	handle, err := newNativeRuntimeHandle(runtime, nativeRuntimeTestHealth(t), nil)
	if err != nil {
		t.Fatalf("newNativeRuntimeHandle: %v", err)
	}

	addressing, err := handle.Addressing()
	if err != nil {
		t.Fatalf("Addressing: %v", err)
	}
	projection, err := addressing.BuildDescriptorRef(
		context.Background(),
		CanonicalDescriptorRefBuildRequest{
			AbilityURA:        "easynet:///r/example/ability/device.dev-a.observe.health",
			DescriptorVersion: "1.0.0",
			DescriptorHash:    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			Action:            "invoke",
		},
	)
	if err != nil {
		t.Fatalf("BuildDescriptorRef: %v", err)
	}
	ref := projection.DescriptorRef
	if ref != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke" {
		t.Fatalf("descriptor_ref = %q", ref)
	}
}

func TestNativeRuntimeHandleProvidesRuntimeAbilityFacade(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc:               func(context.Context, []byte) ([]byte, error) { return nil, nil },
		ResolveDescriptorRefFunc: testResolveDescriptorRef(t),
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	handle, err := newNativeRuntimeHandle(runtime, nativeRuntimeTestHealth(t), nil)
	if err != nil {
		t.Fatalf("newNativeRuntimeHandle: %v", err)
	}

	ability, err := handle.AbilityClient()
	if err != nil {
		t.Fatalf("AbilityClient: %v", err)
	}
	draft, err := ability.Build(context.Background(), RuntimeCallContext{
		CallerURA:     "easynet:///r/example/agent/alice.client",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/device/dev-a",
		NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext: map[string]any{"form": "none"},
	}, "observe.health", map[string]any{"ready": true})
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if draft.DescriptorRef() != "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read" {
		t.Fatalf("descriptor_ref = %q", draft.DescriptorRef())
	}

	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if _, err := handle.AbilityClient(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("AbilityClient after close error = %v, want %s", err, ErrInvalidArgument)
	}
}

func TestNativeRuntimeHandleRequiresHealthFacade(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		InvokeFunc: func(context.Context, []byte) ([]byte, error) {
			return nil, nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	if _, err := newNativeRuntimeHandle(runtime, nil, nil); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("newNativeRuntimeHandle missing health = %v, want %s", err, ErrInvalidArgument)
	}
}

func nativeRuntimeTestHealth(t *testing.T) *HealthClient {
	t.Helper()
	health, err := NewHealthClient(HealthTransportFunc(func(context.Context) ([]byte, error) {
		return []byte(`{"api_ready":true,"invocation_ready":true,"directory_ready":true,"trust_ready":true,"runtime_ready":true,"diagnostics":[]}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}
	return health
}
