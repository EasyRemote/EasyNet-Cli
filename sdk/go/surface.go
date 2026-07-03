package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const surfaceProfile = "surface"
const surfaceReadModelSource = "pages_read_model"

const (
	DefaultSurfacePageSize = 50
	MaxSurfacePageSize     = 500
)

// SurfaceCarrierBase is the complete carrier context shared by Surface operations.
type SurfaceCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

type SurfaceListPagesRequest struct {
	SurfaceCarrierBase
	Limit  int    `json:"limit,omitempty"`
	Cursor string `json:"cursor,omitempty"`
}

type SurfaceCreatePageRequest struct {
	SurfaceCarrierBase
	ProjectID  string `json:"project_id"`
	Folder     string `json:"folder"`
	Visibility string `json:"visibility,omitempty"`
}

type SurfaceDeletePageRequest struct {
	SurfaceCarrierBase
	ProjectID string `json:"project_id"`
}

type SurfaceManifestRequest struct {
	SurfaceCarrierBase
	ProjectID string `json:"project_id"`
}

type SurfacePublicPageRefRequest struct {
	Page SurfacePageRecord `json:"page"`
}

type PageQuery = SurfaceListPagesRequest
type CreatePageRequest = SurfaceCreatePageRequest
type DeletePageRequest = SurfaceDeletePageRequest
type PublicPageRefRequest = SurfacePublicPageRefRequest

type SurfacePageRecord struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	PageID     string         `json:"page_id"`
	OwnerURA   string         `json:"owner_ura"`
	SurfaceRef string         `json:"surface_ref"`
	PublicRef  *string        `json:"public_ref"`
	Status     *string        `json:"status"`
	Metadata   map[string]any `json:"metadata"`
}

type SurfacePagePage struct {
	Profile    string              `json:"profile"`
	Kind       string              `json:"kind"`
	ItemKind   string              `json:"item_kind"`
	Items      []SurfacePageRecord `json:"items"`
	NextCursor *string             `json:"next_cursor"`
	Limit      int                 `json:"limit"`
	Source     string              `json:"source"`
	Metadata   map[string]any      `json:"metadata"`
}

type SurfaceManifest struct {
	Profile    string            `json:"profile"`
	Kind       string            `json:"kind"`
	PageID     string            `json:"page_id"`
	OwnerURA   string            `json:"owner_ura"`
	SurfaceRef string            `json:"surface_ref"`
	PublicRef  string            `json:"public_ref"`
	Page       SurfacePageRecord `json:"page"`
	Entrypoint map[string]any    `json:"entrypoint"`
	Metadata   map[string]any    `json:"metadata"`
}

type SurfacePublicPageRef struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	PageID     string         `json:"page_id"`
	OwnerURA   string         `json:"owner_ura"`
	SurfaceRef string         `json:"surface_ref"`
	PublicRef  string         `json:"public_ref"`
	RouteKind  string         `json:"route_kind"`
	Metadata   map[string]any `json:"metadata"`
}

type SurfaceMutationResult struct {
	Profile   string         `json:"profile"`
	Kind      string         `json:"kind"`
	Operation string         `json:"operation"`
	PageID    string         `json:"page_id"`
	Removed   bool           `json:"removed"`
	State     string         `json:"state"`
	Metadata  map[string]any `json:"metadata"`
}

// SurfaceTransport supplies daemon Surface operations behind the facade.
type SurfaceTransport interface {
	BuildListPagesInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCreatePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildDeletePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildManifestInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListPages(ctx context.Context, requestJSON []byte) ([]byte, error)
	CreatePage(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeletePage(ctx context.Context, requestJSON []byte) ([]byte, error)
	SurfaceManifest(ctx context.Context, requestJSON []byte) ([]byte, error)
	PublicPageRef(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// SurfaceTransportFunc adapts functions into a SurfaceTransport.
type SurfaceTransportFunc struct {
	BuildListPagesInvocationFunc  func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildCreatePageInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildDeletePageInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildManifestInvocationFunc   func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListPagesFunc                 func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CreatePageFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeletePageFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	SurfaceManifestFunc           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	PublicPageRefFunc             func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f SurfaceTransportFunc) BuildListPagesInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildListPagesInvocationFunc == nil {
		return nil, invalidRuntimeClient("surface list-pages invocation transport function is required")
	}
	return f.BuildListPagesInvocationFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) BuildCreatePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildCreatePageInvocationFunc == nil {
		return nil, invalidRuntimeClient("surface create-page invocation transport function is required")
	}
	return f.BuildCreatePageInvocationFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) BuildDeletePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildDeletePageInvocationFunc == nil {
		return nil, invalidRuntimeClient("surface delete-page invocation transport function is required")
	}
	return f.BuildDeletePageInvocationFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) BuildManifestInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildManifestInvocationFunc == nil {
		return nil, invalidRuntimeClient("surface manifest invocation transport function is required")
	}
	return f.BuildManifestInvocationFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) ListPages(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListPagesFunc == nil {
		return nil, invalidRuntimeClient("surface list-pages transport function is required")
	}
	return f.ListPagesFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) CreatePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.CreatePageFunc == nil {
		return nil, invalidRuntimeClient("surface create-page transport function is required")
	}
	return f.CreatePageFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) DeletePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.DeletePageFunc == nil {
		return nil, invalidRuntimeClient("surface delete-page transport function is required")
	}
	return f.DeletePageFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) SurfaceManifest(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.SurfaceManifestFunc == nil {
		return nil, invalidRuntimeClient("surface manifest transport function is required")
	}
	return f.SurfaceManifestFunc(ctx, requestJSON)
}

func (f SurfaceTransportFunc) PublicPageRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.PublicPageRefFunc == nil {
		return nil, invalidRuntimeClient("surface public-page-ref transport function is required")
	}
	return f.PublicPageRefFunc(ctx, requestJSON)
}

// SurfaceClient is the Surface profile facade.
type SurfaceClient struct {
	transport SurfaceTransport
}

func NewSurfaceClient(transport SurfaceTransport) (*SurfaceClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("surface transport is required")
	}
	return &SurfaceClient{transport: transport}, nil
}

func (c *SurfaceClient) BuildListPagesInvocation(ctx context.Context, req SurfaceListPagesRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateSurfaceListPagesRequest, c.transport.BuildListPagesInvocation, "surface list-pages invocation failed")
}

func (c *SurfaceClient) BuildCreatePageInvocation(ctx context.Context, req SurfaceCreatePageRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateSurfaceCreatePageRequest, c.transport.BuildCreatePageInvocation, "surface create-page invocation failed")
}

func (c *SurfaceClient) BuildDeletePageInvocation(ctx context.Context, req SurfaceDeletePageRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateSurfaceDeletePageRequest, c.transport.BuildDeletePageInvocation, "surface delete-page invocation failed")
}

func (c *SurfaceClient) BuildManifestInvocation(ctx context.Context, req SurfaceManifestRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateSurfaceManifestRequest, c.transport.BuildManifestInvocation, "surface manifest invocation failed")
}

func (c *SurfaceClient) ListPages(ctx context.Context, req SurfaceListPagesRequest) (SurfacePagePage, error) {
	return c.pageOperation(ctx, req, validateSurfaceListPagesRequest, c.transport.ListPages, "surface list pages failed")
}

func (c *SurfaceClient) CreatePage(ctx context.Context, req SurfaceCreatePageRequest) (SurfacePageRecord, error) {
	if err := c.requireReady(ctx); err != nil {
		return SurfacePageRecord{}, err
	}
	requestJSON, err := marshalSurfaceRequest(req, validateSurfaceCreatePageRequest)
	if err != nil {
		return SurfacePageRecord{}, err
	}
	raw, err := c.transport.CreatePage(ctx, requestJSON)
	if err != nil {
		return SurfacePageRecord{}, wrapSurfaceTransportError("surface create page failed", err)
	}
	return NewSurfacePageRecordFromJSON(raw)
}

func (c *SurfaceClient) DeletePage(ctx context.Context, req SurfaceDeletePageRequest) (SurfaceMutationResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return SurfaceMutationResult{}, err
	}
	requestJSON, err := marshalSurfaceRequest(req, validateSurfaceDeletePageRequest)
	if err != nil {
		return SurfaceMutationResult{}, err
	}
	raw, err := c.transport.DeletePage(ctx, requestJSON)
	if err != nil {
		return SurfaceMutationResult{}, wrapSurfaceTransportError("surface delete page failed", err)
	}
	return NewSurfaceMutationResultFromJSON(raw)
}

func (c *SurfaceClient) SurfaceManifest(ctx context.Context, req SurfaceManifestRequest) (SurfaceManifest, error) {
	if err := c.requireReady(ctx); err != nil {
		return SurfaceManifest{}, err
	}
	requestJSON, err := marshalSurfaceRequest(req, validateSurfaceManifestRequest)
	if err != nil {
		return SurfaceManifest{}, err
	}
	raw, err := c.transport.SurfaceManifest(ctx, requestJSON)
	if err != nil {
		return SurfaceManifest{}, wrapSurfaceTransportError("surface manifest failed", err)
	}
	return NewSurfaceManifestFromJSON(raw)
}

func (c *SurfaceClient) PublicPageRef(ctx context.Context, req SurfacePublicPageRefRequest) (SurfacePublicPageRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return SurfacePublicPageRef{}, err
	}
	if err := validateSurfacePageRecord(req.Page); err != nil {
		return SurfacePublicPageRef{}, err
	}
	requestJSON, err := json.Marshal(req.Page)
	if err != nil {
		return SurfacePublicPageRef{}, invalidRuntimePayload(fmt.Sprintf("encode surface public ref request: %v", err), err)
	}
	raw, err := c.transport.PublicPageRef(ctx, requestJSON)
	if err != nil {
		return SurfacePublicPageRef{}, wrapSurfaceTransportError("surface public page ref failed", err)
	}
	return NewSurfacePublicPageRefFromJSON(raw)
}

func (c *SurfaceClient) buildInvocation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalSurfaceRequest(req, validate)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapSurfaceTransportError(label, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *SurfaceClient) pageOperation(ctx context.Context, req any, validate func(any) error, fn func(context.Context, []byte) ([]byte, error), label string) (SurfacePagePage, error) {
	if err := c.requireReady(ctx); err != nil {
		return SurfacePagePage{}, err
	}
	requestJSON, err := marshalSurfaceRequest(req, validate)
	if err != nil {
		return SurfacePagePage{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return SurfacePagePage{}, wrapSurfaceTransportError(label, err)
	}
	return NewSurfacePagePageFromJSON(raw)
}

func (c *SurfaceClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("surface client is not initialized")
	}
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	return nil
}

func NewSurfacePageRecordFromJSON(raw []byte) (SurfacePageRecord, error) {
	var record SurfacePageRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return SurfacePageRecord{}, invalidRuntimePayload(fmt.Sprintf("decode surface page record JSON: %v", err), err)
	}
	if err := validateSurfacePageRecord(record); err != nil {
		return SurfacePageRecord{}, err
	}
	return record, nil
}

func NewSurfacePagePageFromJSON(raw []byte) (SurfacePagePage, error) {
	var page SurfacePagePage
	if err := json.Unmarshal(raw, &page); err != nil {
		return SurfacePagePage{}, invalidRuntimePayload(fmt.Sprintf("decode surface page page JSON: %v", err), err)
	}
	if page.Profile != surfaceProfile || page.Kind != "surface_page_page" ||
		page.ItemKind != "page_record" || page.Source != surfaceReadModelSource ||
		page.Items == nil || page.Metadata == nil {
		return SurfacePagePage{}, invalidRuntimePayload("invalid surface page projection", nil)
	}
	if page.Limit < 1 || page.Limit > MaxSurfacePageSize {
		return SurfacePagePage{}, invalidRuntimePayload("surface page limit exceeds bounds", nil)
	}
	for _, item := range page.Items {
		if err := validateSurfacePageRecord(item); err != nil {
			return SurfacePagePage{}, err
		}
	}
	return page, nil
}

func NewSurfaceManifestFromJSON(raw []byte) (SurfaceManifest, error) {
	var manifest SurfaceManifest
	if err := json.Unmarshal(raw, &manifest); err != nil {
		return SurfaceManifest{}, invalidRuntimePayload(fmt.Sprintf("decode surface manifest JSON: %v", err), err)
	}
	if manifest.Profile != surfaceProfile || manifest.Kind != "surface_manifest" ||
		manifest.PageID == "" || manifest.OwnerURA == "" || manifest.SurfaceRef == "" ||
		manifest.PublicRef == "" || manifest.Entrypoint == nil || manifest.Metadata == nil {
		return SurfaceManifest{}, invalidRuntimePayload("invalid surface manifest projection", nil)
	}
	if err := validateSurfacePageRecord(manifest.Page); err != nil {
		return SurfaceManifest{}, err
	}
	return manifest, nil
}

func NewSurfacePublicPageRefFromJSON(raw []byte) (SurfacePublicPageRef, error) {
	var ref SurfacePublicPageRef
	if err := json.Unmarshal(raw, &ref); err != nil {
		return SurfacePublicPageRef{}, invalidRuntimePayload(fmt.Sprintf("decode surface public page ref JSON: %v", err), err)
	}
	if ref.Profile != surfaceProfile || ref.Kind != "public_page_ref" || ref.PageID == "" ||
		ref.OwnerURA == "" || ref.SurfaceRef == "" || ref.PublicRef == "" ||
		ref.RouteKind != "hub_web" || ref.Metadata == nil {
		return SurfacePublicPageRef{}, invalidRuntimePayload("invalid surface public page ref projection", nil)
	}
	return ref, nil
}

func NewSurfaceMutationResultFromJSON(raw []byte) (SurfaceMutationResult, error) {
	var result SurfaceMutationResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return SurfaceMutationResult{}, invalidRuntimePayload(fmt.Sprintf("decode surface mutation result JSON: %v", err), err)
	}
	if result.Profile != surfaceProfile || result.Kind != "surface_mutation_result" ||
		result.Operation != "delete" || result.PageID == "" || result.Metadata == nil {
		return SurfaceMutationResult{}, invalidRuntimePayload("invalid surface mutation result projection", nil)
	}
	if result.State != "deleted" && result.State != "unknown" {
		return SurfaceMutationResult{}, invalidRuntimePayload("invalid surface mutation state", nil)
	}
	return result, nil
}

func marshalSurfaceRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode surface request: %v", err), err)
	}
	return requestJSON, nil
}

func validateSurfaceListPagesRequest(req any) error {
	value := req.(SurfaceListPagesRequest)
	if err := validateSurfaceCarrierBase(value.SurfaceCarrierBase); err != nil {
		return err
	}
	limit := value.Limit
	if limit == 0 {
		limit = DefaultSurfacePageSize
	}
	if limit < 1 || limit > MaxSurfacePageSize {
		return invalidRuntimePayload("surface page limit exceeds bounds", nil)
	}
	return nil
}

func validateSurfaceCreatePageRequest(req any) error {
	value := req.(SurfaceCreatePageRequest)
	if err := validateSurfaceCarrierBase(value.SurfaceCarrierBase); err != nil {
		return err
	}
	if err := validateSurfaceProjectID(value.ProjectID); err != nil {
		return err
	}
	if value.Folder == "" || !strings.HasPrefix(value.Folder, "/") {
		return invalidRuntimePayload("surface folder must be absolute", nil)
	}
	if value.Visibility != "" && value.Visibility != "public" && value.Visibility != "private" {
		return invalidRuntimePayload("invalid surface visibility", nil)
	}
	return nil
}

func validateSurfaceDeletePageRequest(req any) error {
	value := req.(SurfaceDeletePageRequest)
	if err := validateSurfaceCarrierBase(value.SurfaceCarrierBase); err != nil {
		return err
	}
	return validateSurfaceProjectID(value.ProjectID)
}

func validateSurfaceManifestRequest(req any) error {
	value := req.(SurfaceManifestRequest)
	if err := validateSurfaceCarrierBase(value.SurfaceCarrierBase); err != nil {
		return err
	}
	return validateSurfaceProjectID(value.ProjectID)
}

func validateSurfaceCarrierBase(base SurfaceCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidRuntimePayload("complete surface invocation carrier is required", nil)
	}
	return nil
}

func validateSurfaceProjectID(value string) error {
	if value == "" || len(value) > 64 {
		return invalidRuntimePayload("invalid surface project_id", nil)
	}
	for _, ch := range value {
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' {
			continue
		}
		return invalidRuntimePayload("invalid surface project_id", nil)
	}
	return nil
}

func validateSurfacePageRecord(record SurfacePageRecord) error {
	if record.Profile != surfaceProfile || record.Kind != "page_record" || record.PageID == "" ||
		record.OwnerURA == "" || record.SurfaceRef == "" {
		return invalidRuntimePayload("invalid surface page record projection", nil)
	}
	return nil
}

func wrapSurfaceTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
