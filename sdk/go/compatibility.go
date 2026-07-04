package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const compatibilityProfile = "compatibility"

// CompatibilityCarrierBase is the complete carrier context shared by Compatibility operations.
type CompatibilityCarrierBase struct {
	CallerURA         string         `json:"caller_ura"`
	CalleeURA         string         `json:"callee_ura"`
	SubjectURA        string         `json:"subject_ura"`
	DescriptorVersion string         `json:"descriptor_version"`
	NonceBase64       string         `json:"nonce_base64"`
	CausalContext     map[string]any `json:"causal_context"`
	AuthToken         string         `json:"auth_token,omitempty"`
	Metadata          map[string]any `json:"metadata,omitempty"`
}

type CompatibilityListModelsRequest struct {
	CompatibilityCarrierBase
}

type CompatibilityChatCompletionRequest struct {
	CompatibilityCarrierBase
	Request map[string]any `json:"request"`
}

type CompatibilityStreamChatCompletionRequest struct {
	CompatibilityCarrierBase
	Request map[string]any `json:"request"`
}

type CompatibilityFileUploadRequest struct {
	CompatibilityCarrierBase
	ID          string         `json:"id,omitempty"`
	FileID      string         `json:"file_id,omitempty"`
	FileRef     string         `json:"file_ref,omitempty"`
	ResourceRef string         `json:"resource_ref,omitempty"`
	ResourceURA string         `json:"resource_ura,omitempty"`
	Filename    string         `json:"filename,omitempty"`
	Purpose     string         `json:"purpose"`
	OwnerURA    string         `json:"owner_ura,omitempty"`
	ContentType string         `json:"content_type,omitempty"`
	ContentHash string         `json:"content_hash,omitempty"`
	Bytes       int64          `json:"bytes,omitempty"`
	SizeBytes   int64          `json:"size_bytes,omitempty"`
	CreatedAt   int64          `json:"created_at,omitempty"`
	Status      string         `json:"status,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type CompatibilityFileRequest struct {
	CompatibilityCarrierBase
	ID          string         `json:"id,omitempty"`
	FileID      string         `json:"file_id,omitempty"`
	FileRef     string         `json:"file_ref,omitempty"`
	ResourceRef string         `json:"resource_ref,omitempty"`
	ResourceURA string         `json:"resource_ura,omitempty"`
	Filename    string         `json:"filename,omitempty"`
	Purpose     string         `json:"purpose,omitempty"`
	OwnerURA    string         `json:"owner_ura,omitempty"`
	ContentType string         `json:"content_type,omitempty"`
	ContentHash string         `json:"content_hash,omitempty"`
	Bytes       int64          `json:"bytes,omitempty"`
	SizeBytes   int64          `json:"size_bytes,omitempty"`
	CreatedAt   int64          `json:"created_at,omitempty"`
	Created     int64          `json:"created,omitempty"`
	Status      string         `json:"status,omitempty"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type CompatibilityFileDeleteRequest struct {
	CompatibilityCarrierBase
	ID          string         `json:"id,omitempty"`
	FileID      string         `json:"file_id,omitempty"`
	FileRef     string         `json:"file_ref,omitempty"`
	ResourceRef string         `json:"resource_ref,omitempty"`
	ResourceURA string         `json:"resource_ura,omitempty"`
	ContentHash string         `json:"content_hash,omitempty"`
	Deleted     bool           `json:"deleted"`
	Metadata    map[string]any `json:"metadata,omitempty"`
}

type ListModelsRequest = CompatibilityListModelsRequest
type ChatCompletionRequest = CompatibilityChatCompletionRequest
type StreamChatCompletionRequest = CompatibilityStreamChatCompletionRequest

type CompatibilityModel struct {
	Profile    string         `json:"profile"`
	Kind       string         `json:"kind"`
	ID         string         `json:"id"`
	Object     string         `json:"object"`
	Created    int64          `json:"created"`
	OwnedBy    string         `json:"owned_by"`
	AbilityRef string         `json:"ability_ref"`
	Metadata   map[string]any `json:"metadata"`
}

type CompatibilityModelPage struct {
	Profile    string               `json:"profile"`
	Kind       string               `json:"kind"`
	Object     string               `json:"object"`
	Data       []CompatibilityModel `json:"data"`
	NextCursor *string              `json:"next_cursor"`
	Metadata   map[string]any       `json:"metadata"`
}

type CompatibilityChatCompletion struct {
	Profile  string           `json:"profile"`
	Kind     string           `json:"kind"`
	ID       string           `json:"id"`
	Object   string           `json:"object"`
	Created  int64            `json:"created"`
	Model    string           `json:"model"`
	Choices  []map[string]any `json:"choices"`
	Usage    map[string]any   `json:"usage"`
	Metadata map[string]any   `json:"metadata"`
}

type CompatibilityChatCompletionChunk struct {
	Profile  string           `json:"profile"`
	Kind     string           `json:"kind"`
	ID       string           `json:"id"`
	Object   string           `json:"object"`
	Created  int64            `json:"created"`
	Model    string           `json:"model"`
	Choices  []map[string]any `json:"choices"`
	Usage    any              `json:"usage"`
	Metadata map[string]any   `json:"metadata"`
}

type CompatibilityChatCompletionStream struct {
	Profile      string                             `json:"profile"`
	Kind         string                             `json:"kind"`
	Stream       bool                               `json:"stream"`
	Items        []CompatibilityChatCompletionChunk `json:"items"`
	DoneSentinel string                             `json:"done_sentinel"`
	Metadata     map[string]any                     `json:"metadata"`
}

type CompatibilityFile struct {
	Profile   string         `json:"profile"`
	Kind      string         `json:"kind"`
	ID        string         `json:"id"`
	Object    string         `json:"object"`
	Bytes     int64          `json:"bytes"`
	CreatedAt int64          `json:"created_at"`
	Filename  string         `json:"filename"`
	Purpose   string         `json:"purpose"`
	Status    string         `json:"status"`
	Metadata  map[string]any `json:"metadata"`
}

type CompatibilityFileDeleteResult struct {
	Profile  string         `json:"profile"`
	Kind     string         `json:"kind"`
	ID       string         `json:"id"`
	Object   string         `json:"object"`
	Deleted  bool           `json:"deleted"`
	Metadata map[string]any `json:"metadata"`
}

// CompatibilityTransport supplies daemon Compatibility operations behind the facade.
type CompatibilityTransport interface {
	BuildListModelsInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildStreamChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileUploadInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileRetrieveInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileDeleteInvocation(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListModels(ctx context.Context, requestJSON []byte) ([]byte, error)
	CreateChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error)
	StreamChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error)
	UploadFile(ctx context.Context, requestJSON []byte) ([]byte, error)
	RetrieveFile(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeleteFile(ctx context.Context, requestJSON []byte) ([]byte, error)
}

// CompatibilityTransportFunc adapts functions into a CompatibilityTransport.
type CompatibilityTransportFunc struct {
	BuildListModelsInvocationFunc           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildChatCompletionInvocationFunc       func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildStreamChatCompletionInvocationFunc func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileUploadInvocationFunc           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileRetrieveInvocationFunc         func(ctx context.Context, requestJSON []byte) ([]byte, error)
	BuildFileDeleteInvocationFunc           func(ctx context.Context, requestJSON []byte) ([]byte, error)
	ListModelsFunc                          func(ctx context.Context, requestJSON []byte) ([]byte, error)
	CreateChatCompletionFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	StreamChatCompletionFunc                func(ctx context.Context, requestJSON []byte) ([]byte, error)
	UploadFileFunc                          func(ctx context.Context, requestJSON []byte) ([]byte, error)
	RetrieveFileFunc                        func(ctx context.Context, requestJSON []byte) ([]byte, error)
	DeleteFileFunc                          func(ctx context.Context, requestJSON []byte) ([]byte, error)
}

func (f CompatibilityTransportFunc) BuildListModelsInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildListModelsInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility list-models invocation transport function is required")
	}
	return f.BuildListModelsInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) BuildChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildChatCompletionInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility chat-completion invocation transport function is required")
	}
	return f.BuildChatCompletionInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) BuildStreamChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildStreamChatCompletionInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility stream-chat-completion invocation transport function is required")
	}
	return f.BuildStreamChatCompletionInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) BuildFileUploadInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildFileUploadInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-upload invocation transport function is required")
	}
	return f.BuildFileUploadInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) BuildFileRetrieveInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildFileRetrieveInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-retrieve invocation transport function is required")
	}
	return f.BuildFileRetrieveInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) BuildFileDeleteInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.BuildFileDeleteInvocationFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-delete invocation transport function is required")
	}
	return f.BuildFileDeleteInvocationFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) ListModels(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.ListModelsFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility list-models transport function is required")
	}
	return f.ListModelsFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) CreateChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.CreateChatCompletionFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility chat-completion transport function is required")
	}
	return f.CreateChatCompletionFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) StreamChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.StreamChatCompletionFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility stream-chat-completion transport function is required")
	}
	return f.StreamChatCompletionFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) UploadFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.UploadFileFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-upload transport function is required")
	}
	return f.UploadFileFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) RetrieveFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.RetrieveFileFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-retrieve transport function is required")
	}
	return f.RetrieveFileFunc(ctx, requestJSON)
}

func (f CompatibilityTransportFunc) DeleteFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	if f.DeleteFileFunc == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility file-delete transport function is required")
	}
	return f.DeleteFileFunc(ctx, requestJSON)
}

// CompatibilityClient is the Compatibility profile facade.
type CompatibilityClient struct {
	transport CompatibilityTransport
	lifecycle profileClientLifecycle
}

func NewCompatibilityClient(transport CompatibilityTransport) (*CompatibilityClient, error) {
	if transport == nil {
		return nil, invalidProfileClient(compatibilityProfile, "compatibility transport is required")
	}
	return &CompatibilityClient{transport: transport}, nil
}

func (c *CompatibilityClient) BuildListModelsInvocation(ctx context.Context, req CompatibilityListModelsRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityListModelsRequest, marshalCompatibilityStruct, c.transport.BuildListModelsInvocation, "compatibility list-models invocation failed")
}

func (c *CompatibilityClient) BuildChatCompletionInvocation(ctx context.Context, req CompatibilityChatCompletionRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityChatCompletionRequest, marshalCompatibilityStruct, c.transport.BuildChatCompletionInvocation, "compatibility chat-completion invocation failed")
}

func (c *CompatibilityClient) BuildStreamChatCompletionInvocation(ctx context.Context, req CompatibilityStreamChatCompletionRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityStreamChatCompletionRequest, marshalCompatibilityStreamRequest, c.transport.BuildStreamChatCompletionInvocation, "compatibility stream-chat-completion invocation failed")
}

func (c *CompatibilityClient) BuildFileUploadInvocation(ctx context.Context, req CompatibilityFileUploadRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityFileUploadCarrierRequest, marshalCompatibilityFileUploadRequest, c.transport.BuildFileUploadInvocation, "compatibility file-upload invocation failed")
}

func (c *CompatibilityClient) BuildFileRetrieveInvocation(ctx context.Context, req CompatibilityFileRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityFileCarrierRequest, marshalCompatibilityFileRequest, c.transport.BuildFileRetrieveInvocation, "compatibility file-retrieve invocation failed")
}

func (c *CompatibilityClient) BuildFileGetInvocation(ctx context.Context, req CompatibilityFileRequest) (InvocationDraft, error) {
	return c.BuildFileRetrieveInvocation(ctx, req)
}

func (c *CompatibilityClient) BuildFileDeleteInvocation(ctx context.Context, req CompatibilityFileDeleteRequest) (InvocationDraft, error) {
	return c.buildInvocation(ctx, req, validateCompatibilityFileDeleteCarrierRequest, marshalCompatibilityFileDeleteRequest, c.transport.BuildFileDeleteInvocation, "compatibility file-delete invocation failed")
}

func (c *CompatibilityClient) ListModels(ctx context.Context, req CompatibilityListModelsRequest) (CompatibilityModelPage, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityModelPage{}, err
	}
	requestJSON, err := marshalCompatibilityStruct(req, validateCompatibilityListModelsRequest)
	if err != nil {
		return CompatibilityModelPage{}, err
	}
	raw, err := c.transport.ListModels(ctx, requestJSON)
	if err != nil {
		return CompatibilityModelPage{}, wrapCompatibilityTransportError("compatibility list models failed", err)
	}
	return NewCompatibilityModelPageFromJSON(raw)
}

func (c *CompatibilityClient) CreateChatCompletion(ctx context.Context, req CompatibilityChatCompletionRequest) (CompatibilityChatCompletion, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityChatCompletion{}, err
	}
	requestJSON, err := marshalCompatibilityStruct(req, validateCompatibilityChatCompletionRequest)
	if err != nil {
		return CompatibilityChatCompletion{}, err
	}
	raw, err := c.transport.CreateChatCompletion(ctx, requestJSON)
	if err != nil {
		return CompatibilityChatCompletion{}, wrapCompatibilityTransportError("compatibility chat completion failed", err)
	}
	return NewCompatibilityChatCompletionFromJSON(raw)
}

func (c *CompatibilityClient) StreamChatCompletion(ctx context.Context, req CompatibilityStreamChatCompletionRequest) (CompatibilityChatCompletionStream, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityChatCompletionStream{}, err
	}
	requestJSON, err := marshalCompatibilityStreamRequest(req, validateCompatibilityStreamChatCompletionRequest)
	if err != nil {
		return CompatibilityChatCompletionStream{}, err
	}
	raw, err := c.transport.StreamChatCompletion(ctx, requestJSON)
	if err != nil {
		return CompatibilityChatCompletionStream{}, wrapCompatibilityTransportError("compatibility stream chat completion failed", err)
	}
	return NewCompatibilityChatCompletionStreamFromJSON(raw)
}

func (c *CompatibilityClient) UploadFile(ctx context.Context, req CompatibilityFileUploadRequest) (CompatibilityFile, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityFile{}, err
	}
	requestJSON, err := marshalCompatibilityFileUploadRequest(req, validateCompatibilityFileUploadCarrierRequest)
	if err != nil {
		return CompatibilityFile{}, err
	}
	raw, err := c.transport.UploadFile(ctx, requestJSON)
	if err != nil {
		return CompatibilityFile{}, wrapCompatibilityTransportError("compatibility file upload failed", err)
	}
	return NewCompatibilityFileFromJSON(raw)
}

func (c *CompatibilityClient) RetrieveFile(ctx context.Context, req CompatibilityFileRequest) (CompatibilityFile, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityFile{}, err
	}
	requestJSON, err := marshalCompatibilityFileRequest(req, validateCompatibilityFileCarrierRequest)
	if err != nil {
		return CompatibilityFile{}, err
	}
	raw, err := c.transport.RetrieveFile(ctx, requestJSON)
	if err != nil {
		return CompatibilityFile{}, wrapCompatibilityTransportError("compatibility file retrieve failed", err)
	}
	return NewCompatibilityFileFromJSON(raw)
}

func (c *CompatibilityClient) GetFile(ctx context.Context, req CompatibilityFileRequest) (CompatibilityFile, error) {
	return c.RetrieveFile(ctx, req)
}

func (c *CompatibilityClient) DeleteFile(ctx context.Context, req CompatibilityFileDeleteRequest) (CompatibilityFileDeleteResult, error) {
	if err := c.requireReady(ctx); err != nil {
		return CompatibilityFileDeleteResult{}, err
	}
	requestJSON, err := marshalCompatibilityFileDeleteRequest(req, validateCompatibilityFileDeleteCarrierRequest)
	if err != nil {
		return CompatibilityFileDeleteResult{}, err
	}
	raw, err := c.transport.DeleteFile(ctx, requestJSON)
	if err != nil {
		return CompatibilityFileDeleteResult{}, wrapCompatibilityTransportError("compatibility file delete failed", err)
	}
	return NewCompatibilityFileDeleteResultFromJSON(raw)
}

func (c *CompatibilityClient) ProjectFileUpload(req CompatibilityFileUploadRequest) (CompatibilityFile, error) {
	if err := validateCompatibilityFileUploadRequest(req); err != nil {
		return CompatibilityFile{}, err
	}
	return compatibilityFileFromFacts(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.Filename, req.Purpose, req.OwnerURA, req.ContentType, req.ContentHash, req.Bytes, req.SizeBytes, req.CreatedAt, 0, req.Status)
}

func (c *CompatibilityClient) ProjectFile(req CompatibilityFileRequest) (CompatibilityFile, error) {
	if err := validateCompatibilityFileRequest(req); err != nil {
		return CompatibilityFile{}, err
	}
	return compatibilityFileFromFacts(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.Filename, req.Purpose, req.OwnerURA, req.ContentType, req.ContentHash, req.Bytes, req.SizeBytes, req.CreatedAt, req.Created, req.Status)
}

func (c *CompatibilityClient) ProjectFileDeleteResult(req CompatibilityFileDeleteRequest) (CompatibilityFileDeleteResult, error) {
	if err := validateCompatibilityFileDeleteRequest(req); err != nil {
		return CompatibilityFileDeleteResult{}, err
	}
	return CompatibilityFileDeleteResult{
		Profile: compatibilityProfile,
		Kind:    "file_delete_result",
		ID:      firstNonEmpty(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.ContentHash),
		Object:  "file",
		Deleted: req.Deleted,
		Metadata: map[string]any{
			"profile": compatibilityProfile,
			"source":  "compatibility.file_delete",
		},
	}, nil
}

func (c *CompatibilityClient) Close(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(compatibilityProfile, "compatibility client is not initialized")
	}
	return c.lifecycle.Close(ctx, c.transport, "compatibility")
}

func (c *CompatibilityClient) buildInvocation(ctx context.Context, req any, validate func(any) error, marshal func(any, func(any) error) ([]byte, error), fn func(context.Context, []byte) ([]byte, error), label string) (InvocationDraft, error) {
	if err := c.requireReady(ctx); err != nil {
		return InvocationDraft{}, err
	}
	requestJSON, err := marshal(req, validate)
	if err != nil {
		return InvocationDraft{}, err
	}
	raw, err := fn(ctx, requestJSON)
	if err != nil {
		return InvocationDraft{}, wrapCompatibilityTransportError(label, err)
	}
	return NewInvocationDraftFromJSON(raw)
}

func (c *CompatibilityClient) requireReady(ctx context.Context) error {
	if c == nil || c.transport == nil {
		return invalidProfileClient(compatibilityProfile, "compatibility client is not initialized")
	}
	return c.lifecycle.RequireOpen(ctx, "compatibility")
}

func NewCompatibilityModelPageFromJSON(raw []byte) (CompatibilityModelPage, error) {
	var page CompatibilityModelPage
	if err := json.Unmarshal(raw, &page); err != nil {
		return CompatibilityModelPage{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility model page JSON: %v", err), err)
	}
	if page.Profile != compatibilityProfile || page.Kind != "model_page" || page.Object != "list" || page.Data == nil || page.Metadata == nil {
		return CompatibilityModelPage{}, invalidProfilePayload(compatibilityProfile, "invalid compatibility model page projection", nil)
	}
	for _, model := range page.Data {
		if err := validateCompatibilityModel(model); err != nil {
			return CompatibilityModelPage{}, err
		}
	}
	return page, nil
}

func NewCompatibilityChatCompletionFromJSON(raw []byte) (CompatibilityChatCompletion, error) {
	var completion CompatibilityChatCompletion
	if err := json.Unmarshal(raw, &completion); err != nil {
		return CompatibilityChatCompletion{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility chat completion JSON: %v", err), err)
	}
	if completion.Profile != compatibilityProfile || completion.Kind != "chat_completion" || completion.Object != "chat.completion" ||
		completion.ID == "" || completion.Created < 0 || completion.Model == "" || completion.Choices == nil || completion.Metadata == nil {
		return CompatibilityChatCompletion{}, invalidProfilePayload(compatibilityProfile, "invalid compatibility chat completion projection", nil)
	}
	return completion, nil
}

func NewCompatibilityChatCompletionStreamFromJSON(raw []byte) (CompatibilityChatCompletionStream, error) {
	var stream CompatibilityChatCompletionStream
	if err := json.Unmarshal(raw, &stream); err != nil {
		return CompatibilityChatCompletionStream{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility chat stream JSON: %v", err), err)
	}
	if stream.Profile != compatibilityProfile || stream.Kind != "chat_completion_stream" || !stream.Stream ||
		stream.DoneSentinel != "[DONE]" || stream.Items == nil || stream.Metadata == nil {
		return CompatibilityChatCompletionStream{}, invalidProfilePayload(compatibilityProfile, "invalid compatibility chat stream projection", nil)
	}
	for _, item := range stream.Items {
		if item.Profile != compatibilityProfile || item.Kind != "chat_completion_chunk" ||
			item.Object != "chat.completion.chunk" || item.ID == "" || item.Created < 0 ||
			item.Model == "" || item.Choices == nil || item.Metadata == nil {
			return CompatibilityChatCompletionStream{}, invalidProfilePayload(compatibilityProfile, "invalid compatibility chat stream chunk projection", nil)
		}
	}
	return stream, nil
}

func NewCompatibilityFileFromJSON(raw []byte) (CompatibilityFile, error) {
	var file CompatibilityFile
	if err := json.Unmarshal(raw, &file); err != nil {
		return CompatibilityFile{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility file JSON: %v", err), err)
	}
	if err := validateCompatibilityFileProjection(file); err != nil {
		return CompatibilityFile{}, err
	}
	return file, nil
}

func NewCompatibilityFileDeleteResultFromJSON(raw []byte) (CompatibilityFileDeleteResult, error) {
	var result CompatibilityFileDeleteResult
	if err := json.Unmarshal(raw, &result); err != nil {
		return CompatibilityFileDeleteResult{}, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("decode compatibility file delete result JSON: %v", err), err)
	}
	if result.Profile != compatibilityProfile || result.Kind != "file_delete_result" || result.Object != "file" ||
		result.ID == "" || !result.Deleted || result.Metadata == nil {
		return CompatibilityFileDeleteResult{}, invalidProfilePayload(compatibilityProfile, "invalid compatibility file delete projection", nil)
	}
	return result, nil
}

func marshalCompatibilityStruct(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	requestJSON, err := json.Marshal(req)
	if err != nil {
		return nil, invalidProfilePayload(compatibilityProfile, fmt.Sprintf("encode compatibility request: %v", err), err)
	}
	return requestJSON, nil
}

func marshalCompatibilityStreamRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	value := req.(CompatibilityStreamChatCompletionRequest)
	request := copyStringAnyMap(value.Request)
	request["stream"] = true
	carrier := compatibilityBaseMap(value.CompatibilityCarrierBase)
	carrier["request"] = request
	return json.Marshal(carrier)
}

func marshalCompatibilityFileUploadRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	value := req.(CompatibilityFileUploadRequest)
	payload := compatibilityBaseMap(value.CompatibilityCarrierBase)
	putNonEmpty(payload, "id", value.ID)
	putNonEmpty(payload, "file_id", value.FileID)
	putNonEmpty(payload, "file_ref", value.FileRef)
	putNonEmpty(payload, "resource_ref", value.ResourceRef)
	putNonEmpty(payload, "resource_ura", value.ResourceURA)
	putNonEmpty(payload, "filename", value.Filename)
	payload["purpose"] = value.Purpose
	putNonEmpty(payload, "owner_ura", value.OwnerURA)
	putNonEmpty(payload, "content_type", value.ContentType)
	putNonEmpty(payload, "content_hash", value.ContentHash)
	putNonZeroInt64(payload, "bytes", value.Bytes)
	putNonZeroInt64(payload, "size_bytes", value.SizeBytes)
	putNonZeroInt64(payload, "created_at", value.CreatedAt)
	putNonEmpty(payload, "status", value.Status)
	mergeCompatibilityMetadata(payload, value.Metadata)
	return json.Marshal(payload)
}

func marshalCompatibilityFileRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	value := req.(CompatibilityFileRequest)
	payload := compatibilityBaseMap(value.CompatibilityCarrierBase)
	putNonEmpty(payload, "id", value.ID)
	putNonEmpty(payload, "file_id", value.FileID)
	putNonEmpty(payload, "file_ref", value.FileRef)
	putNonEmpty(payload, "resource_ref", value.ResourceRef)
	putNonEmpty(payload, "resource_ura", value.ResourceURA)
	putNonEmpty(payload, "filename", value.Filename)
	putNonEmpty(payload, "purpose", value.Purpose)
	putNonEmpty(payload, "owner_ura", value.OwnerURA)
	putNonEmpty(payload, "content_type", value.ContentType)
	putNonEmpty(payload, "content_hash", value.ContentHash)
	putNonZeroInt64(payload, "bytes", value.Bytes)
	putNonZeroInt64(payload, "size_bytes", value.SizeBytes)
	putNonZeroInt64(payload, "created_at", value.CreatedAt)
	putNonZeroInt64(payload, "created", value.Created)
	putNonEmpty(payload, "status", value.Status)
	mergeCompatibilityMetadata(payload, value.Metadata)
	return json.Marshal(payload)
}

func marshalCompatibilityFileDeleteRequest(req any, validate func(any) error) ([]byte, error) {
	if err := validate(req); err != nil {
		return nil, err
	}
	value := req.(CompatibilityFileDeleteRequest)
	payload := compatibilityBaseMap(value.CompatibilityCarrierBase)
	putNonEmpty(payload, "id", value.ID)
	putNonEmpty(payload, "file_id", value.FileID)
	putNonEmpty(payload, "file_ref", value.FileRef)
	putNonEmpty(payload, "resource_ref", value.ResourceRef)
	putNonEmpty(payload, "resource_ura", value.ResourceURA)
	putNonEmpty(payload, "content_hash", value.ContentHash)
	payload["deleted"] = value.Deleted
	mergeCompatibilityMetadata(payload, value.Metadata)
	return json.Marshal(payload)
}

func validateCompatibilityListModelsRequest(req any) error {
	value := req.(CompatibilityListModelsRequest)
	return validateCompatibilityCarrierBase(value.CompatibilityCarrierBase)
}

func validateCompatibilityChatCompletionRequest(req any) error {
	value := req.(CompatibilityChatCompletionRequest)
	if err := validateCompatibilityCarrierBase(value.CompatibilityCarrierBase); err != nil {
		return err
	}
	if err := validateCompatibilityChatRequest(value.Request); err != nil {
		return err
	}
	if stream, ok := value.Request["stream"].(bool); ok && stream {
		return invalidProfilePayload(compatibilityProfile, "unary chat completion request must not set stream=true", nil)
	}
	return nil
}

func validateCompatibilityStreamChatCompletionRequest(req any) error {
	value := req.(CompatibilityStreamChatCompletionRequest)
	if err := validateCompatibilityCarrierBase(value.CompatibilityCarrierBase); err != nil {
		return err
	}
	return validateCompatibilityChatRequest(value.Request)
}

func validateCompatibilityCarrierBase(base CompatibilityCarrierBase) error {
	if base.CallerURA == "" || base.CalleeURA == "" || base.SubjectURA == "" ||
		base.DescriptorVersion == "" || base.NonceBase64 == "" || base.CausalContext == nil {
		return invalidProfilePayload(compatibilityProfile, "complete compatibility invocation carrier is required", nil)
	}
	return nil
}

func validateCompatibilityChatRequest(request map[string]any) error {
	if request == nil {
		return invalidProfilePayload(compatibilityProfile, "compatibility chat request is required", nil)
	}
	model, ok := request["model"].(string)
	if !ok || model == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility model is required", nil)
	}
	if err := validateCompatibilityAbilityModel(model); err != nil {
		return err
	}
	messages, ok := request["messages"].([]any)
	if !ok || len(messages) == 0 {
		return invalidProfilePayload(compatibilityProfile, "compatibility messages are required", nil)
	}
	return nil
}

func validateCompatibilityAbilityModel(model string) error {
	if strings.HasPrefix(model, "easynet://") && strings.Contains(model, "/ability/") {
		return nil
	}
	return invalidProfilePayload(compatibilityProfile, "compatibility model must be an EasyNet ability ref", nil)
}

func validateCompatibilityModel(model CompatibilityModel) error {
	if model.Profile != compatibilityProfile || model.Kind != "model" || model.Object != "model" ||
		model.ID == "" || model.OwnedBy == "" || model.AbilityRef == "" || model.Metadata == nil {
		return invalidProfilePayload(compatibilityProfile, "invalid compatibility model projection", nil)
	}
	if model.Created < 0 {
		return invalidProfilePayload(compatibilityProfile, "compatibility model created must be non-negative", nil)
	}
	return nil
}

func validateCompatibilityFileUploadRequest(req CompatibilityFileUploadRequest) error {
	if req.Purpose == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility file purpose is required", nil)
	}
	return validateCompatibilityFileFacts(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.Filename, req.Bytes, req.SizeBytes, req.CreatedAt, 0)
}

func validateCompatibilityFileUploadCarrierRequest(req any) error {
	value := req.(CompatibilityFileUploadRequest)
	if err := validateCompatibilityCarrierBase(value.CompatibilityCarrierBase); err != nil {
		return err
	}
	return validateCompatibilityFileUploadRequest(value)
}

func validateCompatibilityFileRequest(req CompatibilityFileRequest) error {
	return validateCompatibilityFileFacts(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.Filename, req.Bytes, req.SizeBytes, req.CreatedAt, req.Created)
}

func validateCompatibilityFileCarrierRequest(req any) error {
	value := req.(CompatibilityFileRequest)
	if err := validateCompatibilityCarrierBase(value.CompatibilityCarrierBase); err != nil {
		return err
	}
	return validateCompatibilityFileRequest(value)
}

func validateCompatibilityFileDeleteRequest(req CompatibilityFileDeleteRequest) error {
	if firstNonEmpty(req.ID, req.FileID, req.FileRef, req.ResourceRef, req.ResourceURA, req.ContentHash) == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility file identity is required", nil)
	}
	if !req.Deleted {
		return invalidProfilePayload(compatibilityProfile, "compatibility file delete result must be deleted", nil)
	}
	return nil
}

func validateCompatibilityFileDeleteCarrierRequest(req any) error {
	value := req.(CompatibilityFileDeleteRequest)
	if err := validateCompatibilityCarrierBase(value.CompatibilityCarrierBase); err != nil {
		return err
	}
	return validateCompatibilityFileDeleteRequest(value)
}

func validateCompatibilityFileFacts(id, fileID, fileRef, resourceRef, resourceURA, filename string, bytesValue, sizeBytes, createdAt, created int64) error {
	if firstNonEmpty(id, fileID) == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility file id is required", nil)
	}
	if firstNonEmpty(fileRef, resourceRef, resourceURA) == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility file ref is required", nil)
	}
	if filename == "" {
		return invalidProfilePayload(compatibilityProfile, "compatibility filename is required", nil)
	}
	if bytesValue < 0 || sizeBytes < 0 || createdAt < 0 || created < 0 {
		return invalidProfilePayload(compatibilityProfile, "compatibility file counters must be non-negative", nil)
	}
	return nil
}

func validateCompatibilityFileProjection(file CompatibilityFile) error {
	if file.Profile != compatibilityProfile || file.Kind != "file" || file.Object != "file" ||
		file.ID == "" || file.Filename == "" || file.Purpose == "" || file.Metadata == nil {
		return invalidProfilePayload(compatibilityProfile, "invalid compatibility file projection", nil)
	}
	if file.Bytes < 0 || file.CreatedAt < 0 {
		return invalidProfilePayload(compatibilityProfile, "compatibility file counters must be non-negative", nil)
	}
	return nil
}

func compatibilityFileFromFacts(id, fileID, fileRef, resourceRef, resourceURA, filename, purpose, ownerURA, contentType, contentHash string, bytesValue, sizeBytes, createdAt, created int64, status string) (CompatibilityFile, error) {
	file := CompatibilityFile{
		Profile:   compatibilityProfile,
		Kind:      "file",
		ID:        firstNonEmpty(id, fileID),
		Object:    "file",
		Bytes:     firstNonZeroInt64(sizeBytes, bytesValue),
		CreatedAt: firstNonZeroInt64(createdAt, created),
		Filename:  filename,
		Purpose:   purpose,
		Status:    firstNonEmpty(status, "processed"),
		Metadata: map[string]any{
			"profile": compatibilityProfile,
			"source":  "compatibility.file",
		},
	}
	if ownerURA != "" {
		file.Metadata["owner_ura"] = ownerURA
	}
	if contentType != "" {
		file.Metadata["content_type"] = contentType
	}
	if contentHash != "" {
		file.Metadata["content_hash"] = contentHash
	}
	if ref := firstNonEmpty(fileRef, resourceRef, resourceURA); ref != "" {
		file.Metadata["file_ref"] = ref
	}
	if err := validateCompatibilityFileProjection(file); err != nil {
		return CompatibilityFile{}, err
	}
	return file, nil
}

func compatibilityBaseMap(base CompatibilityCarrierBase) map[string]any {
	value := map[string]any{
		"caller_ura":         base.CallerURA,
		"callee_ura":         base.CalleeURA,
		"subject_ura":        base.SubjectURA,
		"descriptor_version": base.DescriptorVersion,
		"nonce_base64":       base.NonceBase64,
		"causal_context":     base.CausalContext,
	}
	if base.AuthToken != "" {
		value["auth_token"] = base.AuthToken
	}
	if base.Metadata != nil {
		value["metadata"] = base.Metadata
	}
	return value
}

func copyStringAnyMap(value map[string]any) map[string]any {
	out := make(map[string]any, len(value)+1)
	for key, raw := range value {
		out[key] = raw
	}
	return out
}

func putNonEmpty(value map[string]any, key string, item string) {
	if item != "" {
		value[key] = item
	}
}

func putNonZeroInt64(value map[string]any, key string, item int64) {
	if item != 0 {
		value[key] = item
	}
}

func mergeCompatibilityMetadata(payload map[string]any, metadata map[string]any) {
	if metadata == nil {
		return
	}
	merged := map[string]any{}
	if base, ok := payload["metadata"].(map[string]any); ok {
		for key, value := range base {
			merged[key] = value
		}
	}
	for key, value := range metadata {
		merged[key] = value
	}
	payload["metadata"] = merged
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func firstNonZeroInt64(values ...int64) int64 {
	for _, value := range values {
		if value != 0 {
			return value
		}
	}
	return 0
}

func wrapCompatibilityTransportError(message string, cause error) error {
	var sdkErr *SDKError
	if errors.As(cause, &sdkErr) {
		return withProfileErrorDetails(sdkErr, compatibilityProfile)
	}
	return transportProfileError(compatibilityProfile, message, cause)
}
