package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// DescriptorRefRequest asks the daemon/Axon boundary to project a DescriptorRef.
type DescriptorRefRequest struct {
	DescriptorRef string         `json:"descriptor_ref"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

// IdentityProjectionRequest asks for a daemon-owned identity projection.
type IdentityProjectionRequest struct {
	URA      string         `json:"ura,omitempty"`
	Kind     string         `json:"kind,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
}

// LocalResourceRefRequest asks the daemon to create a local resource ref.
type LocalResourceRefRequest struct {
	Path       string `json:"path"`
	Capability string `json:"capability"`
}

// IdentityProjection is the SDK identity.schema.json projection.
type IdentityProjection struct {
	Kind              string         `json:"kind"`
	Valid             bool           `json:"valid"`
	URA               string         `json:"ura,omitempty"`
	Realm             string         `json:"realm,omitempty"`
	DisplayID         string         `json:"display_id,omitempty"`
	DescriptorRef     string         `json:"descriptor_ref,omitempty"`
	AbilityURA        string         `json:"ability_ura,omitempty"`
	DescriptorVersion string         `json:"descriptor_version,omitempty"`
	Profile           string         `json:"profile"`
	Components        map[string]any `json:"components"`
	Metadata          map[string]any `json:"metadata"`
}

// ResourceRef is the SDK resource-ref.schema.json projection.
type ResourceRef struct {
	ResourceURA   string `json:"resource_ura"`
	OwnerURA      string `json:"owner_ura"`
	Namespace     string `json:"namespace"`
	DisplayPath   string `json:"display_path,omitempty"`
	Capability    string `json:"capability"`
	ExpiresUnixMS int64  `json:"expires_unix_ms"`
	Revision      string `json:"revision"`
}

// IdentityTransport supplies identity projections behind the SDK facade.
type IdentityTransport interface {
	ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// IdentityTransportFunc adapts functions into an IdentityTransport.
type IdentityTransportFunc struct {
	ProjectDescriptorRefFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectIdentityFunc      func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildResourceRefFunc     func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f IdentityTransportFunc) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ProjectDescriptorRefFunc == nil {
		return nil, invalidRuntimeClient("identity descriptor projection transport function is required")
	}
	return f.ProjectDescriptorRefFunc(ctx, requestJSON)
}

func (f IdentityTransportFunc) ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ProjectIdentityFunc == nil {
		return nil, invalidRuntimeClient("identity projection transport function is required")
	}
	return f.ProjectIdentityFunc(ctx, requestJSON)
}

func (f IdentityTransportFunc) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildResourceRefFunc == nil {
		return nil, invalidRuntimeClient("identity resource-ref transport function is required")
	}
	return f.BuildResourceRefFunc(ctx, requestJSON)
}

// IdentityClient is the Directory + Identity projection facade.
type IdentityClient struct {
	transport IdentityTransport
}

func NewIdentityClient(transport IdentityTransport) (*IdentityClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("identity transport is required")
	}
	return &IdentityClient{transport: transport}, nil
}

func (c *IdentityClient) ProjectDescriptorRef(ctx context.Context, req DescriptorRefRequest) (IdentityProjection, error) {
	if err := c.requireReady(ctx); err != nil {
		return IdentityProjection{}, err
	}
	if req.DescriptorRef == "" {
		return IdentityProjection{}, invalidRuntimePayload("descriptor_ref is required", nil)
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return IdentityProjection{}, invalidRuntimePayload(fmt.Sprintf("encode descriptor projection request: %v", err), err)
	}
	raw, err := c.transport.ProjectDescriptorRef(ctx, requestJSON)
	if err != nil {
		return IdentityProjection{}, wrapIdentityTransportError("identity descriptor projection failed", err)
	}
	return NewIdentityProjectionFromJSON(raw)
}

func (c *IdentityClient) ProjectIdentity(ctx context.Context, req IdentityProjectionRequest) (IdentityProjection, error) {
	if err := c.requireReady(ctx); err != nil {
		return IdentityProjection{}, err
	}
	if req.URA == "" && req.Kind == "" {
		return IdentityProjection{}, invalidRuntimePayload("ura or kind is required", nil)
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return IdentityProjection{}, invalidRuntimePayload(fmt.Sprintf("encode identity projection request: %v", err), err)
	}
	raw, err := c.transport.ProjectIdentity(ctx, requestJSON)
	if err != nil {
		return IdentityProjection{}, wrapIdentityTransportError("identity projection failed", err)
	}
	return NewIdentityProjectionFromJSON(raw)
}

func (c *IdentityClient) BuildResourceRef(ctx context.Context, req LocalResourceRefRequest) (ResourceRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return ResourceRef{}, err
	}
	if req.Path == "" || req.Capability == "" {
		return ResourceRef{}, invalidRuntimePayload("path and capability are required", nil)
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return ResourceRef{}, invalidRuntimePayload(fmt.Sprintf("encode resource-ref request: %v", err), err)
	}
	raw, err := c.transport.BuildResourceRef(ctx, requestJSON)
	if err != nil {
		return ResourceRef{}, wrapIdentityTransportError("identity resource-ref build failed", err)
	}
	return NewResourceRefFromJSON(raw)
}

func (c *IdentityClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("identity client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewIdentityProjectionFromJSON(raw []byte) (IdentityProjection, error) {
	var projection IdentityProjection
	if err := json.Unmarshal(raw, &projection); err != nil {
		return IdentityProjection{}, invalidRuntimePayload(fmt.Sprintf("decode identity projection JSON: %v", err), err)
	}
	if projection.Kind == "" || projection.Profile == "" || projection.Components == nil || projection.Metadata == nil {
		return IdentityProjection{}, invalidRuntimePayload("invalid identity projection", nil)
	}
	if projection.Kind == "descriptor_ref" && (projection.DescriptorRef == "" || projection.AbilityURA == "" || projection.DescriptorVersion == "") {
		return IdentityProjection{}, invalidRuntimePayload("invalid descriptor_ref projection", nil)
	}
	return projection, nil
}

func NewResourceRefFromJSON(raw []byte) (ResourceRef, error) {
	var ref ResourceRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return ResourceRef{}, invalidRuntimePayload(fmt.Sprintf("decode resource-ref JSON: %v", err), err)
	}
	if ref.ResourceURA == "" || ref.OwnerURA == "" || ref.Namespace == "" || ref.Capability == "" || ref.Revision == "" {
		return ResourceRef{}, invalidRuntimePayload("invalid resource-ref projection", nil)
	}
	return ref, nil
}

func wrapIdentityTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
