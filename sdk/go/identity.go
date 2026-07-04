package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const (
	DefaultSigningKeyPageSize = 50
	MaxSigningKeyPageSize     = 500
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

// SigningKeyRegistrationRequest registers daemon-owned public signing-key metadata.
type SigningKeyRegistrationRequest struct {
	OwnerURA        string         `json:"owner_ura"`
	KeyID           string         `json:"key_id"`
	Algorithm       string         `json:"algorithm"`
	PublicKeyBase64 string         `json:"public_key_base64"`
	Usage           []string       `json:"usage"`
	Metadata        map[string]any `json:"metadata,omitempty"`
}

// SigningKeyListRequest asks for a bounded signing-key read-model page.
type SigningKeyListRequest struct {
	OwnerURA string `json:"owner_ura,omitempty"`
	Limit    int    `json:"limit,omitempty"`
	Cursor   string `json:"cursor,omitempty"`
}

// SigningKeyRevokeRequest revokes one daemon-owned signing key.
type SigningKeyRevokeRequest struct {
	KeyID  string `json:"key_id"`
	Reason string `json:"reason"`
}

// SignerRequest asks the daemon for an authorized signer handle projection.
type SignerRequest struct {
	OwnerURA string         `json:"owner_ura"`
	KeyID    string         `json:"key_id"`
	Usage    string         `json:"usage,omitempty"`
	Metadata map[string]any `json:"metadata,omitempty"`
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

type SigningKeyRecord struct {
	Profile         string         `json:"profile"`
	KeyID           string         `json:"key_id"`
	OwnerURA        string         `json:"owner_ura"`
	Algorithm       string         `json:"algorithm"`
	PublicKeyBase64 string         `json:"public_key_base64"`
	State           string         `json:"state"`
	Usage           []string       `json:"usage"`
	CreatedUnixMS   int64          `json:"created_unix_ms,omitempty"`
	RevokedUnixMS   int64          `json:"revoked_unix_ms,omitempty"`
	Metadata        map[string]any `json:"metadata"`
}

type SigningKeyPage struct {
	Profile    string             `json:"profile"`
	Items      []SigningKeyRecord `json:"items"`
	NextCursor *string            `json:"next_cursor"`
	Limit      int                `json:"limit"`
	Metadata   map[string]any     `json:"metadata"`
}

type SigningKeyRevokeResult struct {
	Profile  string         `json:"profile"`
	KeyID    string         `json:"key_id"`
	Revoked  bool           `json:"revoked"`
	State    string         `json:"state"`
	Metadata map[string]any `json:"metadata"`
}

// SignerHandle is a daemon-authorized signer reference, not local key material.
type SignerHandle struct {
	Profile   string         `json:"profile"`
	SignerID  string         `json:"signer_id"`
	OwnerURA  string         `json:"owner_ura"`
	KeyID     string         `json:"key_id"`
	Algorithm string         `json:"algorithm"`
	Policy    map[string]any `json:"policy"`
	Metadata  map[string]any `json:"metadata"`
}

// IdentityTransport supplies identity projections behind the SDK facade.
type IdentityTransport interface {
	ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error)
	RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error)
	RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error)
	Signer(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// IdentityTransportFunc adapts functions into an IdentityTransport.
type IdentityTransportFunc struct {
	ProjectDescriptorRefFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ProjectIdentityFunc      func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildResourceRefFunc     func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RegisterSigningKeyFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListSigningKeysFunc      func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RevokeSigningKeyFunc     func(ctx context.Context, requestJSON []byte) ([]byte, error)
	SignerFunc               func(ctx context.Context, requestJSON []byte) ([]byte, error)
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

func (f IdentityTransportFunc) RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RegisterSigningKeyFunc == nil {
		return nil, invalidRuntimeClient("identity register-signing-key transport function is required")
	}
	return f.RegisterSigningKeyFunc(ctx, requestJSON)
}

func (f IdentityTransportFunc) ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListSigningKeysFunc == nil {
		return nil, invalidRuntimeClient("identity list-signing-keys transport function is required")
	}
	return f.ListSigningKeysFunc(ctx, requestJSON)
}

func (f IdentityTransportFunc) RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RevokeSigningKeyFunc == nil {
		return nil, invalidRuntimeClient("identity revoke-signing-key transport function is required")
	}
	return f.RevokeSigningKeyFunc(ctx, requestJSON)
}

func (f IdentityTransportFunc) Signer(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.SignerFunc == nil {
		return nil, invalidRuntimeClient("identity signer transport function is required")
	}
	return f.SignerFunc(ctx, requestJSON)
}

// IdentityClient is the Directory + Identity projection facade.
type IdentityClient struct {
	lifecycle profileClientLifecycle
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

func (c *IdentityClient) RegisterSigningKey(ctx context.Context, req SigningKeyRegistrationRequest) (SigningKeyRecord, error) {
	if err := c.requireReady(ctx); err != nil {
		return SigningKeyRecord{}, err
	}
	requestJSON, err := marshalSigningKeyRegistrationRequest(req)
	if err != nil {
		return SigningKeyRecord{}, err
	}
	raw, err := c.transport.RegisterSigningKey(ctx, requestJSON)
	if err != nil {
		return SigningKeyRecord{}, wrapIdentityTransportError("identity register signing key failed", err)
	}
	return NewSigningKeyRecordFromJSON(raw)
}

func (c *IdentityClient) ListSigningKeys(ctx context.Context, req SigningKeyListRequest) (SigningKeyPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return SigningKeyPage{}, err
	}
	requestJSON, err := marshalSigningKeyListRequest(req)
	if err != nil {
		return SigningKeyPage{}, err
	}
	raw, err := c.transport.ListSigningKeys(ctx, requestJSON)
	if err != nil {
		return SigningKeyPage{}, wrapIdentityTransportError("identity list signing keys failed", err)
	}
	return NewSigningKeyPageFromJSON(raw)
}

func (c *IdentityClient) RevokeSigningKey(ctx context.Context, req SigningKeyRevokeRequest) (SigningKeyRevokeResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return SigningKeyRevokeResult{}, err
	}
	requestJSON, err := marshalSigningKeyRevokeRequest(req)
	if err != nil {
		return SigningKeyRevokeResult{}, err
	}
	raw, err := c.transport.RevokeSigningKey(ctx, requestJSON)
	if err != nil {
		return SigningKeyRevokeResult{}, wrapIdentityTransportError("identity revoke signing key failed", err)
	}
	return NewSigningKeyRevokeResultFromJSON(raw)
}

func (c *IdentityClient) Signer(ctx context.Context, req SignerRequest) (SignerHandle, error) {
	if err := c.requireReady(ctx); err != nil {
		return SignerHandle{}, err
	}
	requestJSON, err := marshalSignerRequest(req)
	if err != nil {
		return SignerHandle{}, err
	}
	raw, err := c.transport.Signer(ctx, requestJSON)
	if err != nil {
		return SignerHandle{}, wrapIdentityTransportError("identity signer failed", err)
	}
	return NewSignerHandleFromJSON(raw)
}

func (c *IdentityClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("identity client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "identity")
}

func (c *IdentityClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("identity client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "identity")
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

func NewSigningKeyRecordFromJSON(raw []byte) (SigningKeyRecord, error) {
	var record SigningKeyRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return SigningKeyRecord{}, invalidRuntimePayload(fmt.Sprintf("decode signing-key record JSON: %v", err), err)
	}
	if err := validateSigningKeyRecord(record); err != nil {
		return SigningKeyRecord{}, err
	}
	return record, nil
}

func NewSigningKeyPageFromJSON(raw []byte) (SigningKeyPage, error) {
	var page SigningKeyPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return SigningKeyPage{}, invalidRuntimePayload(fmt.Sprintf("decode signing-key page JSON: %v", err), err)
	}
	if page.Profile == "" || page.Items == nil || page.Limit < 1 || page.Limit > MaxSigningKeyPageSize || page.Metadata == nil {
		return SigningKeyPage{}, invalidRuntimePayload("invalid signing-key page projection", nil)
	}
	for _, record := range page.Items {
		if err := validateSigningKeyRecord(record); err != nil {
			return SigningKeyPage{}, err
		}
	}
	return page, nil
}

func NewSigningKeyRevokeResultFromJSON(raw []byte) (SigningKeyRevokeResult, error) {
	var result SigningKeyRevokeResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return SigningKeyRevokeResult{}, invalidRuntimePayload(fmt.Sprintf("decode signing-key revoke result JSON: %v", err), err)
	}
	if result.Profile == "" || result.KeyID == "" || result.State == "" || result.Metadata == nil {
		return SigningKeyRevokeResult{}, invalidRuntimePayload("invalid signing-key revoke result projection", nil)
	}
	if !result.Revoked {
		return SigningKeyRevokeResult{}, invalidRuntimePayload("signing-key revoke result is not terminal", nil)
	}
	return result, nil
}

func NewSignerHandleFromJSON(raw []byte) (SignerHandle, error) {
	var signer SignerHandle
	if err := json.Unmarshal(raw, &signer); err != nil {
		return SignerHandle{}, invalidRuntimePayload(fmt.Sprintf("decode signer handle JSON: %v", err), err)
	}
	if signer.Profile == "" || signer.SignerID == "" || signer.OwnerURA == "" || signer.KeyID == "" ||
		signer.Algorithm == "" || signer.Policy == nil || signer.Metadata == nil {
		return SignerHandle{}, invalidRuntimePayload("invalid signer handle projection", nil)
	}
	return signer, nil
}

func marshalSigningKeyRegistrationRequest(req SigningKeyRegistrationRequest) ([]byte, error) {
	if err := requiredCleanIdentityField(req.OwnerURA, "owner_ura"); err != nil {
		return nil, err
	}
	if err := requiredCleanIdentityField(req.KeyID, "key_id"); err != nil {
		return nil, err
	}
	if err := requiredCleanIdentityField(req.Algorithm, "algorithm"); err != nil {
		return nil, err
	}
	if err := requiredCleanIdentityField(req.PublicKeyBase64, "public_key_base64"); err != nil {
		return nil, err
	}
	if len(req.Usage) == 0 {
		return nil, invalidRuntimePayload("owner_ura, key_id, algorithm, public_key_base64, and usage are required", nil)
	}
	for _, usage := range req.Usage {
		if err := requiredCleanIdentityField(usage, "usage"); err != nil {
			return nil, err
		}
	}
	if containsPrivateKeyMetadata(req.Metadata) {
		return nil, invalidRuntimePayload("private key material must not be supplied to identity facade", nil)
	}
	return marshalIdentityRequest(req, "signing-key registration request")
}

func marshalSigningKeyListRequest(req SigningKeyListRequest) ([]byte, error) {
	if strings.TrimSpace(req.OwnerURA) != req.OwnerURA || strings.TrimSpace(req.Cursor) != req.Cursor {
		return nil, invalidRuntimePayload("owner_ura and cursor must not contain surrounding whitespace", nil)
	}
	if req.Limit == 0 {
		req.Limit = DefaultSigningKeyPageSize
	}
	if req.Limit < 1 || req.Limit > MaxSigningKeyPageSize {
		return nil, invalidRuntimePayload("signing-key page limit exceeds bounds", nil)
	}
	return marshalIdentityRequest(req, "signing-key list request")
}

func marshalSigningKeyRevokeRequest(req SigningKeyRevokeRequest) ([]byte, error) {
	if err := requiredCleanIdentityField(req.KeyID, "key_id"); err != nil {
		return nil, err
	}
	if err := requiredCleanIdentityField(req.Reason, "reason"); err != nil {
		return nil, err
	}
	return marshalIdentityRequest(req, "signing-key revoke request")
}

func marshalSignerRequest(req SignerRequest) ([]byte, error) {
	if err := requiredCleanIdentityField(req.OwnerURA, "owner_ura"); err != nil {
		return nil, err
	}
	if err := requiredCleanIdentityField(req.KeyID, "key_id"); err != nil {
		return nil, err
	}
	if req.Usage != "" {
		if err := requiredCleanIdentityField(req.Usage, "usage"); err != nil {
			return nil, err
		}
	}
	if containsPrivateKeyMetadata(req.Metadata) {
		return nil, invalidRuntimePayload("private key material must not be supplied to identity facade", nil)
	}
	return marshalIdentityRequest(req, "signer request")
}

func marshalIdentityRequest(req any, label string) ([]byte, error) {
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode %s: %v", label, err), err)
	}
	return requestJSON, nil
}

func requiredCleanIdentityField(value string, field string) error {
	if strings.TrimSpace(value) == "" {
		return invalidRuntimePayload(fmt.Sprintf("%s is required", field), nil)
	}
	if strings.TrimSpace(value) != value {
		return invalidRuntimePayload(fmt.Sprintf("%s must not contain surrounding whitespace", field), nil)
	}
	return nil
}

func validateSigningKeyRecord(record SigningKeyRecord) error {
	if record.Profile == "" || record.KeyID == "" || record.OwnerURA == "" ||
		record.Algorithm == "" || record.PublicKeyBase64 == "" || record.State == "" ||
		len(record.Usage) == 0 || record.Metadata == nil {
		return invalidRuntimePayload("invalid signing-key record projection", nil)
	}
	return nil
}

func containsPrivateKeyMetadata(metadata map[string]any) bool {
	for key := range metadata {
		normalized := strings.ToLower(strings.ReplaceAll(key, "_", ""))
		if strings.Contains(normalized, "privatekey") || strings.Contains(normalized, "secret") || strings.Contains(normalized, "seed") {
			return true
		}
	}
	return false
}

func wrapIdentityTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
