//go:build easynet_direct_runtime

package easynet

import (
	"context"
	"errors"
	"testing"
	"time"
)

type directRuntimeAbilityTransport struct {
	*DirectRuntimeTransport
	resolve func(context.Context, []byte) ([]byte, error)
}

func (t directRuntimeAbilityTransport) ResolveDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.resolve(ctx, requestJSON)
}

func TestRuntimeAbilityClientDeadlineIsProviderOwned(t *testing.T) {
	transport, daemon, cleanup := openDirectRuntimeTestTransportWithOptions(t, DirectRuntimeOptions{
		DialTimeoutMS:   3000,
		InvokeTimeoutMS: 50,
	})
	defer cleanup()
	daemon.invokeDelay = time.Second
	daemon.invokeStarted = make(chan struct{})

	runtime, err := NewRuntimeClient(directRuntimeAbilityTransport{
		DirectRuntimeTransport: transport,
		resolve:                testResolveDescriptorRef(t),
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	client, err := NewRuntimeAbilityClient(runtime, NewCanonicalAddressing())
	if err != nil {
		t.Fatalf("NewRuntimeAbilityClient: %v", err)
	}

	done := make(chan error, 1)
	go func() {
		_, err := client.Invoke(context.Background(), directRuntimeAbilityCallContext(), "er.weather", map[string]any{"city": "Singapore"})
		done <- err
	}()

	select {
	case <-daemon.invokeStarted:
	case <-time.After(time.Second):
		t.Fatalf("ability deadline test did not dispatch the runtime invocation")
	}

	err = <-done
	if !IsCode(err, ErrTimeout) {
		t.Fatalf("ability deadline = %v, want %s", err, ErrTimeout)
	}
	var sdkErr *SDKError
	if !errors.As(err, &sdkErr) {
		t.Fatalf("ability deadline error is not SDKError: %T", err)
	}
	if sdkErr.Stage != "direct_runtime" || sdkErr.Retry != RetrySafe || !sdkErr.Retryable {
		t.Fatalf("ability deadline classification = stage %q retry %s retryable %v", sdkErr.Stage, sdkErr.Retry, sdkErr.Retryable)
	}

	daemon.invokeDelay = 0
	daemon.invokeStarted = nil
	output, err := client.Invoke(context.Background(), directRuntimeAbilityCallContext(), "er.weather", map[string]any{"city": "Singapore"})
	if err != nil {
		t.Fatalf("retry ability Invoke after deadline cleanup: %v", err)
	}
	if output["ok"] != true {
		t.Fatalf("retry output = %#v", output)
	}
}

func directRuntimeAbilityCallContext() RuntimeCallContext {
	return RuntimeCallContext{
		CallerURA:     "easynet:///r/example/agent/alice",
		CalleeURA:     "easynet:///r/example/device/dev-a",
		SubjectURA:    "easynet:///r/example/device/dev-a",
		NonceBase64:   "AQIDBAUGBwgJCgsMDQ4PEA==",
		CausalContext: map[string]any{"form": "none"},
		Metadata:      map[string]any{"trace_id": "ability-deadline-test"},
	}
}
