//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_receipt_abi_version_fn)(void);
typedef int32_t (*easynet_receipt_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_receipt_string_free_fn)(char *s);
typedef int32_t (*easynet_receipt_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_receipt_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_receipt_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_receipt_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_receipt_call_abi_version(void *fn) {
	return ((easynet_receipt_abi_version_fn)fn)();
}

static int32_t easynet_receipt_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_receipt_last_error_json_fn)fn)(out_error_json);
}

static void easynet_receipt_call_string_free(void *fn, char *s) {
	((easynet_receipt_string_free_fn)fn)(s);
}

static int32_t easynet_receipt_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_receipt_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_receipt_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_receipt_shutdown_fn)fn)(handle);
}

static int32_t easynet_receipt_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_receipt_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_receipt_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_receipt_json_fn)fn)(handle, request_json, out_json);
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

type cabiReceiptSymbols struct {
	abiVersion         unsafe.Pointer
	lastErrorJSON      unsafe.Pointer
	stringFree         unsafe.Pointer
	init               unsafe.Pointer
	shutdown           unsafe.Pointer
	invocationInvoke   unsafe.Pointer
	receiptBuildFetch  unsafe.Pointer
	receiptBuildList   unsafe.Pointer
	receiptBuildGet    unsafe.Pointer
	receiptBuildTrace  unsafe.Pointer
	receiptProject     unsafe.Pointer
	receiptVerify      unsafe.Pointer
	receiptVerifyChain unsafe.Pointer
	receiptCausalRef   unsafe.Pointer
}

// CABIReceiptTransport is an optional Receipt profile transport over
// libeasynet_cli. It keeps C ABI handles private and delegates carrier,
// projection, and conservative verification semantics to the Rust/C ABI
// contract.
type CABIReceiptTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiReceiptSymbols
	handle  uint64
	closed  bool
}

var _ ReceiptTransport = (*CABIReceiptTransport)(nil)

// OpenCABIReceiptTransport loads libeasynet_cli, opens an EasynetHandle via
// easynet_init, and returns a concrete Receipt profile transport.
func OpenCABIReceiptTransport(path string, controlPath string) (*CABIReceiptTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIReceiptSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_receipt_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiReceiptInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIReceiptTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIReceiptClient creates a ReceiptClient over libeasynet_cli.
func NewCABIReceiptClient(path string, controlPath string) (*ReceiptClient, *CABIReceiptTransport, error) {
	transport, err := OpenCABIReceiptTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewReceiptClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIReceiptTransport) Fetch(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := t.callJSON(handle, t.symbols.receiptBuildFetch, requestJSON, "C ABI receipt build fetch failed")
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.invoke(handle, draftJSON)
	if err != nil {
		return nil, err
	}
	outputJSON, err := outputJSONFromInvocationResult(resultJSON)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.receiptProject, outputJSON, "C ABI receipt project failed")
}

func (t *CABIReceiptTransport) buildHistoryInvocation(ctx context.Context, requestJSON []byte, symbol unsafe.Pointer) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, requestJSON, "C ABI receipt history invocation build failed")
}

func (t *CABIReceiptTransport) BuildListHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocation(ctx, requestJSON, t.symbols.receiptBuildList)
}

func (t *CABIReceiptTransport) BuildGetHistoryInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocation(ctx, requestJSON, t.symbols.receiptBuildGet)
}

func (t *CABIReceiptTransport) BuildTraceInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.buildHistoryInvocation(ctx, requestJSON, t.symbols.receiptBuildTrace)
}

func (t *CABIReceiptTransport) ListHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, t.symbols.receiptBuildList)
}

func (t *CABIReceiptTransport) GetHistory(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, t.symbols.receiptBuildGet)
}

func (t *CABIReceiptTransport) GetTrace(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeHistory(ctx, requestJSON, t.symbols.receiptBuildTrace)
}

func (t *CABIReceiptTransport) invokeHistory(ctx context.Context, requestJSON []byte, symbol unsafe.Pointer) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := t.callJSON(handle, symbol, requestJSON, "C ABI receipt history invocation build failed")
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.invoke(handle, draftJSON)
	if err != nil {
		return nil, err
	}
	return outputJSONFromInvocationResult(resultJSON)
}

func (t *CABIReceiptTransport) Project(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.receiptProject, receiptJSON, "C ABI receipt project failed")
}

func (t *CABIReceiptTransport) Verify(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.receiptVerify, receiptJSON, "C ABI receipt verify failed")
}

func (t *CABIReceiptTransport) VerifyChain(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.receiptVerifyChain, requestJSON, "C ABI receipt verify-chain failed")
}

func (t *CABIReceiptTransport) CausalRef(ctx context.Context, receiptJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.receiptCausalRef, receiptJSON, "C ABI receipt causal-ref failed")
}

func (t *CABIReceiptTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_receipt_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiReceiptLastErrorOrCode(symbols, code, "C ABI receipt shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIReceiptTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI receipt transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI receipt transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI receipt transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIReceiptTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_receipt_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiReceiptLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiReceiptTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIReceiptTransport) invoke(handle uint64, draftJSON []byte) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_receipt_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiReceiptLastErrorOrCode(t.symbols, code, "C ABI receipt invocation invoke failed")
	}
	return cabiReceiptTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIReceiptSymbols(library unsafe.Pointer) (cabiReceiptSymbols, error) {
	var symbols cabiReceiptSymbols
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
		{"easynet_receipt_build_fetch_invocation", &symbols.receiptBuildFetch},
		{"easynet_receipt_build_list_history_invocation", &symbols.receiptBuildList},
		{"easynet_receipt_build_get_history_invocation", &symbols.receiptBuildGet},
		{"easynet_receipt_build_trace_invocation", &symbols.receiptBuildTrace},
		{"easynet_receipt_project", &symbols.receiptProject},
		{"easynet_receipt_verify", &symbols.receiptVerify},
		{"easynet_receipt_verify_chain", &symbols.receiptVerifyChain},
		{"easynet_receipt_causal_ref", &symbols.receiptCausalRef},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiReceiptSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiReceiptInit(symbols cabiReceiptSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_receipt_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_receipt_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiReceiptLastErrorOrCode(symbols, int32(code), "C ABI receipt init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI receipt init returned an invalid handle")
	}
	return handle, nil
}

func cabiReceiptLastErrorOrCode(symbols cabiReceiptSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_receipt_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiReceiptTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiReceiptTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_receipt_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}

func outputJSONFromInvocationResult(resultJSON []byte) ([]byte, error) {
	return outputJSONFromProfileInvocationResult(resultJSON, "receipt")
}

func outputJSONFromProfileInvocationResult(resultJSON []byte, profile string) ([]byte, error) {
	var result struct {
		OutputJSON json.RawMessage `json:"output_json"`
	}
	if err := json.Unmarshal(resultJSON, &result); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode %s invocation result JSON: %v", profile, err), err)
	}
	if len(result.OutputJSON) == 0 || string(result.OutputJSON) == "null" {
		return nil, invalidRuntimePayload(fmt.Sprintf("%s invocation result output_json is required", profile), nil)
	}
	var output map[string]any
	if err := json.Unmarshal(result.OutputJSON, &output); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("%s invocation result output_json must be an object", profile), err)
	}
	return json.Marshal(output)
}
