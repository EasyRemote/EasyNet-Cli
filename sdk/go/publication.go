package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"path/filepath"
	"strings"
)

const (
	DefaultPublishedAbilityPageSize = 50
	MaxPublishedAbilityPageSize     = 500
)

const publicationProfile = "publication"

// AbilityPackageManifest is the input manifest projected by the Publication profile.
type AbilityPackageManifest struct {
	Name              string         `json:"name"`
	Namespace         string         `json:"namespace"`
	Description       string         `json:"description"`
	DescriptorVersion string         `json:"descriptor_version,omitempty"`
	InputSchema       map[string]any `json:"input_schema"`
	OutputSchema      any            `json:"output_schema,omitempty"`
	Exec              map[string]any `json:"exec,omitempty"`
}

// ValidatePackageOptions carries facade-owned validation inputs to the daemon boundary.
type ValidatePackageOptions struct {
	Manifest *AbilityPackageManifest `json:"manifest,omitempty"`
	Metadata map[string]any          `json:"metadata,omitempty"`
}

// PackageValidation is the sdk/schemas/package-validation.schema.json projection.
type PackageValidation struct {
	Profile      string                    `json:"profile"`
	Kind         string                    `json:"kind"`
	Valid        bool                      `json:"valid"`
	PackagePath  string                    `json:"package_path"`
	ManifestPath string                    `json:"manifest_path"`
	ManifestHash string                    `json:"manifest_hash"`
	Manifest     PackageValidationManifest `json:"manifest"`
	Errors       []any                     `json:"errors"`
	Metadata     map[string]any            `json:"metadata"`
}

// PackageValidationManifest is the normalized daemon-authored manifest summary.
type PackageValidationManifest struct {
	Name              string         `json:"name"`
	Namespace         string         `json:"namespace"`
	WireKey           string         `json:"wire_key"`
	DescriptorVersion string         `json:"descriptor_version"`
	Description       string         `json:"description"`
	ExecKind          string         `json:"exec_kind"`
	TimeoutSeconds    *int64         `json:"timeout_seconds"`
	InputSchema       map[string]any `json:"input_schema"`
	OutputSchema      any            `json:"output_schema"`
}

// AbilityDeployRequest is the complete carrier for daemon ability deployment.
type AbilityDeployRequest struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	ResourceRef       ResourceRef    `json:"resource_ref"`
	NodeID            string         `json:"node_id"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// AbilityDeployResult is the daemon projection after deploy execution.
type AbilityDeployResult struct {
	PublicName string `json:"public_name"`
	Namespace  string `json:"namespace"`
	AbilityURA string `json:"ability_ura"`
	NodeID     string `json:"node_id"`
	MutatedBy  string `json:"mutated_by,omitempty"`
	InstallID  string `json:"install_id"`
	Bundle     string `json:"bundle,omitempty"`
	State      string `json:"state"`
}

// AbilityImplID identifies one executable binding for enable/disable operations.
type AbilityImplID struct {
	ImplID     string         `json:"impl_id"`
	AbilityURA string         `json:"ability_ura"`
	Metadata   map[string]any `json:"metadata,omitempty"`
}

// PublishedAbility is the sdk/schemas/published-ability.schema.json projection.
type PublishedAbility struct {
	Descriptor     map[string]any `json:"descriptor"`
	Implementation map[string]any `json:"implementation"`
	Metadata       map[string]any `json:"metadata"`
}

// PublishedAbilityQuery is a bounded read-model query over published abilities.
type PublishedAbilityQuery struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	Limit             int            `json:"limit,omitempty"`
	Cursor            string         `json:"cursor,omitempty"`
	OwnerURA          string         `json:"owner_ura,omitempty"`
	AbilityURA        string         `json:"ability_ura,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// PublishedAbilityPage is a bounded daemon read-model page.
type PublishedAbilityPage struct {
	Profile    string             `json:"profile"`
	Kind       string             `json:"kind"`
	ItemKind   string             `json:"item_kind"`
	Items      []PublishedAbility `json:"items"`
	NextCursor *string            `json:"next_cursor"`
	Limit      int                `json:"limit"`
	Source     string             `json:"source"`
	Metadata   map[string]any     `json:"metadata"`
}

// DescriptorRef identifies a descriptor-bound ability version.
type DescriptorRef string

// ShowAbilityRequest asks the daemon read model for one published ability.
type ShowAbilityRequest struct {
	DescriptorRef DescriptorRef  `json:"descriptor_ref"`
	Metadata      map[string]any `json:"metadata,omitempty"`
}

// UnpublishAbilityRequest builds or executes an unpublish carrier.
type UnpublishAbilityRequest struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	AbilityURA        string         `json:"ability_ura"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

// PublicationRecord is a generic daemon publication operation projection.
type PublicationRecord struct {
	Profile       string         `json:"profile"`
	Kind          string         `json:"kind"`
	DescriptorRef string         `json:"descriptor_ref,omitempty"`
	OwnerURA      string         `json:"owner_ura,omitempty"`
	ResourceRef   *string        `json:"resource_ref"`
	Status        *string        `json:"status"`
	Metadata      map[string]any `json:"metadata"`
}

// PluginInstallResult is a daemon-authored implementation-resource management projection.
type PluginInstallResult struct {
	Profile   string         `json:"profile"`
	Kind      string         `json:"kind"`
	Source    string         `json:"source"`
	InstallID string         `json:"install_id"`
	Status    string         `json:"status"`
	Metadata  map[string]any `json:"metadata"`
}

// InstallOptions carries plugin/skill implementation-resource install options.
type InstallOptions struct {
	Metadata map[string]any `json:"metadata,omitempty"`
}

// PublicationTransport supplies daemon publication operations behind the facade.
type PublicationTransport interface {
	BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error)
	ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeployAbility(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildDeployInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error)
	ShowAbility(ctx context.Context, requestJSON []byte) ([]byte, error)
	EnableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error)
	DisableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildUnpublishInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	UnpublishAbility(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// PublicationTransportFunc adapts functions into a PublicationTransport.
type PublicationTransportFunc struct {
	BuildResourceRefFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ValidatePackageFunc          func(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeployAbilityFunc            func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildDeployInvocationFunc    func(ctx context.Context, requestJSON []byte) ([]byte, error)
	InstallPluginFunc            func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListAbilitiesFunc            func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ShowAbilityFunc              func(ctx context.Context, requestJSON []byte) ([]byte, error)
	EnableAbilityImplFunc        func(ctx context.Context, requestJSON []byte) ([]byte, error)
	DisableAbilityImplFunc       func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildUnpublishInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	UnpublishAbilityFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f PublicationTransportFunc) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildResourceRefFunc == nil {
		return nil, invalidRuntimeClient("publication resource-ref transport function is required")
	}
	return f.BuildResourceRefFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ValidatePackageFunc == nil {
		return nil, invalidRuntimeClient("publication validate-package transport function is required")
	}
	return f.ValidatePackageFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) DeployAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.DeployAbilityFunc == nil {
		return nil, invalidRuntimeClient("publication deploy transport function is required")
	}
	return f.DeployAbilityFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) BuildDeployInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildDeployInvocationFunc == nil {
		return nil, invalidRuntimeClient("publication deploy-invocation transport function is required")
	}
	return f.BuildDeployInvocationFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.InstallPluginFunc == nil {
		return nil, invalidRuntimeClient("publication install-plugin transport function is required")
	}
	return f.InstallPluginFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListAbilitiesFunc == nil {
		return nil, invalidRuntimeClient("publication list transport function is required")
	}
	return f.ListAbilitiesFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) ShowAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ShowAbilityFunc == nil {
		return nil, invalidRuntimeClient("publication show transport function is required")
	}
	return f.ShowAbilityFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) EnableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.EnableAbilityImplFunc == nil {
		return nil, invalidRuntimeClient("publication enable transport function is required")
	}
	return f.EnableAbilityImplFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) DisableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.DisableAbilityImplFunc == nil {
		return nil, invalidRuntimeClient("publication disable transport function is required")
	}
	return f.DisableAbilityImplFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) BuildUnpublishInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildUnpublishInvocationFunc == nil {
		return nil, invalidRuntimeClient("publication unpublish-invocation transport function is required")
	}
	return f.BuildUnpublishInvocationFunc(ctx, requestJSON)
}

func (f PublicationTransportFunc) UnpublishAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.UnpublishAbilityFunc == nil {
		return nil, invalidRuntimeClient("publication unpublish transport function is required")
	}
	return f.UnpublishAbilityFunc(ctx, requestJSON)
}

// PublicationClient is the Publication profile facade.
type PublicationClient struct {
	transport PublicationTransport
	lifecycle profileClientLifecycle
}

func NewPublicationClient(transport PublicationTransport) (*PublicationClient, error) {
	if transport == nil {
		return nil, invalidRuntimeClient("publication transport is required")
	}
	return &PublicationClient{transport: transport}, nil
}

func (c *PublicationClient) BuildLocalResourceRef(ctx context.Context, req LocalResourceRefRequest) (ResourceRef, error) {
	if err := c.requireReady(ctx); err != nil {
		return ResourceRef{}, err
	}
	if req.Path == "" || req.Capability == "" {
		return ResourceRef{}, invalidRuntimePayload("path and capability are required", nil)
	}
	if !filepath.IsAbs(req.Path) {
		return ResourceRef{}, invalidRuntimePayload("absolute resource path is required", nil)
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return ResourceRef{}, invalidRuntimePayload(fmt.Sprintf("encode resource-ref request: %v", err), err)
	}
	raw, err := c.transport.BuildResourceRef(ctx, requestJSON)
	if err != nil {
		return ResourceRef{}, wrapPublicationTransportError("publication resource-ref build failed", err)
	}
	return NewResourceRefFromJSON(raw)
}

func (c *PublicationClient) ValidatePackage(ctx context.Context, path string, opts ValidatePackageOptions) (PackageValidation, error) {
	if err := c.requireReady(ctx); err != nil {
		return PackageValidation{}, err
	}
	if path == "" && opts.Manifest == nil {
		return PackageValidation{}, invalidRuntimePayload("package path or manifest is required", nil)
	}
	requestJSON, err := json.Marshal(struct {
		PackagePath string                  `json:"package_path,omitempty"`
		Manifest    *AbilityPackageManifest `json:"manifest,omitempty"`
		Metadata    map[string]any          `json:"metadata,omitempty"`
	}{PackagePath: path, Manifest: opts.Manifest, Metadata: opts.Metadata})
	if err != nil {
		return PackageValidation{}, invalidRuntimePayload(fmt.Sprintf("encode package validation request: %v", err), err)
	}
	raw, err := c.transport.ValidatePackage(ctx, requestJSON)
	if err != nil {
		return PackageValidation{}, wrapPublicationTransportError("publication validate package failed", err)
	}
	return NewPackageValidationFromJSON(raw)
}

func (c *PublicationClient) DeployAbility(ctx context.Context, req AbilityDeployRequest) (AbilityDeployResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return AbilityDeployResult{}, err
	}
	requestJSON, err := marshalAbilityDeployRequest(req)
	if err != nil {
		return AbilityDeployResult{}, err
	}
	raw, err := c.transport.DeployAbility(ctx, requestJSON)
	if err != nil {
		return AbilityDeployResult{}, wrapPublicationTransportError("publication deploy failed", err)
	}
	return NewAbilityDeployResultFromJSON(raw)
}

func (c *PublicationClient) BuildDeployInvocation(ctx context.Context, req AbilityDeployRequest) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalAbilityDeployRequest(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := c.transport.BuildDeployInvocation(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapPublicationTransportError("publication deploy invocation failed", err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *PublicationClient) InstallPlugin(ctx context.Context, source string, opts InstallOptions) (PluginInstallResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return PluginInstallResult{}, err
	}
	if source == "" {
		return PluginInstallResult{}, invalidRuntimePayload("plugin source is required", nil)
	}
	requestJSON, err := json.Marshal(struct {
		Source   string         `json:"source"`
		Metadata map[string]any `json:"metadata,omitempty"`
	}{Source: source, Metadata: opts.Metadata})
	if err != nil {
		return PluginInstallResult{}, invalidRuntimePayload(fmt.Sprintf("encode plugin install request: %v", err), err)
	}
	raw, err := c.transport.InstallPlugin(ctx, requestJSON)
	if err != nil {
		return PluginInstallResult{}, wrapPublicationTransportError("publication install plugin failed", err)
	}
	return NewPluginInstallResultFromJSON(raw)
}

func (c *PublicationClient) ListAbilities(ctx context.Context, query PublishedAbilityQuery) (PublishedAbilityPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return PublishedAbilityPage{}, err
	}
	query = normalizePublishedAbilityQuery(query)
	if err := validatePublishedAbilityQuery(query); err != nil {
		return PublishedAbilityPage{}, err
	}
	requestJSON, err := json.Marshal(query)
	if err != nil {
		return PublishedAbilityPage{}, invalidRuntimePayload(fmt.Sprintf("encode publication list query: %v", err), err)
	}
	raw, err := c.transport.ListAbilities(ctx, requestJSON)
	if err != nil {
		return PublishedAbilityPage{}, wrapPublicationTransportError("publication list abilities failed", err)
	}
	return NewPublishedAbilityPageFromJSON(raw)
}

func (c *PublicationClient) ShowAbility(ctx context.Context, ref DescriptorRef) (PublishedAbility, error) {
	if err := c.requireReady(ctx); err != nil {
		return PublishedAbility{}, err
	}
	if ref == "" {
		return PublishedAbility{}, invalidRuntimePayload("descriptor_ref is required", nil)
	}
	requestJSON, err := json.Marshal(ShowAbilityRequest{DescriptorRef: ref})
	if err != nil {
		return PublishedAbility{}, invalidRuntimePayload(fmt.Sprintf("encode publication show request: %v", err), err)
	}
	raw, err := c.transport.ShowAbility(ctx, requestJSON)
	if err != nil {
		return PublishedAbility{}, wrapPublicationTransportError("publication show ability failed", err)
	}
	return NewPublishedAbilityFromJSON(raw)
}

func (c *PublicationClient) EnableAbilityImpl(ctx context.Context, id AbilityImplID) error {
	if err := c.requireReady(ctx); err != nil {
		return err
	}
	requestJSON, err := marshalAbilityImplID(id)
	if err != nil {
		return err
	}
	raw, err := c.transport.EnableAbilityImpl(ctx, requestJSON)
	if err != nil {
		return wrapPublicationTransportError("publication enable ability impl failed", err)
	}
	return validatePublicationRecord(raw, "ability_impl_enabled")
}

func (c *PublicationClient) DisableAbilityImpl(ctx context.Context, id AbilityImplID) error {
	if err := c.requireReady(ctx); err != nil {
		return err
	}
	requestJSON, err := marshalAbilityImplID(id)
	if err != nil {
		return err
	}
	raw, err := c.transport.DisableAbilityImpl(ctx, requestJSON)
	if err != nil {
		return wrapPublicationTransportError("publication disable ability impl failed", err)
	}
	return validatePublicationRecord(raw, "ability_impl_disabled")
}

func (c *PublicationClient) BuildUnpublishInvocation(ctx context.Context, req UnpublishAbilityRequest) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshalUnpublishAbilityRequest(req)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := c.transport.BuildUnpublishInvocation(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapPublicationTransportError("publication unpublish invocation failed", err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *PublicationClient) UnpublishAbility(ctx context.Context, ref DescriptorRef) error {
	if err := c.requireReady(ctx); err != nil {
		return err
	}
	if ref == "" {
		return invalidRuntimePayload("descriptor_ref is required", nil)
	}
	requestJSON, err := json.Marshal(ShowAbilityRequest{DescriptorRef: ref})
	if err != nil {
		return invalidRuntimePayload(fmt.Sprintf("encode publication unpublish request: %v", err), err)
	}
	raw, err := c.transport.UnpublishAbility(ctx, requestJSON)
	if err != nil {
		return wrapPublicationTransportError("publication unpublish ability failed", err)
	}
	return validatePublicationRecord(raw, "ability_unpublished")
}

func (c *PublicationClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("publication client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "publication")
}

func (c *PublicationClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidRuntimeClient("publication client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "publication")
}

func NewPackageValidationFromJSON(raw []byte) (PackageValidation, error) {
	var validation PackageValidation
	if err := json.Unmarshal(raw, &validation); err != nil {
		return PackageValidation{}, invalidRuntimePayload(fmt.Sprintf("decode package validation JSON: %v", err), err)
	}
	if validation.Profile != publicationProfile || validation.Kind != "package_validation" ||
		validation.PackagePath == "" || validation.ManifestPath == "" ||
		validation.ManifestHash == "" || validation.Manifest.Name == "" ||
		validation.Manifest.Namespace == "" || validation.Manifest.WireKey == "" ||
		validation.Manifest.DescriptorVersion == "" || validation.Manifest.ExecKind == "" ||
		validation.Manifest.InputSchema == nil || validation.Errors == nil || validation.Metadata == nil {
		return PackageValidation{}, invalidRuntimePayload("invalid package validation projection", nil)
	}
	return validation, nil
}

func NewAbilityDeployResultFromJSON(raw []byte) (AbilityDeployResult, error) {
	var result AbilityDeployResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return AbilityDeployResult{}, invalidRuntimePayload(fmt.Sprintf("decode ability deploy result JSON: %v", err), err)
	}
	if result.PublicName == "" || result.Namespace == "" || result.AbilityURA == "" ||
		result.NodeID == "" || result.InstallID == "" || result.State == "" {
		return AbilityDeployResult{}, invalidRuntimePayload("invalid ability deploy result projection", nil)
	}
	return result, nil
}

func NewPublishedAbilityFromJSON(raw []byte) (PublishedAbility, error) {
	var ability PublishedAbility
	if err := json.Unmarshal(raw, &ability); err != nil {
		return PublishedAbility{}, invalidRuntimePayload(fmt.Sprintf("decode published ability JSON: %v", err), err)
	}
	if ability.Descriptor == nil || ability.Implementation == nil || ability.Metadata == nil {
		return PublishedAbility{}, invalidRuntimePayload("invalid published ability projection", nil)
	}
	return ability, nil
}

func NewPublishedAbilityPageFromJSON(raw []byte) (PublishedAbilityPage, error) {
	var page PublishedAbilityPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return PublishedAbilityPage{}, invalidRuntimePayload(fmt.Sprintf("decode published ability page JSON: %v", err), err)
	}
	if page.Profile != publicationProfile || page.Kind == "" || page.ItemKind != "published_ability" ||
		page.Source == "" || page.Limit <= 0 || page.Limit > MaxPublishedAbilityPageSize || page.Items == nil || page.Metadata == nil {
		return PublishedAbilityPage{}, invalidRuntimePayload("invalid published ability page projection", nil)
	}
	for _, item := range page.Items {
		if item.Descriptor == nil || item.Implementation == nil || item.Metadata == nil {
			return PublishedAbilityPage{}, invalidRuntimePayload("invalid published ability item projection", nil)
		}
	}
	return page, nil
}

func NewPluginInstallResultFromJSON(raw []byte) (PluginInstallResult, error) {
	var result PluginInstallResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return PluginInstallResult{}, invalidRuntimePayload(fmt.Sprintf("decode plugin install result JSON: %v", err), err)
	}
	if result.Profile != publicationProfile || result.Kind == "" || result.Source == "" ||
		result.InstallID == "" || result.Status == "" || result.Metadata == nil {
		return PluginInstallResult{}, invalidRuntimePayload("invalid plugin install projection", nil)
	}
	return result, nil
}

func marshalAbilityDeployRequest(req AbilityDeployRequest) ([]byte, error) {
	if req.CallerURA == "" || req.CalleeURA == "" || req.SubjectURA == "" ||
		req.DescriptorVersion == "" || req.NonceBase64 == "" || req.NodeID == "" ||
		req.CausalContext == nil {
		return nil, invalidRuntimePayload("complete deploy invocation carrier is required", nil)
	}
	if err := validatePublicationResourceRef(req.ResourceRef); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode ability deploy request: %v", err), err)
	}
	return requestJSON, nil
}

func validatePublicationResourceRef(ref ResourceRef) error {
	if ref.ResourceURA == "" || ref.OwnerURA == "" ||
		ref.Namespace == "" || ref.Capability == "" ||
		ref.Revision == "" {
		return invalidRuntimePayload("valid resource_ref is required", nil)
	}
	switch strings.ToLower(ref.Namespace) {
	case "axon", "daemon", "easynet", "internal", "system":
		return invalidRuntimePayload("resource_ref namespace is reserved", nil)
	}
	return nil
}

func marshalAbilityImplID(id AbilityImplID) ([]byte, error) {
	if id.ImplID == "" || id.AbilityURA == "" {
		return nil, invalidRuntimePayload("impl_id and ability_ura are required", nil)
	}
	requestJSON, err := json.Marshal(id)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode ability impl id: %v", err), err)
	}
	return requestJSON, nil
}

func marshalUnpublishAbilityRequest(req UnpublishAbilityRequest) ([]byte, error) {
	if req.CallerURA == "" || req.CalleeURA == "" || req.SubjectURA == "" ||
		req.DescriptorVersion == "" || req.NonceBase64 == "" || req.CausalContext == nil ||
		req.AbilityURA == "" {
		return nil, invalidRuntimePayload("complete unpublish invocation carrier is required", nil)
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode unpublish request: %v", err), err)
	}
	return requestJSON, nil
}

func normalizePublishedAbilityQuery(query PublishedAbilityQuery) PublishedAbilityQuery {
	if query.Limit == 0 {
		query.Limit = DefaultPublishedAbilityPageSize
	}
	return query
}

func validatePublishedAbilityQuery(query PublishedAbilityQuery) error {
	if query.CallerURA == "" || query.CalleeURA == "" || query.SubjectURA == "" ||
		query.DescriptorVersion == "" || query.NonceBase64 == "" || query.CausalContext == nil {
		return invalidRuntimePayload("complete publication query carrier is required", nil)
	}
	if query.Limit <= 0 || query.Limit > MaxPublishedAbilityPageSize {
		return invalidRuntimePayload("publication query limit exceeds bounds", nil)
	}
	return nil
}

func validatePublicationRecord(raw []byte, expectedKind string) error {
	var record PublicationRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return invalidRuntimePayload(fmt.Sprintf("decode publication record JSON: %v", err), err)
	}
	if record.Profile != publicationProfile || record.Kind != expectedKind || record.Metadata == nil {
		return invalidRuntimePayload("invalid publication record projection", nil)
	}
	return nil
}

func wrapPublicationTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return sdkErr
	}
	return transportRuntimeError(message, cause)
}
