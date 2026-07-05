//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_compatibility_abi_version_fn)(void);
typedef int32_t (*easynet_compatibility_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_compatibility_string_free_fn)(char *s);
typedef int32_t (*easynet_compatibility_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_compatibility_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_compatibility_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_compatibility_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_compatibility_call_abi_version(void *fn) {
	return ((easynet_compatibility_abi_version_fn)fn)();
}

static int32_t easynet_compatibility_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_compatibility_last_error_json_fn)fn)(out_error_json);
}

static void easynet_compatibility_call_string_free(void *fn, char *s) {
	((easynet_compatibility_string_free_fn)fn)(s);
}

static int32_t easynet_compatibility_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_compatibility_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_compatibility_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_compatibility_shutdown_fn)fn)(handle);
}

static int32_t easynet_compatibility_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_compatibility_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_compatibility_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_compatibility_json_fn)fn)(handle, request_json, out_json);
}
*/
import "C"

import (
	"context"
	"fmt"
	"sync"
	"unsafe"
)

type cabiCompatibilitySymbols struct {
	abiVersion        unsafe.Pointer
	lastErrorJSON     unsafe.Pointer
	stringFree        unsafe.Pointer
	init              unsafe.Pointer
	shutdown          unsafe.Pointer
	invocationInvoke  unsafe.Pointer
	buildListModels   unsafe.Pointer
	buildChat         unsafe.Pointer
	buildStreamChat   unsafe.Pointer
	buildFileUpload   unsafe.Pointer
	buildFileRetrieve unsafe.Pointer
	buildFileDelete   unsafe.Pointer
	projectModelPage  unsafe.Pointer
	projectChat       unsafe.Pointer
	projectChatStream unsafe.Pointer
	projectFileUpload unsafe.Pointer
	projectFile       unsafe.Pointer
	projectFileDelete unsafe.Pointer
}

// CABICompatibilityTransport is an optional Compatibility profile transport
// over libeasynet_cli. It delegates OpenAI-shaped carrier construction and
// result projection to the Rust-owned daemon SDK contract.
type CABICompatibilityTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiCompatibilitySymbols
	handle  uint64
	closed  bool
}

var _ CompatibilityTransport = (*CABICompatibilityTransport)(nil)

// OpenCABICompatibilityTransport loads libeasynet_cli and opens a Compatibility profile transport.
func OpenCABICompatibilityTransport(path string, controlPath string) (*CABICompatibilityTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABICompatibilitySymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_compatibility_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiCompatibilityInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABICompatibilityTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABICompatibilityClient creates a CompatibilityClient over libeasynet_cli.
func NewCABICompatibilityClient(path string, controlPath string) (*CompatibilityClient, *CABICompatibilityTransport, error) {
	transport, err := OpenCABICompatibilityTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewCompatibilityClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABICompatibilityTransport) BuildListModelsInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildListModels, requestJSON, "C ABI compatibility list-models invocation build failed")
}

func (t *CABICompatibilityTransport) BuildChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildChat, requestJSON, "C ABI compatibility chat invocation build failed")
}

func (t *CABICompatibilityTransport) BuildStreamChatCompletionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildStreamChat, requestJSON, "C ABI compatibility stream chat invocation build failed")
}

func (t *CABICompatibilityTransport) BuildFileUploadInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildFileUpload, requestJSON, "C ABI compatibility file-upload invocation build failed")
}

func (t *CABICompatibilityTransport) BuildFileRetrieveInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildFileRetrieve, requestJSON, "C ABI compatibility file-retrieve invocation build failed")
}

func (t *CABICompatibilityTransport) BuildFileDeleteInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildFileDelete, requestJSON, "C ABI compatibility file-delete invocation build failed")
}

func (t *CABICompatibilityTransport) ListModels(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildListModels, t.symbols.projectModelPage, "C ABI compatibility list models failed")
}

func (t *CABICompatibilityTransport) CreateChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildChat, t.symbols.projectChat, "C ABI compatibility chat completion failed")
}

func (t *CABICompatibilityTransport) StreamChatCompletion(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildStreamChat, t.symbols.projectChatStream, "C ABI compatibility stream chat completion failed")
}

func (t *CABICompatibilityTransport) UploadFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildFileUpload, t.symbols.projectFileUpload, "C ABI compatibility file upload failed")
}

func (t *CABICompatibilityTransport) RetrieveFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildFileRetrieve, t.symbols.projectFile, "C ABI compatibility file retrieve failed")
}

func (t *CABICompatibilityTransport) DeleteFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildFileDelete, t.symbols.projectFileDelete, "C ABI compatibility file delete failed")
}

func (t *CABICompatibilityTransport) ProjectModelPage(ctx context.Context, modelsJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectModelPage, modelsJSON, "C ABI compatibility model page projection failed")
}

func (t *CABICompatibilityTransport) ProjectChatCompletion(ctx context.Context, completionJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectChat, completionJSON, "C ABI compatibility chat projection failed")
}

func (t *CABICompatibilityTransport) ProjectChatStream(ctx context.Context, streamJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectChatStream, streamJSON, "C ABI compatibility chat stream projection failed")
}

func (t *CABICompatibilityTransport) ProjectFileUpload(ctx context.Context, fileJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectFileUpload, fileJSON, "C ABI compatibility file upload projection failed")
}

func (t *CABICompatibilityTransport) ProjectFile(ctx context.Context, fileJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectFile, fileJSON, "C ABI compatibility file projection failed")
}

func (t *CABICompatibilityTransport) ProjectFileDeleteResult(ctx context.Context, resultJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectFileDelete, resultJSON, "C ABI compatibility file delete projection failed")
}

func (t *CABICompatibilityTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	t.closed = true
	handle := t.handle
	t.handle = 0
	library := t.library
	t.library = nil
	symbols := t.symbols
	t.mu.Unlock()

	var first error
	if handle != 0 {
		code := int32(C.easynet_compatibility_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiCompatibilityLastErrorOrCode(symbols, code, "C ABI compatibility shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABICompatibilityTransport) invokeAndProject(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, projectSymbol unsafe.Pointer, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := t.callJSON(handle, buildSymbol, requestJSON, fallback)
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.invoke(handle, draftJSON, fallback)
	if err != nil {
		return nil, err
	}
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, compatibilityProfile)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, projectSymbol, outputJSON, fallback)
}

func (t *CABICompatibilityTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABICompatibilityTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI compatibility transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI compatibility transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI compatibility transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABICompatibilityTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_compatibility_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiCompatibilityLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiCompatibilityTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABICompatibilityTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_compatibility_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiCompatibilityLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiCompatibilityTakeCString(t.symbols.stringFree, out), nil
}

func bindCABICompatibilitySymbols(library unsafe.Pointer) (cabiCompatibilitySymbols, error) {
	var symbols cabiCompatibilitySymbols
	bindings := []struct {
		name string
		out  *unsafe.Pointer
	}{
		{"easynet_abi_version", &symbols.abiVersion},
		{"easynet_last_error_json", &symbols.lastErrorJSON},
		{"easynet_string_free", &symbols.stringFree},
		{"easynet_init", &symbols.init},
		{"easynet_shutdown", &symbols.shutdown},
		{"easynet_invocation_invoke", &symbols.invocationInvoke},
		{"easynet_compatibility_build_list_models_invocation", &symbols.buildListModels},
		{"easynet_compatibility_build_chat_completion_invocation", &symbols.buildChat},
		{"easynet_compatibility_build_stream_chat_completion_invocation", &symbols.buildStreamChat},
		{"easynet_compatibility_build_file_upload_invocation", &symbols.buildFileUpload},
		{"easynet_compatibility_build_file_retrieve_invocation", &symbols.buildFileRetrieve},
		{"easynet_compatibility_build_file_delete_invocation", &symbols.buildFileDelete},
		{"easynet_compatibility_project_model_page", &symbols.projectModelPage},
		{"easynet_compatibility_project_chat_completion", &symbols.projectChat},
		{"easynet_compatibility_project_chat_stream", &symbols.projectChatStream},
		{"easynet_compatibility_project_file_upload", &symbols.projectFileUpload},
		{"easynet_compatibility_project_file", &symbols.projectFile},
		{"easynet_compatibility_project_file_delete_result", &symbols.projectFileDelete},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiCompatibilitySymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiCompatibilityInit(symbols cabiCompatibilitySymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_compatibility_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_compatibility_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiCompatibilityLastErrorOrCode(symbols, int32(code), "C ABI compatibility init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI compatibility init returned an invalid handle")
	}
	return handle, nil
}

func cabiCompatibilityLastErrorOrCode(symbols cabiCompatibilitySymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_compatibility_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiCompatibilityTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiCompatibilityTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_compatibility_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
