//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_abi_version_fn)(void);
typedef int32_t (*easynet_feature_discovery_fn)(char **out_features_json);
typedef int32_t (*easynet_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_string_free_fn)(char *s);

static uint32_t easynet_call_abi_version(void *fn) {
	return ((easynet_abi_version_fn)fn)();
}

static int32_t easynet_call_feature_discovery(void *fn, char **out_features_json) {
	return ((easynet_feature_discovery_fn)fn)(out_features_json);
}

static int32_t easynet_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_last_error_json_fn)fn)(out_error_json);
}

static void easynet_call_string_free(void *fn, char *s) {
	((easynet_string_free_fn)fn)(s);
}
*/
import "C"

import (
	"context"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

const expectedCABIABIVersion uint32 = 4

// CABIDiscoveryTransport is an optional Go discovery transport backed by
// libeasynet_cli. It owns no daemon or Axon semantics; it only loads the C ABI
// discovery symbols and projects their JSON into the existing Client facade.
type CABIDiscoveryTransport struct {
	mu               sync.Mutex
	library          unsafe.Pointer
	abiVersion       unsafe.Pointer
	featureDiscovery unsafe.Pointer
	lastErrorJSON    unsafe.Pointer
	stringFree       unsafe.Pointer
	closed           bool
}

// OpenCABIDiscoveryTransport loads libeasynet_cli through dlopen and verifies
// the ABI version before returning a DiscoveryTransport.
//
// This constructor is available only with the easynet_cabi,cgo build tags.
// Callers that do not want cgo keep using the transport interfaces directly.
func OpenCABIDiscoveryTransport(path string) (*CABIDiscoveryTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	transport := &CABIDiscoveryTransport{library: library}
	if err := transport.bindSymbols(); err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_call_abi_version(transport.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	return transport, nil
}

// NewCABIClient creates a Runtime Core feature-discovery facade over
// libeasynet_cli. It does not expose the C ABI handle to product code.
func NewCABIClient(path string) (*Client, error) {
	transport, err := OpenCABIDiscoveryTransport(path)
	if err != nil {
		return nil, err
	}
	client, err := NewClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, err
	}
	return client, nil
}

func (t *CABIDiscoveryTransport) FeatureDiscovery(ctx context.Context) ([]byte, error) {
	if ctx == nil {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "context is required",
		}
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return nil, &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "C ABI discovery transport is closed",
		}
	}
	var out *C.char
	code := int32(C.easynet_call_feature_discovery(t.featureDiscovery, &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code)
	}
	if out == nil {
		return []byte{}, nil
	}
	defer C.easynet_call_string_free(t.stringFree, out)
	return []byte(C.GoString(out)), nil
}

func (t *CABIDiscoveryTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return &SDKError{
			Code:      ErrInvalidArgument,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "context is required",
		}
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return nil
	}
	t.closed = true
	if t.library != nil {
		C.dlclose(t.library)
		t.library = nil
	}
	return nil
}

func (t *CABIDiscoveryTransport) bindSymbols() error {
	var err error
	if t.abiVersion, err = requireCABISymbol(t.library, "easynet_abi_version"); err != nil {
		return err
	}
	if t.featureDiscovery, err = requireCABISymbol(t.library, "easynet_feature_discovery"); err != nil {
		return err
	}
	if t.lastErrorJSON, err = requireCABISymbol(t.library, "easynet_last_error_json"); err != nil {
		return err
	}
	if t.stringFree, err = requireCABISymbol(t.library, "easynet_string_free"); err != nil {
		return err
	}
	return nil
}

func (t *CABIDiscoveryTransport) lastErrorOrCode(code int32) error {
	var out *C.char
	errCode := int32(C.easynet_call_last_error_json(t.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		defer C.easynet_call_string_free(t.stringFree, out)
		if decoded, err := DecodeDaemonErrorJSON([]byte(C.GoString(out))); err == nil && decoded != nil {
			return decoded
		}
	}
	return &SDKError{
		Code:      ErrGeneric,
		Stage:     "cabi",
		Retry:     RetryUnknown,
		Retryable: false,
		Message:   fmt.Sprintf("C ABI discovery call failed with code %d", code),
	}
}

func openCABIDynamicLibrary(path string) (unsafe.Pointer, string, error) {
	candidates := cabiLibraryCandidates(path)
	var failures []string
	for _, candidate := range candidates {
		cPath := C.CString(candidate)
		C.dlerror()
		handle := C.dlopen(cPath, C.RTLD_NOW)
		errText := C.dlerror()
		C.free(unsafe.Pointer(cPath))
		if handle != nil {
			return handle, candidate, nil
		}
		if errText != nil {
			failures = append(failures, fmt.Sprintf("%s: %s", candidate, C.GoString(errText)))
		} else {
			failures = append(failures, candidate)
		}
	}
	return nil, "", &SDKError{
		Code:      ErrRouteUnavailable,
		Stage:     "cabi",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "load libeasynet_cli failed: " + fmt.Sprint(failures),
	}
}

func requireCABISymbol(library unsafe.Pointer, symbol string) (unsafe.Pointer, error) {
	cSymbol := C.CString(symbol)
	C.dlerror()
	ptr := C.dlsym(library, cSymbol)
	errText := C.dlerror()
	C.free(unsafe.Pointer(cSymbol))
	if ptr == nil {
		if errText != nil {
			return nil, fmt.Errorf("%s: %s", symbol, C.GoString(errText))
		}
		return nil, fmt.Errorf("%s: symbol not found", symbol)
	}
	return ptr, nil
}

func cabiLibraryCandidates(path string) []string {
	if path != "" {
		return []string{path}
	}
	switch runtime.GOOS {
	case "darwin":
		return []string{"libeasynet_cli.dylib", "target/debug/libeasynet_cli.dylib", "target/release/libeasynet_cli.dylib"}
	default:
		return []string{"libeasynet_cli.so", "target/debug/libeasynet_cli.so", "target/release/libeasynet_cli.so"}
	}
}
