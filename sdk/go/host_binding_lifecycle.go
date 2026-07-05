package easynet

import (
	"context"
	"errors"
	"sync"
)

// HostStreamLifecycleState is the SDK-owned endpoint lifecycle state.
type HostStreamLifecycleState string

const (
	HostStreamLifecycleDeclared HostStreamLifecycleState = "declared"
	HostStreamLifecycleChecking HostStreamLifecycleState = "checking"
	HostStreamLifecycleReady    HostStreamLifecycleState = "ready"
	HostStreamLifecycleNotReady HostStreamLifecycleState = "not_ready"
	HostStreamLifecycleCleaning HostStreamLifecycleState = "cleaning"
	HostStreamLifecycleCleaned  HostStreamLifecycleState = "cleaned"
	HostStreamLifecycleFailed   HostStreamLifecycleState = "failed"
	HostStreamLifecycleClosed   HostStreamLifecycleState = "closed"
)

// HostStreamCleanup is the typed cleanup contract for a host-stream binding.
type HostStreamCleanup struct {
	Mode     string         `json:"mode,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

func NewHostStreamCleanupFromMap(value map[string]any) HostStreamCleanup {
	cleanup := HostStreamCleanup{Metadata: mapWithoutKeys(value, "mode")}
	if mode, ok := value["mode"].(string); ok {
		cleanup.Mode = mode
	}
	return cleanup
}

func (c HostStreamCleanup) Map() map[string]any {
	out := copyMap(c.Metadata)
	if out == nil {
		out = map[string]any{}
	}
	if c.Mode != "" {
		out["mode"] = c.Mode
	}
	return out
}

// HostStreamReadiness is the typed endpoint readiness projection.
type HostStreamReadiness struct {
	State         string         `json:"state,omitempty"`
	Checked       bool           `json:"checked"`
	EndpointReady *bool          `json:"endpoint_ready"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

func NewHostStreamReadinessFromMap(value map[string]any) HostStreamReadiness {
	readiness := HostStreamReadiness{
		Metadata: mapWithoutKeys(value, "state", "checked", "endpoint_ready"),
	}
	if state, ok := value["state"].(string); ok {
		readiness.State = state
	}
	if checked, ok := value["checked"].(bool); ok {
		readiness.Checked = checked
	}
	if endpointReady, ok := value["endpoint_ready"].(bool); ok {
		readiness.EndpointReady = &endpointReady
	}
	return readiness
}

func (r HostStreamReadiness) Map() map[string]any {
	out := copyMap(r.Metadata)
	if out == nil {
		out = map[string]any{}
	}
	if r.State != "" {
		out["state"] = r.State
	}
	out["checked"] = r.Checked
	if r.EndpointReady == nil {
		out["endpoint_ready"] = nil
	} else {
		out["endpoint_ready"] = *r.EndpointReady
	}
	return out
}

// HostStreamLifecycleProvider supplies generic endpoint lifecycle behavior.
// It must not execute user code or own host-stream frame semantics.
type HostStreamLifecycleProvider interface {
	CheckReadiness(ctx context.Context, binding HostStreamBinding) (HostStreamReadiness, error)
	Cleanup(ctx context.Context, binding HostStreamBinding) (HostStreamCleanup, error)
}

// HostStreamLifecycleController drives readiness and cleanup for one binding.
type HostStreamLifecycleController struct {
	mu            sync.Mutex
	binding       HostStreamBinding
	provider      HostStreamLifecycleProvider
	state         HostStreamLifecycleState
	readiness     HostStreamReadiness
	cleanupResult *HostStreamCleanup
}

func NewHostStreamLifecycleController(binding HostStreamBinding, provider HostStreamLifecycleProvider) *HostStreamLifecycleController {
	return &HostStreamLifecycleController{
		binding:   binding,
		provider:  provider,
		state:     HostStreamLifecycleDeclared,
		readiness: NewHostStreamReadinessFromMap(binding.Readiness),
	}
}

func (c *HostStreamLifecycleController) State() HostStreamLifecycleState {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.state
}

func (c *HostStreamLifecycleController) Readiness() HostStreamReadiness {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.readiness
}

func (c *HostStreamLifecycleController) CleanupResult() *HostStreamCleanup {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.cleanupResult == nil {
		return nil
	}
	copy := *c.cleanupResult
	copy.Metadata = copyMap(copy.Metadata)
	return &copy
}

func (c *HostStreamLifecycleController) CheckReadiness(ctx context.Context) (HostStreamReadiness, error) {
	if ctx == nil {
		return HostStreamReadiness{}, invalidProfileClient(hostBindingProfile, "context is required")
	}
	c.mu.Lock()
	if c.provider == nil {
		c.mu.Unlock()
		return HostStreamReadiness{}, invalidProfileClient(hostBindingProfile, "host stream lifecycle provider is required")
	}
	if c.state == HostStreamLifecycleCleaning || c.state == HostStreamLifecycleCleaned || c.state == HostStreamLifecycleClosed {
		c.mu.Unlock()
		return HostStreamReadiness{}, invalidProfilePayload(hostBindingProfile, "host stream lifecycle is not readable", nil)
	}
	c.state = HostStreamLifecycleChecking
	provider := c.provider
	binding := c.binding
	c.mu.Unlock()
	readiness, err := provider.CheckReadiness(ctx, binding)
	c.mu.Lock()
	if err != nil {
		c.state = HostStreamLifecycleFailed
		c.mu.Unlock()
		return HostStreamReadiness{}, wrapHostBindingProviderError("host binding readiness provider failed", err)
	}
	c.readiness = readiness
	if readiness.EndpointReady != nil && *readiness.EndpointReady {
		c.state = HostStreamLifecycleReady
	} else {
		c.state = HostStreamLifecycleNotReady
	}
	c.mu.Unlock()
	return readiness, nil
}

func (c *HostStreamLifecycleController) Cleanup(ctx context.Context) (HostStreamCleanup, error) {
	if ctx == nil {
		return HostStreamCleanup{}, invalidProfileClient(hostBindingProfile, "context is required")
	}
	c.mu.Lock()
	if c.provider == nil {
		c.mu.Unlock()
		return HostStreamCleanup{}, invalidProfileClient(hostBindingProfile, "host stream lifecycle provider is required")
	}
	if c.state == HostStreamLifecycleCleaned || c.state == HostStreamLifecycleClosed {
		result := c.cleanupResultOrBindingLocked()
		c.mu.Unlock()
		return result, nil
	}
	if c.state == HostStreamLifecycleCleaning {
		c.mu.Unlock()
		return HostStreamCleanup{}, invalidProfilePayload(hostBindingProfile, "host stream lifecycle cleanup is already running", nil)
	}
	if c.state == HostStreamLifecycleChecking {
		c.mu.Unlock()
		return HostStreamCleanup{}, invalidProfilePayload(hostBindingProfile, "host stream lifecycle readiness check is running", nil)
	}
	c.state = HostStreamLifecycleCleaning
	provider := c.provider
	binding := c.binding
	c.mu.Unlock()
	cleanup, err := provider.Cleanup(ctx, binding)
	c.mu.Lock()
	if err != nil {
		c.state = HostStreamLifecycleFailed
		c.mu.Unlock()
		return HostStreamCleanup{}, wrapHostBindingProviderError("host binding cleanup provider failed", err)
	}
	c.cleanupResult = &cleanup
	c.state = HostStreamLifecycleCleaned
	c.mu.Unlock()
	return cleanup, nil
}

func (c *HostStreamLifecycleController) Close(ctx context.Context) error {
	c.mu.Lock()
	if c.state == HostStreamLifecycleClosed {
		c.mu.Unlock()
		return nil
	}
	needsCleanup := c.state != HostStreamLifecycleCleaned
	c.mu.Unlock()
	if needsCleanup {
		if _, err := c.Cleanup(ctx); err != nil {
			return err
		}
	}
	c.mu.Lock()
	c.state = HostStreamLifecycleClosed
	c.mu.Unlock()
	return nil
}

func (c *HostStreamLifecycleController) cleanupResultOrBindingLocked() HostStreamCleanup {
	if c.cleanupResult != nil {
		return *c.cleanupResult
	}
	return NewHostStreamCleanupFromMap(c.binding.Cleanup)
}

func mapWithoutKeys(value map[string]any, keys ...string) map[string]any {
	if len(value) == 0 {
		return map[string]any{}
	}
	blocked := make(map[string]struct{}, len(keys))
	for _, key := range keys {
		blocked[key] = struct{}{}
	}
	out := make(map[string]any)
	for key, item := range value {
		if _, ok := blocked[key]; !ok {
			out[key] = item
		}
	}
	return out
}

func wrapHostBindingProviderError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return withProfileErrorDetails(sdkErr, hostBindingProfile)
	}
	return withProfileErrorDetails(&SDKError{
		Code:      ErrRouteUnavailable,
		Stage:     "provider",
		Retry:     RetrySafe,
		Retryable: RetryableForHint(RetrySafe),
		Message:   message,
		Cause:     cause,
	}, hostBindingProfile)
}
