package easynet

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
)

// DiscoveryTransport supplies Runtime Core feature discovery JSON.
//
// Implementations may use provider transport. Public backend code consumes
// this Go interface rather than Axon packages, generated protobufs, C ABI
// handles, or raw provider sockets.
type DiscoveryTransport interface {
	FeatureDiscovery(ctx context.Context) ([]byte, error)
	Close(ctx context.Context) error
}

// DiscoveryTransportFunc adapts a function into a DiscoveryTransport.
type DiscoveryTransportFunc func(ctx context.Context) ([]byte, error)

func (f DiscoveryTransportFunc) FeatureDiscovery(ctx context.Context) ([]byte, error) {
	return f(ctx)
}

func (f DiscoveryTransportFunc) Close(ctx context.Context) error {
	return nil
}

// Client is the Go Runtime Core facade root.
type Client struct {
	mu        sync.Mutex
	transport DiscoveryTransport
	closed    bool
}

// NewClient creates a Go SDK client over a runtime discovery transport.
func NewClient(transport DiscoveryTransport) (*Client, error) {
	if transport == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "discovery transport is required",
		}
	}
	return &Client{transport: transport}, nil
}

func (c *Client) discoveryTransport(ctx context.Context) (DiscoveryTransport, error) {
	if c == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "client is not initialized",
		}
	}
	if ctx == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "context is required",
		}
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "client is closed",
		}
	}
	if c.transport == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "client is not initialized",
		}
	}
	return c.transport, nil
}

// FeatureSet is the language-neutral SDK feature discovery DTO.
type FeatureSet struct {
	ABIVersion uint32            `json:"abi_version"`
	SDKVersion string            `json:"sdk_version"`
	Profiles   map[string]string `json:"profiles"`
	Symbols    map[string]bool   `json:"symbols"`
	AxonPB     bool              `json:"axon_pb"`
}

// Version returns SDK version facts in a small typed DTO.
func (f FeatureSet) Version() Version {
	return Version{
		ABIVersion: f.ABIVersion,
		SDKVersion: f.SDKVersion,
	}
}

// Version is the negotiated Runtime Core version DTO.
type Version struct {
	ABIVersion uint32
	SDKVersion string
}

// FeatureDiscovery reads and decodes SDK feature discovery.
func (c *Client) FeatureDiscovery(ctx context.Context) (FeatureSet, error) {
	transport, err := c.discoveryTransport(ctx)
	if err != nil {
		return FeatureSet{}, err
	}
	raw, err := transport.FeatureDiscovery(ctx)
	if err != nil {
		return FeatureSet{}, &SDKError{
			Code:      ErrTransport,
			Stage:     "transport",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   "feature discovery transport failed",
			Cause:     err,
		}
	}
	var features FeatureSet
	if err := json.Unmarshal(raw, &features); err != nil {
		return FeatureSet{}, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "decode",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   fmt.Sprintf("decode feature discovery JSON: %v", err),
			Cause:     err,
		}
	}
	if features.Profiles == nil {
		features.Profiles = map[string]string{}
	}
	if features.Symbols == nil {
		features.Symbols = map[string]bool{}
	}
	return features, nil
}

// Close releases the SDK discovery boundary without stopping the daemon.
func (c *Client) Close(ctx context.Context) error {
	if c == nil {
		return &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "client is not initialized",
		}
	}
	if ctx == nil {
		return &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "context is required",
		}
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	transport := c.transport
	c.closed = true
	c.transport = nil
	c.mu.Unlock()

	if transport == nil {
		return &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "client is not initialized",
		}
	}
	if err := transport.Close(ctx); err != nil {
		return &SDKError{
			Code:      ErrTransport,
			Stage:     "transport",
			Retry:     RetrySafe,
			Retryable: RetryableForHint(RetrySafe),
			Message:   "client close transport failed",
			Cause:     err,
		}
	}
	return nil
}

// RequireABI reads feature discovery and fails with VersionMismatch when
// the runtime SDK ABI does not match the caller's expected ABI.
func (c *Client) RequireABI(ctx context.Context, expected uint32) (FeatureSet, error) {
	if expected == 0 {
		return FeatureSet{}, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   "expected ABI version must be non-zero",
		}
	}
	features, err := c.FeatureDiscovery(ctx)
	if err != nil {
		return FeatureSet{}, err
	}
	if features.ABIVersion != expected {
		return FeatureSet{}, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "sdk",
			Retry:     RetryNever,
			Retryable: RetryableForHint(RetryNever),
			Message:   fmt.Sprintf("runtime ABI version %d does not match expected %d", features.ABIVersion, expected),
		}
	}
	return features, nil
}
