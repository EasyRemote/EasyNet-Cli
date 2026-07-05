//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_host_binding_abi_version_fn)(void);
typedef int32_t (*easynet_host_binding_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_host_binding_string_free_fn)(char *s);
typedef int32_t (*easynet_host_binding_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_host_binding_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_host_binding_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_host_binding_call_abi_version(void *fn) {
	return ((easynet_host_binding_abi_version_fn)fn)();
}

static int32_t easynet_host_binding_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_host_binding_last_error_json_fn)fn)(out_error_json);
}

static void easynet_host_binding_call_string_free(void *fn, char *s) {
	((easynet_host_binding_string_free_fn)fn)(s);
}

static int32_t easynet_host_binding_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_host_binding_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_host_binding_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_host_binding_shutdown_fn)fn)(handle);
}

static int32_t easynet_host_binding_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_host_binding_json_fn)fn)(handle, request_json, out_json);
}
*/
import "C"

import (
	"context"
	"fmt"
	"sync"
	"unsafe"
)

type cabiHostBindingSymbols struct {
	abiVersion             unsafe.Pointer
	lastErrorJSON          unsafe.Pointer
	stringFree             unsafe.Pointer
	init                   unsafe.Pointer
	shutdown               unsafe.Pointer
	buildHostStreamBinding unsafe.Pointer
	decodeRequest          unsafe.Pointer
	encodeItem             unsafe.Pointer
	encodeError            unsafe.Pointer
	encodeTerminal         unsafe.Pointer
	foldOutputHash         unsafe.Pointer
}

// CABIHostBindingTransport is an optional Host Binding profile transport over
// libeasynet_cli. It keeps C ABI handles private while delegating host-stream
// codec and output-hash semantics to the Rust-owned daemon SDK contract.
type CABIHostBindingTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiHostBindingSymbols
	handle  uint64
	closed  bool
}

var _ HostBindingTransport = (*CABIHostBindingTransport)(nil)

// OpenCABIHostBindingTransport loads libeasynet_cli and opens a Host Binding profile transport.
func OpenCABIHostBindingTransport(path string, controlPath string) (*CABIHostBindingTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIHostBindingSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_host_binding_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiHostBindingInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIHostBindingTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIHostBindingClient creates a HostBindingClient over libeasynet_cli.
func NewCABIHostBindingClient(path string, controlPath string) (*HostBindingClient, *CABIHostBindingTransport, error) {
	transport, err := OpenCABIHostBindingTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewHostBindingClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIHostBindingTransport) BuildHostStreamBinding(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildHostStreamBinding, requestJSON, "C ABI host binding build failed")
}

func (t *CABIHostBindingTransport) DecodeRequest(ctx context.Context, envelopeJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.decodeRequest, envelopeJSON, "C ABI host binding decode request failed")
}

func (t *CABIHostBindingTransport) EncodeItem(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.encodeItem, requestJSON, "C ABI host binding encode item failed")
}

func (t *CABIHostBindingTransport) EncodeError(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.encodeError, requestJSON, "C ABI host binding encode error failed")
}

func (t *CABIHostBindingTransport) EncodeTerminal(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.encodeTerminal, requestJSON, "C ABI host binding encode terminal failed")
}

func (t *CABIHostBindingTransport) FoldOutputHash(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.foldOutputHash, requestJSON, "C ABI host binding hash fold failed")
}

func (t *CABIHostBindingTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_host_binding_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiHostBindingLastErrorOrCode(symbols, code, "C ABI host binding shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIHostBindingTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABIHostBindingTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI host binding transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI host binding transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI host binding transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIHostBindingTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_host_binding_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiHostBindingLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiHostBindingTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIHostBindingSymbols(library unsafe.Pointer) (cabiHostBindingSymbols, error) {
	var symbols cabiHostBindingSymbols
	bindings := []struct {
		name string
		out  *unsafe.Pointer
	}{
		{"easynet_abi_version", &symbols.abiVersion},
		{"easynet_last_error_json", &symbols.lastErrorJSON},
		{"easynet_string_free", &symbols.stringFree},
		{"easynet_init", &symbols.init},
		{"easynet_shutdown", &symbols.shutdown},
		{"easynet_host_binding_build", &symbols.buildHostStreamBinding},
		{"easynet_host_binding_decode_request", &symbols.decodeRequest},
		{"easynet_host_binding_encode_item", &symbols.encodeItem},
		{"easynet_host_binding_encode_error", &symbols.encodeError},
		{"easynet_host_binding_encode_terminal", &symbols.encodeTerminal},
		{"easynet_host_binding_fold_output_hash", &symbols.foldOutputHash},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiHostBindingSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiHostBindingInit(symbols cabiHostBindingSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_host_binding_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_host_binding_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiHostBindingLastErrorOrCode(symbols, int32(code), "C ABI host binding init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI host binding init returned an invalid handle")
	}
	return handle, nil
}

func cabiHostBindingLastErrorOrCode(symbols cabiHostBindingSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_host_binding_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiHostBindingTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func cabiHostBindingTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_host_binding_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
