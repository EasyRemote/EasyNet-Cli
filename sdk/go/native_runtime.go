package easynet

import (
	"context"
	"sync"
)

// NativeRuntimeOptions configures the SDK-owned native runtime provider.
// The Go facade keeps native library and handle details private; callers see
// only Runtime Core lifecycle concepts.
type NativeRuntimeOptions struct {
	LibraryPath     string
	ControlPath     string
	Endpoint        string
	DialTimeoutMS   int64
	InvokeTimeoutMS int64
	MaxMessageBytes int
	// Signer decorates direct Invoke/OpenStream/OpenBidi runtime calls with a
	// caller_signature when the draft does not already carry one. Prepare and
	// SubmitSigned remain unchanged because their signatures are user/browser
	// supplied by design.
	Signer *Signer
	// StartConfig requests an explicit runtime lifecycle start/adopt operation
	// before opening the runtime. When nil, OpenNativeRuntime only discovers
	// and attaches to an existing daemon.
	StartConfig *StartConfig
	// StopDaemonOnClose stops the lifecycle handle opened from StartConfig when
	// the native runtime handle is closed. When false, the runtime host is detached
	// after the runtime transport opens and keeps running.
	StopDaemonOnClose bool
}

// NativeRuntimeHandle owns SDK facades opened from one native runtime provider.
type NativeRuntimeHandle struct {
	mu         sync.Mutex
	client     *RuntimeClient
	health     *HealthClient
	addressing Addressing
	closeFn    func(context.Context) error
	closed     bool
}

func newNativeRuntimeHandle(client *RuntimeClient, health *HealthClient, closeFn func(context.Context) error) (*NativeRuntimeHandle, error) {
	return newNativeRuntimeHandleWithAddressing(client, health, NewCanonicalAddressing(), closeFn)
}

func newNativeRuntimeHandleWithAddressing(client *RuntimeClient, health *HealthClient, addressing Addressing, closeFn func(context.Context) error) (*NativeRuntimeHandle, error) {
	if client == nil {
		return nil, invalidRuntimeClient("native runtime client is required")
	}
	if health == nil {
		return nil, invalidRuntimeClient("native runtime health client is required")
	}
	if addressing == nil {
		return nil, invalidRuntimeClient("native runtime addressing provider is required")
	}
	if closeFn == nil {
		closeFn = func(context.Context) error { return nil }
	}
	return &NativeRuntimeHandle{
		client:     client,
		health:     health,
		addressing: addressing,
		closeFn:    closeFn,
	}, nil
}

// Client returns the Runtime Core facade opened by this native provider.
func (h *NativeRuntimeHandle) Client() (*RuntimeClient, error) {
	if h == nil {
		return nil, invalidRuntimeClient("native runtime handle is not initialized")
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return nil, invalidRuntimeClient("native runtime handle is closed")
	}
	if h.client == nil {
		return nil, invalidRuntimeClient("native runtime client is not initialized")
	}
	return h.client, nil
}

// Health returns the Health facade opened from the same native provider.
func (h *NativeRuntimeHandle) Health() (*HealthClient, error) {
	if h == nil {
		return nil, invalidRuntimeClient("native runtime handle is not initialized")
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return nil, invalidRuntimeClient("native runtime handle is closed")
	}
	if h.health == nil {
		return nil, invalidRuntimeClient("native runtime health client is not initialized")
	}
	return h.health, nil
}

// Addressing returns the canonical Axon-backed Addressing seam owned by this
// native provider. It is always present for every open native Runtime handle.
func (h *NativeRuntimeHandle) Addressing() (Addressing, error) {
	if h == nil {
		return nil, invalidRuntimeClient("native runtime handle is not initialized")
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return nil, invalidRuntimeClient("native runtime handle is closed")
	}
	if h.addressing == nil {
		return nil, invalidRuntimeClient("native runtime addressing provider is not initialized")
	}
	return h.addressing, nil
}

// AbilityClient returns a generic runtime ability facade backed by this native
// Runtime provider. The handle keeps ownership of the Runtime and Addressing
// facades; callers close the NativeRuntimeHandle, not this borrowed facade.
func (h *NativeRuntimeHandle) AbilityClient() (*RuntimeAbilityClient, error) {
	runtime, err := h.Client()
	if err != nil {
		return nil, err
	}
	addressing, err := h.Addressing()
	if err != nil {
		return nil, err
	}
	return NewRuntimeAbilityClient(runtime, addressing)
}

// Close releases the RuntimeClient and native provider resources exactly once.
func (h *NativeRuntimeHandle) Close(ctx context.Context) error {
	if h == nil {
		return nil
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	h.mu.Lock()
	if h.closed {
		h.mu.Unlock()
		return nil
	}
	h.closed = true
	client := h.client
	closeFn := h.closeFn
	h.client = nil
	h.health = nil
	h.addressing = nil
	h.closeFn = nil
	h.mu.Unlock()

	var first error
	if client != nil {
		if err := client.Close(ctx); err != nil && first == nil {
			first = err
		}
	}
	if closeFn != nil {
		if err := closeFn(ctx); err != nil && first == nil {
			first = err
		}
	}
	return first
}
