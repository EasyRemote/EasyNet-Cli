package easynet

import (
	"context"
	"fmt"
	"sync"
)

type profileCloseTransport interface {
	Close(context.Context) error
}

type profileClientLifecycle struct {
	mu     sync.Mutex
	closed bool
}

func (l *profileClientLifecycle) RequireOpen(ctx context.Context, profile string) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return invalidRuntimeClient(fmt.Sprintf("%s client is closed", profile))
	}
	return nil
}

func (l *profileClientLifecycle) Close(ctx context.Context, transport any, profile string) error {
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

	if closer, ok := transport.(profileCloseTransport); ok {
		if err := closer.Close(ctx); err != nil {
			return transportRuntimeError(fmt.Sprintf("%s close transport failed", profile), err)
		}
	}
	return nil
}
