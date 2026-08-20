package easynet

import (
	"context"
	"fmt"
	"sync"
)

type runtimeCloseTransport interface {
	Close(context.Context) error
}

type runtimeClientLifecycle struct {
	mu     sync.Mutex
	closed bool
}

func (l *runtimeClientLifecycle) RequireOpen(ctx context.Context, capability string) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return invalidRuntimeClient(fmt.Sprintf("%s client is closed", capability))
	}
	return nil
}

func (l *runtimeClientLifecycle) Close(ctx context.Context, transport any, capability string) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	l.mu.Lock()
	if l.closed {
		l.mu.Unlock()
		return nil
	}
	l.closed = true
	l.mu.Unlock()

	if closer, ok := transport.(runtimeCloseTransport); ok {
		if err := closer.Close(ctx); err != nil {
			return transportRuntimeError(fmt.Sprintf("%s close transport failed", capability), err)
		}
	}
	return nil
}
