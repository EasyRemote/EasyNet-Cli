package easynet

import (
	"context"
	"errors"
	"sync"
)

// RuntimeProfileBundleOptions configures Runtime Core-backed profile clients.
//
// Optional providers keep daemon/Axon-owned projection semantics behind
// explicit SDK seams. They are not fallbacks and must not reimplement protocol
// canonicalization in product code.
type RuntimeProfileBundleOptions struct {
	ReceiptProjectionProvider ReceiptProjectionProvider
	PublicationLocalProvider  PublicationLocalProvider
	AdminGatewayStatus        AdminGatewayStatusProvider
	EventsProjectionProvider  EventsProjectionProvider
	OwnRuntime                bool
	OwnIdentity               bool
}

type runtimeProfileClosable interface {
	Close(context.Context) error
}

// RuntimeProfileBundle is the Go SDK root for Runtime Core-backed profiles.
//
// It keeps product integrations from assembling profile transports outside the
// SDK while preserving the existing RuntimeClient and IdentityClient boundary.
type RuntimeProfileBundle struct {
	mu       sync.Mutex
	runtime  *RuntimeClient
	identity *IdentityClient
	options  RuntimeProfileBundleOptions
	owned    []runtimeProfileClosable
	closed   bool
}

// NewRuntimeProfileBundle creates a profile root over an open RuntimeClient and
// an IdentityClient that delegates addressing/canonical helpers to the daemon.
func NewRuntimeProfileBundle(runtime *RuntimeClient, identity *IdentityClient, options RuntimeProfileBundleOptions) (*RuntimeProfileBundle, error) {
	if runtime == nil {
		return nil, invalidRuntimeClient("runtime client is required")
	}
	if identity == nil {
		return nil, invalidRuntimeClient("identity client is required")
	}
	return &RuntimeProfileBundle{
		runtime:  runtime,
		identity: identity,
		options:  options,
	}, nil
}

// OpenRuntimeProfiles opens a Runtime Core client from this daemon handle and
// returns a scoped profile root. The returned bundle owns the opened runtime
// client and closes it when the bundle closes.
func (h *DaemonHandle) OpenRuntimeProfiles(ctx context.Context, opts ConnectOptions, identity *IdentityClient, options RuntimeProfileBundleOptions) (*RuntimeProfileBundle, error) {
	runtime, err := h.OpenRuntime(ctx, opts)
	if err != nil {
		return nil, err
	}
	options.OwnRuntime = true
	bundle, err := NewRuntimeProfileBundle(runtime, identity, options)
	if err != nil {
		_ = runtime.Close(ctx)
		return nil, err
	}
	return bundle, nil
}

// Runtime returns the scoped Runtime Core facade.
func (b *RuntimeProfileBundle) Runtime(ctx context.Context) (*RuntimeClient, error) {
	runtime, _, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	return runtime, nil
}

// Identity returns the scoped Identity/Addressing facade.
func (b *RuntimeProfileBundle) Identity(ctx context.Context) (*IdentityClient, error) {
	_, identity, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	return identity, nil
}

// Directory opens a Runtime Core-backed Directory profile client.
func (b *RuntimeProfileBundle) Directory(ctx context.Context) (*DirectoryClient, error) {
	runtime, identity, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeDirectoryClient(runtime, identity)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Receipts opens a Runtime Core-backed Receipt profile client.
func (b *RuntimeProfileBundle) Receipts(ctx context.Context) (*ReceiptClient, error) {
	runtime, identity, options, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeReceiptClientWithProjectionProvider(runtime, identity, options.ReceiptProjectionProvider)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Publication opens a Runtime Core-backed Publication profile client.
func (b *RuntimeProfileBundle) Publication(ctx context.Context) (*PublicationClient, error) {
	runtime, identity, options, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimePublicationClientWithLocalProvider(runtime, identity, options.PublicationLocalProvider)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Admin opens a Runtime Core-backed Admin + Gateway profile client.
func (b *RuntimeProfileBundle) Admin(ctx context.Context) (*AdminClient, error) {
	runtime, identity, options, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeAdminClientWithGatewayStatus(runtime, identity, options.AdminGatewayStatus)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Events opens a Runtime Core-backed Events profile client.
func (b *RuntimeProfileBundle) Events(ctx context.Context) (*EventClient, error) {
	runtime, identity, options, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeEventClientWithProjectionProvider(runtime, identity, options.EventsProjectionProvider)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Surface opens a Runtime Core-backed Surface profile client.
func (b *RuntimeProfileBundle) Surface(ctx context.Context) (*SurfaceClient, error) {
	runtime, identity, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeSurfaceClient(runtime, identity)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Compatibility opens a Runtime Core-backed Compatibility profile client.
func (b *RuntimeProfileBundle) Compatibility(ctx context.Context) (*CompatibilityClient, error) {
	runtime, identity, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeCompatibilityClient(runtime, identity)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Wrappers opens a Runtime Core-backed Convenience Wrapper profile client.
func (b *RuntimeProfileBundle) Wrappers(ctx context.Context) (*WrapperClient, error) {
	runtime, identity, _, err := b.clients(ctx)
	if err != nil {
		return nil, err
	}
	client, err := NewRuntimeWrapperClient(runtime, identity)
	return trackRuntimeProfileClient(ctx, b, client, err)
}

// Close closes profile clients created by the bundle and then any root clients
// explicitly owned by the bundle. It does not stop the daemon process.
func (b *RuntimeProfileBundle) Close(ctx context.Context) error {
	if b == nil {
		return invalidRuntimeClient("runtime profile bundle is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return nil
	}
	b.closed = true
	owned := append([]runtimeProfileClosable(nil), b.owned...)
	runtime := b.runtime
	identity := b.identity
	options := b.options
	b.owned = nil
	b.runtime = nil
	b.identity = nil
	b.mu.Unlock()

	var closeErr error
	for i := len(owned) - 1; i >= 0; i-- {
		closeErr = errors.Join(closeErr, owned[i].Close(ctx))
	}
	if options.OwnIdentity && identity != nil {
		closeErr = errors.Join(closeErr, identity.Close(ctx))
	}
	if options.OwnRuntime && runtime != nil {
		closeErr = errors.Join(closeErr, runtime.Close(ctx))
	}
	return closeErr
}

func (b *RuntimeProfileBundle) clients(ctx context.Context) (*RuntimeClient, *IdentityClient, RuntimeProfileBundleOptions, error) {
	if b == nil {
		return nil, nil, RuntimeProfileBundleOptions{}, invalidRuntimeClient("runtime profile bundle is not initialized")
	}
	if ctx == nil {
		return nil, nil, RuntimeProfileBundleOptions{}, invalidRuntimeClient("context is required")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return nil, nil, RuntimeProfileBundleOptions{}, invalidRuntimeClient("runtime profile bundle is closed")
	}
	if b.runtime == nil {
		return nil, nil, RuntimeProfileBundleOptions{}, invalidRuntimeClient("runtime client is required")
	}
	if b.identity == nil {
		return nil, nil, RuntimeProfileBundleOptions{}, invalidRuntimeClient("identity client is required")
	}
	return b.runtime, b.identity, b.options, nil
}

func (b *RuntimeProfileBundle) track(ctx context.Context, client runtimeProfileClosable) error {
	if client == nil {
		return invalidRuntimeClient("profile client is required")
	}
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		_ = client.Close(ctx)
		return invalidRuntimeClient("runtime profile bundle is closed")
	}
	b.owned = append(b.owned, client)
	b.mu.Unlock()
	return nil
}

func trackRuntimeProfileClient[T runtimeProfileClosable](ctx context.Context, bundle *RuntimeProfileBundle, client T, err error) (T, error) {
	if err != nil {
		var zero T
		return zero, err
	}
	if err := bundle.track(ctx, client); err != nil {
		var zero T
		return zero, err
	}
	return client, nil
}
