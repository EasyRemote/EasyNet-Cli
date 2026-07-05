//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_surface_abi_version_fn)(void);
typedef int32_t (*easynet_surface_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_surface_string_free_fn)(char *s);
typedef int32_t (*easynet_surface_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_surface_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_surface_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_surface_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_surface_call_abi_version(void *fn) {
	return ((easynet_surface_abi_version_fn)fn)();
}

static int32_t easynet_surface_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_surface_last_error_json_fn)fn)(out_error_json);
}

static void easynet_surface_call_string_free(void *fn, char *s) {
	((easynet_surface_string_free_fn)fn)(s);
}

static int32_t easynet_surface_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_surface_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_surface_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_surface_shutdown_fn)fn)(handle);
}

static int32_t easynet_surface_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_surface_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_surface_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_surface_json_fn)fn)(handle, request_json, out_json);
}
*/
import "C"

import (
	"context"
	"fmt"
	"sync"
	"unsafe"
)

type cabiSurfaceSymbols struct {
	abiVersion            unsafe.Pointer
	lastErrorJSON         unsafe.Pointer
	stringFree            unsafe.Pointer
	init                  unsafe.Pointer
	shutdown              unsafe.Pointer
	invocationInvoke      unsafe.Pointer
	buildListPages        unsafe.Pointer
	buildCreatePage       unsafe.Pointer
	buildDeletePage       unsafe.Pointer
	buildManifest         unsafe.Pointer
	buildHealth           unsafe.Pointer
	projectPageRecord     unsafe.Pointer
	projectPagePage       unsafe.Pointer
	projectManifest       unsafe.Pointer
	projectPublicPageRef  unsafe.Pointer
	projectMutationResult unsafe.Pointer
	projectSurfaceHealth  unsafe.Pointer
}

// CABISurfaceTransport is an optional Surface profile transport over
// libeasynet_cli. It delegates page carrier construction and page/readiness
// projections to the Rust-owned daemon SDK contract.
type CABISurfaceTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiSurfaceSymbols
	handle  uint64
	closed  bool
}

var _ SurfaceTransport = (*CABISurfaceTransport)(nil)

// OpenCABISurfaceTransport loads libeasynet_cli and opens a Surface profile transport.
func OpenCABISurfaceTransport(path string, controlPath string) (*CABISurfaceTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABISurfaceSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_surface_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiSurfaceInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABISurfaceTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABISurfaceClient creates a SurfaceClient over libeasynet_cli.
func NewCABISurfaceClient(path string, controlPath string) (*SurfaceClient, *CABISurfaceTransport, error) {
	transport, err := OpenCABISurfaceTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewSurfaceClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABISurfaceTransport) BuildListPagesInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildListPages, requestJSON, "C ABI surface list-pages invocation build failed")
}

func (t *CABISurfaceTransport) BuildCreatePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildCreatePage, requestJSON, "C ABI surface create-page invocation build failed")
}

func (t *CABISurfaceTransport) BuildDeletePageInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildDeletePage, requestJSON, "C ABI surface delete-page invocation build failed")
}

func (t *CABISurfaceTransport) BuildManifestInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildManifest, requestJSON, "C ABI surface manifest invocation build failed")
}

func (t *CABISurfaceTransport) BuildHealthInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildHealth, requestJSON, "C ABI surface health invocation build failed")
}

func (t *CABISurfaceTransport) ListPages(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildListPages, t.symbols.projectPagePage, "C ABI surface list pages failed")
}

func (t *CABISurfaceTransport) CreatePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildCreatePage, t.symbols.projectPageRecord, "C ABI surface create page failed")
}

func (t *CABISurfaceTransport) DeletePage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildDeletePage, t.symbols.projectMutationResult, "C ABI surface delete page failed")
}

func (t *CABISurfaceTransport) SurfaceManifest(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildManifest, t.symbols.projectManifest, "C ABI surface manifest failed")
}

func (t *CABISurfaceTransport) PublicPageRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectPublicPageRef, requestJSON, "C ABI surface public page ref projection failed")
}

func (t *CABISurfaceTransport) SurfaceHealth(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildHealth, t.symbols.projectSurfaceHealth, "C ABI surface health failed")
}

func (t *CABISurfaceTransport) ProjectPageRecord(ctx context.Context, pageJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectPageRecord, pageJSON, "C ABI surface page record projection failed")
}

func (t *CABISurfaceTransport) ProjectPagePage(ctx context.Context, pagesJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectPagePage, pagesJSON, "C ABI surface page page projection failed")
}

func (t *CABISurfaceTransport) ProjectManifest(ctx context.Context, pageJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectManifest, pageJSON, "C ABI surface manifest projection failed")
}

func (t *CABISurfaceTransport) ProjectMutationResult(ctx context.Context, resultJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectMutationResult, resultJSON, "C ABI surface mutation projection failed")
}

func (t *CABISurfaceTransport) ProjectHealth(ctx context.Context, healthJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectSurfaceHealth, healthJSON, "C ABI surface health projection failed")
}

func (t *CABISurfaceTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_surface_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiSurfaceLastErrorOrCode(symbols, code, "C ABI surface shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABISurfaceTransport) invokeAndProject(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, projectSymbol unsafe.Pointer, fallback string) ([]byte, error) {
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
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, surfaceProfile)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, projectSymbol, outputJSON, fallback)
}

func (t *CABISurfaceTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABISurfaceTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI surface transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI surface transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI surface transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABISurfaceTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_surface_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiSurfaceLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiSurfaceTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABISurfaceTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_surface_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiSurfaceLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiSurfaceTakeCString(t.symbols.stringFree, out), nil
}

func bindCABISurfaceSymbols(library unsafe.Pointer) (cabiSurfaceSymbols, error) {
	var symbols cabiSurfaceSymbols
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
		{"easynet_surface_build_list_pages_invocation", &symbols.buildListPages},
		{"easynet_surface_build_create_page_invocation", &symbols.buildCreatePage},
		{"easynet_surface_build_delete_page_invocation", &symbols.buildDeletePage},
		{"easynet_surface_build_manifest_invocation", &symbols.buildManifest},
		{"easynet_surface_build_health_invocation", &symbols.buildHealth},
		{"easynet_surface_project_page_record", &symbols.projectPageRecord},
		{"easynet_surface_project_page_page", &symbols.projectPagePage},
		{"easynet_surface_project_manifest", &symbols.projectManifest},
		{"easynet_surface_project_public_page_ref", &symbols.projectPublicPageRef},
		{"easynet_surface_project_mutation_result", &symbols.projectMutationResult},
		{"easynet_surface_project_health", &symbols.projectSurfaceHealth},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiSurfaceSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiSurfaceInit(symbols cabiSurfaceSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_surface_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_surface_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiSurfaceLastErrorOrCode(symbols, int32(code), "C ABI surface init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI surface init returned an invalid handle")
	}
	return handle, nil
}

func cabiSurfaceLastErrorOrCode(symbols cabiSurfaceSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_surface_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiSurfaceTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiSurfaceTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_surface_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
