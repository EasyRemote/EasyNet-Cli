package easynet

import (
	"context"
	"sync"
)

// NativeRuntimeOptions configures the SDK-owned native daemon Runtime provider.
// The Go facade keeps native library and handle details private; callers see
// only Runtime Core lifecycle concepts.
type NativeRuntimeOptions struct {
	LibraryPath     string
	ControlPath     string
	Endpoint        string
	DialTimeoutMS   int64
	InvokeTimeoutMS int64
	MaxMessageBytes int
}

// NativeRuntimeHandle owns one SDK RuntimeClient plus its native provider
// resources.
type NativeRuntimeHandle struct {
	mu      sync.Mutex
	client  *RuntimeClient
	closeFn func(context.Context) error
	closed  bool
}

func newNativeRuntimeHandle(client *RuntimeClient, closeFn func(context.Context) error) (*NativeRuntimeHandle, error) {
	if client == nil {
		return nil, invalidRuntimeClient("native runtime client is required")
	}
	if closeFn == nil {
		closeFn = func(context.Context) error { return nil }
	}
	return &NativeRuntimeHandle{client: client, closeFn: closeFn}, nil
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
	h.closeFn = nil
	h.mu.Unlock()

	var first error
	if client != nil {
		first = client.Close(ctx)
	}
	if closeFn != nil {
		if err := closeFn(ctx); err != nil && first == nil {
			first = err
		}
	}
	return first
}
