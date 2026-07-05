//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_publication_abi_version_fn)(void);
typedef int32_t (*easynet_publication_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_publication_string_free_fn)(char *s);
typedef int32_t (*easynet_publication_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_publication_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_publication_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_publication_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_publication_call_abi_version(void *fn) {
	return ((easynet_publication_abi_version_fn)fn)();
}

static int32_t easynet_publication_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_publication_last_error_json_fn)fn)(out_error_json);
}

static void easynet_publication_call_string_free(void *fn, char *s) {
	((easynet_publication_string_free_fn)fn)(s);
}

static int32_t easynet_publication_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_publication_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_publication_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_publication_shutdown_fn)fn)(handle);
}

static int32_t easynet_publication_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_publication_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_publication_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_publication_json_fn)fn)(handle, request_json, out_json);
}
*/
import "C"

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"
	"unsafe"
)

type cabiPublicationSymbols struct {
	abiVersion                   unsafe.Pointer
	lastErrorJSON                unsafe.Pointer
	stringFree                   unsafe.Pointer
	init                         unsafe.Pointer
	shutdown                     unsafe.Pointer
	invocationInvoke             unsafe.Pointer
	buildResourceRef             unsafe.Pointer
	validatePackage              unsafe.Pointer
	installPlugin                unsafe.Pointer
	buildDeployInvocation        unsafe.Pointer
	projectDeployResult          unsafe.Pointer
	buildListAbilitiesInvocation unsafe.Pointer
	projectAbilityPage           unsafe.Pointer
	buildShowAbilityInvocation   unsafe.Pointer
	projectAbilityRecord         unsafe.Pointer
	buildEnableImplInvocation    unsafe.Pointer
	projectEnableImplResult      unsafe.Pointer
	buildDisableImplInvocation   unsafe.Pointer
	projectDisableImplResult     unsafe.Pointer
	buildUnpublishInvocation     unsafe.Pointer
	projectUnpublishResult       unsafe.Pointer
}

// CABIPublicationTransport is an optional Publication profile transport over
// libeasynet_cli. It keeps C ABI handles private while delegating ResourceRef,
// package validation, publication Invocation carriers, daemon execution, and
// result projection to the Rust-owned daemon SDK contract.
type CABIPublicationTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiPublicationSymbols
	handle  uint64
	closed  bool
}

var _ PublicationTransport = (*CABIPublicationTransport)(nil)

// OpenCABIPublicationTransport loads libeasynet_cli and opens a Publication profile transport.
func OpenCABIPublicationTransport(path string, controlPath string) (*CABIPublicationTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIPublicationSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_publication_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiPublicationInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIPublicationTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIPublicationClient creates a PublicationClient over libeasynet_cli.
func NewCABIPublicationClient(path string, controlPath string) (*PublicationClient, *CABIPublicationTransport, error) {
	transport, err := OpenCABIPublicationTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewPublicationClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIPublicationTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildResourceRef, requestJSON, "C ABI publication resource-ref build failed")
}

func (t *CABIPublicationTransport) ValidatePackage(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.validatePackage, requestJSON, "C ABI publication package validation failed")
}

func (t *CABIPublicationTransport) DeployAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildDeployInvocation, t.symbols.projectDeployResult, "C ABI publication deploy failed")
}

func (t *CABIPublicationTransport) BuildDeployInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildDeployInvocation, requestJSON, "C ABI publication deploy invocation build failed")
}

func (t *CABIPublicationTransport) InstallPlugin(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.installPlugin, requestJSON, "C ABI publication plugin install failed")
}

func (t *CABIPublicationTransport) ListAbilities(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildListAbilitiesInvocation, t.symbols.projectAbilityPage, "C ABI publication list abilities failed")
}

func (t *CABIPublicationTransport) ShowAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildShowAbilityInvocation, t.symbols.projectAbilityRecord, "C ABI publication show ability failed")
}

func (t *CABIPublicationTransport) EnableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildEnableImplInvocation, t.symbols.projectEnableImplResult, "C ABI publication enable ability impl failed")
}

func (t *CABIPublicationTransport) DisableAbilityImpl(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildDisableImplInvocation, t.symbols.projectDisableImplResult, "C ABI publication disable ability impl failed")
}

func (t *CABIPublicationTransport) BuildUnpublishInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildUnpublishInvocation, requestJSON, "C ABI publication unpublish invocation build failed")
}

func (t *CABIPublicationTransport) UnpublishAbility(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildUnpublishInvocation, t.symbols.projectUnpublishResult, "C ABI publication unpublish ability failed")
}

func (t *CABIPublicationTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_publication_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiPublicationLastErrorOrCode(symbols, code, "C ABI publication shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIPublicationTransport) invokeAndProject(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, projectSymbol unsafe.Pointer, fallback string) ([]byte, error) {
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
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, publicationProfile)
	if err != nil {
		return nil, err
	}
	projectionJSON, err := publicationProjectionEnvelope(requestJSON, outputJSON)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, projectSymbol, projectionJSON, fallback)
}

func (t *CABIPublicationTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABIPublicationTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI publication transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI publication transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI publication transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIPublicationTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_publication_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiPublicationLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiPublicationTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIPublicationTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_publication_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiPublicationLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiPublicationTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIPublicationSymbols(library unsafe.Pointer) (cabiPublicationSymbols, error) {
	var symbols cabiPublicationSymbols
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
		{"easynet_publication_build_resource_ref", &symbols.buildResourceRef},
		{"easynet_publication_validate_package", &symbols.validatePackage},
		{"easynet_publication_install_plugin", &symbols.installPlugin},
		{"easynet_publication_build_deploy_invocation", &symbols.buildDeployInvocation},
		{"easynet_publication_project_deploy_result", &symbols.projectDeployResult},
		{"easynet_publication_build_list_abilities_invocation", &symbols.buildListAbilitiesInvocation},
		{"easynet_publication_project_ability_page", &symbols.projectAbilityPage},
		{"easynet_publication_build_show_ability_invocation", &symbols.buildShowAbilityInvocation},
		{"easynet_publication_project_ability_record", &symbols.projectAbilityRecord},
		{"easynet_publication_build_enable_ability_impl_invocation", &symbols.buildEnableImplInvocation},
		{"easynet_publication_project_enable_ability_impl_result", &symbols.projectEnableImplResult},
		{"easynet_publication_build_disable_ability_impl_invocation", &symbols.buildDisableImplInvocation},
		{"easynet_publication_project_disable_ability_impl_result", &symbols.projectDisableImplResult},
		{"easynet_publication_build_unpublish_invocation", &symbols.buildUnpublishInvocation},
		{"easynet_publication_project_unpublish_result", &symbols.projectUnpublishResult},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiPublicationSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiPublicationInit(symbols cabiPublicationSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_publication_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_publication_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiPublicationLastErrorOrCode(symbols, int32(code), "C ABI publication init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI publication init returned an invalid handle")
	}
	return handle, nil
}

func cabiPublicationLastErrorOrCode(symbols cabiPublicationSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_publication_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiPublicationTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiPublicationTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_publication_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}

func publicationProjectionEnvelope(requestJSON []byte, resultJSON []byte) ([]byte, error) {
	var request map[string]any
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication projection request: %v", err), err)
	}
	var result any
	if err := json.Unmarshal(resultJSON, &result); err != nil {
		return nil, invalidProfilePayload(publicationProfile, fmt.Sprintf("decode publication projection result: %v", err), err)
	}
	request["result"] = result
	return json.Marshal(request)
}
