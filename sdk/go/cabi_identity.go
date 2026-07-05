//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_identity_abi_version_fn)(void);
typedef int32_t (*easynet_identity_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_identity_string_free_fn)(char *s);
typedef int32_t (*easynet_identity_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_identity_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_identity_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_identity_project_fn)(uint64_t handle, const char *value, char **out_json);
typedef int32_t (*easynet_identity_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_identity_call_abi_version(void *fn) {
	return ((easynet_identity_abi_version_fn)fn)();
}

static int32_t easynet_identity_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_identity_last_error_json_fn)fn)(out_error_json);
}

static void easynet_identity_call_string_free(void *fn, char *s) {
	((easynet_identity_string_free_fn)fn)(s);
}

static int32_t easynet_identity_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_identity_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_identity_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_identity_shutdown_fn)fn)(handle);
}

static int32_t easynet_identity_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_identity_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_identity_call_project(void *fn, uint64_t handle, const char *value, char **out_json) {
	return ((easynet_identity_project_fn)fn)(handle, value, out_json);
}

static int32_t easynet_identity_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_identity_json_fn)fn)(handle, request_json, out_json);
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

type cabiIdentitySymbols struct {
	abiVersion              unsafe.Pointer
	lastErrorJSON           unsafe.Pointer
	stringFree              unsafe.Pointer
	init                    unsafe.Pointer
	shutdown                unsafe.Pointer
	invocationInvoke        unsafe.Pointer
	projectURA              unsafe.Pointer
	buildURA                unsafe.Pointer
	projectDescriptorRef    unsafe.Pointer
	buildDescriptorRef      unsafe.Pointer
	buildRegisterSigningKey unsafe.Pointer
	buildListSigningKeys    unsafe.Pointer
	buildRevokeSigningKey   unsafe.Pointer
	projectSigningKeyRecord unsafe.Pointer
	projectSigningKeyPage   unsafe.Pointer
	projectRevokeResult     unsafe.Pointer
	projectSignerHandle     unsafe.Pointer
	buildResourceRef        unsafe.Pointer
}

// CABIIdentityTransport is an optional Directory + Identity profile transport
// over libeasynet_cli. It keeps C ABI handles private and delegates identity
// grammar, resource-ref construction, lifecycle carriers, and daemon output
// projection to the Rust/C ABI contract.
type CABIIdentityTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiIdentitySymbols
	handle  uint64
	closed  bool
}

var _ IdentityTransport = (*CABIIdentityTransport)(nil)

// OpenCABIIdentityTransport loads libeasynet_cli and opens an Identity profile transport.
func OpenCABIIdentityTransport(path string, controlPath string) (*CABIIdentityTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIIdentitySymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_identity_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiIdentityInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIIdentityTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIIdentityClient creates an IdentityClient over libeasynet_cli.
func NewCABIIdentityClient(path string, controlPath string) (*IdentityClient, *CABIIdentityTransport, error) {
	transport, err := OpenCABIIdentityTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewIdentityClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIIdentityTransport) ProjectDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	value, err := requiredJSONString(requestJSON, "descriptor_ref", "descriptor-ref projection request")
	if err != nil {
		return nil, err
	}
	return t.callProject(handle, t.symbols.projectDescriptorRef, value, "C ABI identity descriptor projection failed")
}

func (t *CABIIdentityTransport) BuildDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.buildDescriptorRef, requestJSON, "C ABI identity descriptor build failed")
}

func (t *CABIIdentityTransport) ProjectIdentity(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	value, err := requiredJSONString(requestJSON, "ura", "identity projection request")
	if err != nil {
		return nil, err
	}
	return t.callProject(handle, t.symbols.projectURA, value, "C ABI identity projection failed")
}

func (t *CABIIdentityTransport) BuildURA(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.buildURA, requestJSON, "C ABI identity URA build failed")
}

func (t *CABIIdentityTransport) BuildResourceRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.buildResourceRef, requestJSON, "C ABI identity resource-ref build failed")
}

func (t *CABIIdentityTransport) RegisterSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeOutputProjected(ctx, requestJSON, t.symbols.buildRegisterSigningKey, t.symbols.projectSigningKeyRecord, []string{
		"owner_ura",
		"key_id",
		"algorithm",
		"public_key_base64",
		"usage",
		"role",
	}, "C ABI identity register signing key failed")
}

func (t *CABIIdentityTransport) ListSigningKeys(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeOutputProjected(ctx, requestJSON, t.symbols.buildListSigningKeys, t.symbols.projectSigningKeyPage, []string{
		"owner_ura",
		"limit",
		"cursor",
	}, "C ABI identity list signing keys failed")
}

func (t *CABIIdentityTransport) RevokeSigningKey(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeOutputProjected(ctx, requestJSON, t.symbols.buildRevokeSigningKey, t.symbols.projectRevokeResult, []string{
		"owner_ura",
		"key_id",
		"public_key_base64",
		"reason",
	}, "C ABI identity revoke signing key failed")
}

func (t *CABIIdentityTransport) Signer(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeOutputProjected(ctx, requestJSON, t.symbols.buildListSigningKeys, t.symbols.projectSignerHandle, []string{
		"owner_ura",
		"key_id",
		"usage",
	}, "C ABI identity signer failed")
}

func (t *CABIIdentityTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_identity_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiIdentityLastErrorOrCode(symbols, code, "C ABI identity shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIIdentityTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI identity transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI identity transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI identity transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIIdentityTransport) invokeOutputProjected(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, projectSymbol unsafe.Pointer, projectionKeys []string, fallback string) ([]byte, error) {
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
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, "identity")
	if err != nil {
		return nil, err
	}
	projectionJSON, err := projectionRequestJSON(requestJSON, outputJSON, projectionKeys)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, projectSymbol, projectionJSON, fallback)
}

func (t *CABIIdentityTransport) callProject(handle uint64, symbol unsafe.Pointer, value string, fallback string) ([]byte, error) {
	var out *C.char
	cValue := C.CString(value)
	defer C.free(unsafe.Pointer(cValue))
	code := int32(C.easynet_identity_call_project(symbol, C.uint64_t(handle), cValue, &out))
	if code != 0 {
		return nil, cabiIdentityLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiIdentityTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIIdentityTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_identity_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiIdentityLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiIdentityTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIIdentityTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_identity_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiIdentityLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiIdentityTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIIdentitySymbols(library unsafe.Pointer) (cabiIdentitySymbols, error) {
	var symbols cabiIdentitySymbols
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
		{"easynet_identity_project_ura", &symbols.projectURA},
		{"easynet_identity_build_ura", &symbols.buildURA},
		{"easynet_identity_project_descriptor_ref", &symbols.projectDescriptorRef},
		{"easynet_identity_build_descriptor_ref", &symbols.buildDescriptorRef},
		{"easynet_identity_build_register_signing_key_invocation", &symbols.buildRegisterSigningKey},
		{"easynet_identity_build_list_signing_keys_invocation", &symbols.buildListSigningKeys},
		{"easynet_identity_build_revoke_signing_key_invocation", &symbols.buildRevokeSigningKey},
		{"easynet_identity_project_signing_key_record", &symbols.projectSigningKeyRecord},
		{"easynet_identity_project_signing_key_page", &symbols.projectSigningKeyPage},
		{"easynet_identity_project_signing_key_revoke_result", &symbols.projectRevokeResult},
		{"easynet_identity_project_signer_handle", &symbols.projectSignerHandle},
		{"easynet_publication_build_resource_ref", &symbols.buildResourceRef},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiIdentitySymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiIdentityInit(symbols cabiIdentitySymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_identity_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_identity_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiIdentityLastErrorOrCode(symbols, int32(code), "C ABI identity init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI identity init returned an invalid handle")
	}
	return handle, nil
}

func cabiIdentityLastErrorOrCode(symbols cabiIdentitySymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_identity_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiIdentityTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiIdentityTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_identity_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}

func requiredJSONString(raw []byte, key string, label string) (string, error) {
	var request map[string]json.RawMessage
	if err := json.Unmarshal(raw, &request); err != nil {
		return "", invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode %s: %v", label, err), err)
	}
	valueRaw, ok := request[key]
	if !ok || len(valueRaw) == 0 || string(valueRaw) == "null" {
		return "", invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("%s is required", key), nil)
	}
	var value string
	if err := json.Unmarshal(valueRaw, &value); err != nil {
		return "", invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("%s must be a string", key), err)
	}
	if value == "" {
		return "", invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("%s is required", key), nil)
	}
	return value, nil
}

func projectionRequestJSON(requestJSON []byte, resultJSON []byte, passthroughKeys []string) ([]byte, error) {
	var request map[string]json.RawMessage
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode identity projection request: %v", err), err)
	}
	var result map[string]any
	if err := json.Unmarshal(resultJSON, &result); err != nil {
		return nil, invalidProfilePayload(directoryIdentityProfile, fmt.Sprintf("decode identity projection result: %v", err), err)
	}
	selected := make(map[string]json.RawMessage, len(passthroughKeys))
	for _, key := range passthroughKeys {
		if value, ok := request[key]; ok {
			selected[key] = value
		}
	}
	envelope := map[string]any{
		"request": selected,
		"result":  result,
	}
	return json.Marshal(envelope)
}
