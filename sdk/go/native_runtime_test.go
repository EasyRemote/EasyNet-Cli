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
	identityTransportClosed := false
	identity, err := NewIdentityClient(nativeRuntimeIdentityTransport{closed: &identityTransportClosed})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	providerClosed := false
	handle, err := newNativeRuntimeHandleWithIdentity(runtime, health, identity, func(context.Context) error {
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
	identityClient, err := handle.Identity()
	if err != nil {
		t.Fatalf("Identity: %v", err)
	}
	if identityClient != identity {
		t.Fatalf("Identity returned different client")
	}
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transportClosed || !identityTransportClosed || !providerClosed {
		t.Fatalf("close did not release client/identity/provider: client=%v identity=%v provider=%v", transportClosed, identityTransportClosed, providerClosed)
	}
	if _, err := handle.Client(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Client after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := handle.Health(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Health after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if _, err := handle.Identity(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Identity after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
}

type nativeRuntimeIdentityTransport struct {
	IdentityTransportFunc
	closed *bool
}

func (t nativeRuntimeIdentityTransport) Close(context.Context) error {
	*t.closed = true
	return nil
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
		return []byte(`{"api_ready":true,"daemon_ready":true,"invocation_ready":true,"directory_ready":true,"trust_ready":true,"runtime_ready":true,"diagnostics":[]}`), nil
	}))
	if err != nil {
		t.Fatalf("NewHealthClient: %v", err)
	}
	return health
}
