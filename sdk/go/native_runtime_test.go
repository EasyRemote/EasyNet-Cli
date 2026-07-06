package easynet

import (
	"context"
	"testing"
)

func TestNativeRuntimeHandleClosesClientAndProvider(t *testing.T) {
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
	providerClosed := false
	handle, err := newNativeRuntimeHandle(runtime, func(context.Context) error {
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
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if !transportClosed || !providerClosed {
		t.Fatalf("close did not release client/provider: client=%v provider=%v", transportClosed, providerClosed)
	}
	if _, err := handle.Client(); !IsCode(err, ErrInvalidArgument) {
		t.Fatalf("Client after close error = %v, want %s", err, ErrInvalidArgument)
	}
	if err := handle.Close(context.Background()); err != nil {
		t.Fatalf("second Close: %v", err)
	}
}
