package easynet

import (
	"context"
	"encoding/json"
	"fmt"
)

// DiscoveryTransport supplies Runtime Core feature discovery JSON.
//
// Implementations may use daemon local transport. Public backend code consumes
// this Go interface rather than Axon packages, generated protobufs, C ABI
// handles, or raw daemon sockets.
type DiscoveryTransport interface {
	FeatureDiscovery(ctx context.Context) ([]byte, error)
}

// DiscoveryTransportFunc adapts a function into a DiscoveryTransport.
type DiscoveryTransportFunc func(ctx context.Context) ([]byte, error)

func (f DiscoveryTransportFunc) FeatureDiscovery(ctx context.Context) ([]byte, error) {
	return f(ctx)
}

// Client is the Go Runtime Core facade root.
type Client struct {
	transport DiscoveryTransport
}

// NewClient creates a Go SDK client over a daemon discovery transport.
func NewClient(transport DiscoveryTransport) (*Client, error) {
	if transport == nil {
		return nil, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "sdk",
			Retry:   RetryNever,
			Message: "discovery transport is required",
		}
	}
	return &Client{transport: transport}, nil
}

// FeatureSet is the language-neutral SDK feature discovery DTO.
type FeatureSet struct {
	ABIVersion uint32            `json:"abi_version"`
	SDKVersion string            `json:"sdk_version"`
	Profiles   map[string]string `json:"profiles"`
	Symbols    map[string]bool   `json:"symbols"`
	AxonPB     bool              `json:"axon_pb"`
}

// Version returns the daemon SDK version facts in a small typed DTO.
func (f FeatureSet) Version() Version {
	return Version{
		ABIVersion: f.ABIVersion,
		SDKVersion: f.SDKVersion,
	}
}

// Version is the Runtime Core version compatibility DTO.
type Version struct {
	ABIVersion uint32
	SDKVersion string
}

// FeatureDiscovery reads and decodes daemon SDK feature discovery.
func (c *Client) FeatureDiscovery(ctx context.Context) (FeatureSet, error) {
	if c == nil || c.transport == nil {
		return FeatureSet{}, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "sdk",
			Retry:   RetryNever,
			Message: "client is not initialized",
		}
	}
	if ctx == nil {
		return FeatureSet{}, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "sdk",
			Retry:   RetryNever,
			Message: "context is required",
		}
	}
	raw, err := c.transport.FeatureDiscovery(ctx)
	if err != nil {
		return FeatureSet{}, &SDKError{
			Code:    ErrorTransport,
			Stage:   "transport",
			Retry:   RetrySafe,
			Message: "feature discovery transport failed",
			Cause:   err,
		}
	}
	var features FeatureSet
	if err := json.Unmarshal(raw, &features); err != nil {
		return FeatureSet{}, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "decode",
			Retry:   RetryNever,
			Message: fmt.Sprintf("decode feature discovery JSON: %v", err),
			Cause:   err,
		}
	}
	if features.ABIVersion == 0 {
		return FeatureSet{}, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "decode",
			Retry:   RetryNever,
			Message: "abi_version must be non-zero",
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

// RequireABI reads feature discovery and fails with VersionIncompatible when
// the daemon SDK ABI does not match the caller's expected ABI.
func (c *Client) RequireABI(ctx context.Context, expected uint32) (FeatureSet, error) {
	if expected == 0 {
		return FeatureSet{}, &SDKError{
			Code:    ErrorInvalidArgument,
			Stage:   "sdk",
			Retry:   RetryNever,
			Message: "expected ABI version must be non-zero",
		}
	}
	features, err := c.FeatureDiscovery(ctx)
	if err != nil {
		return FeatureSet{}, err
	}
	if features.ABIVersion != expected {
		return FeatureSet{}, &SDKError{
			Code:    ErrorVersionIncompatible,
			Stage:   "sdk",
			Retry:   RetryNever,
			Message: fmt.Sprintf("daemon ABI version %d does not match expected %d", features.ABIVersion, expected),
		}
	}
	return features, nil
}
