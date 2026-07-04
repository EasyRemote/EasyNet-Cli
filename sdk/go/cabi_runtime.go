//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_runtime_abi_version_fn)(void);
typedef int32_t (*easynet_runtime_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_runtime_string_free_fn)(char *s);
typedef int32_t (*easynet_daemon_start_fn)(const char *config_json, uint64_t *out_daemon_handle);
typedef int32_t (*easynet_daemon_attach_fn)(const char *options_json, uint64_t *out_daemon_handle);
typedef int32_t (*easynet_daemon_discover_fn)(const char *options_json, char **out_discovery_json);
typedef int32_t (*easynet_daemon_stop_fn)(uint64_t handle);
typedef int32_t (*easynet_daemon_detach_fn)(uint64_t handle);
typedef int32_t (*easynet_daemon_status_fn)(uint64_t handle, char **out_status_json);
typedef int32_t (*easynet_daemon_open_client_fn)(uint64_t daemon_handle, uint64_t *out_handle);
typedef int32_t (*easynet_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_runtime_health_fn)(uint64_t handle, char **out_health_json);
typedef int32_t (*easynet_invocation_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_invocation_prepare_fn)(uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json);
typedef int32_t (*easynet_invocation_sign_prepared_fn)(uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json);
typedef int32_t (*easynet_invocation_submit_signed_handle_fn)(uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json);
typedef int32_t (*easynet_invocation_handle_await_fn)(uint64_t handle, uint64_t invocation_handle_id, char **out_result_json);
typedef int32_t (*easynet_invocation_handle_cancel_fn)(uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json);
typedef int32_t (*easynet_invocation_handle_events_fn)(uint64_t handle, uint64_t invocation_handle_id, char **out_events_json);
typedef int32_t (*easynet_invocation_handle_free_fn)(uint64_t handle, uint64_t invocation_handle_id);
typedef int32_t (*easynet_prepared_invocation_free_fn)(uint64_t prepared_id);
typedef int32_t (*easynet_signed_invocation_free_fn)(uint64_t signed_id);

static uint32_t easynet_runtime_call_abi_version(void *fn) {
	return ((easynet_runtime_abi_version_fn)fn)();
}

static int32_t easynet_runtime_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_runtime_last_error_json_fn)fn)(out_error_json);
}

static void easynet_runtime_call_string_free(void *fn, char *s) {
	((easynet_runtime_string_free_fn)fn)(s);
}

static int32_t easynet_runtime_call_daemon_start(void *fn, const char *config_json, uint64_t *out_daemon_handle) {
	return ((easynet_daemon_start_fn)fn)(config_json, out_daemon_handle);
}

static int32_t easynet_runtime_call_daemon_attach(void *fn, const char *options_json, uint64_t *out_daemon_handle) {
	return ((easynet_daemon_attach_fn)fn)(options_json, out_daemon_handle);
}

static int32_t easynet_runtime_call_daemon_discover(void *fn, const char *options_json, char **out_discovery_json) {
	return ((easynet_daemon_discover_fn)fn)(options_json, out_discovery_json);
}

static int32_t easynet_runtime_call_daemon_stop(void *fn, uint64_t handle) {
	return ((easynet_daemon_stop_fn)fn)(handle);
}

static int32_t easynet_runtime_call_daemon_detach(void *fn, uint64_t handle) {
	return ((easynet_daemon_detach_fn)fn)(handle);
}

static int32_t easynet_runtime_call_daemon_status(void *fn, uint64_t handle, char **out_status_json) {
	return ((easynet_daemon_status_fn)fn)(handle, out_status_json);
}

static int32_t easynet_runtime_call_daemon_open_client(void *fn, uint64_t daemon_handle, uint64_t *out_handle) {
	return ((easynet_daemon_open_client_fn)fn)(daemon_handle, out_handle);
}

static int32_t easynet_runtime_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_shutdown_fn)fn)(handle);
}

static int32_t easynet_runtime_call_health(void *fn, uint64_t handle, char **out_health_json) {
	return ((easynet_runtime_health_fn)fn)(handle, out_health_json);
}

static int32_t easynet_runtime_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_invocation_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_runtime_call_prepare(void *fn, uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json) {
	return ((easynet_invocation_prepare_fn)fn)(handle, invocation_json, options_json, out_prepared_id, out_prepared_json);
}

static int32_t easynet_runtime_call_sign_prepared(void *fn, uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json) {
	return ((easynet_invocation_sign_prepared_fn)fn)(prepared_id, signature_json, out_signed_id, out_signed_json);
}

static int32_t easynet_runtime_call_submit_signed_handle(void *fn, uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json) {
	return ((easynet_invocation_submit_signed_handle_fn)fn)(handle, signed_id, out_invocation_handle_id, out_submitted_json);
}

static int32_t easynet_runtime_call_handle_await(void *fn, uint64_t handle, uint64_t invocation_handle_id, char **out_result_json) {
	return ((easynet_invocation_handle_await_fn)fn)(handle, invocation_handle_id, out_result_json);
}

static int32_t easynet_runtime_call_handle_cancel(void *fn, uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json) {
	return ((easynet_invocation_handle_cancel_fn)fn)(handle, invocation_handle_id, reason_json, out_cancel_json);
}

static int32_t easynet_runtime_call_handle_events(void *fn, uint64_t handle, uint64_t invocation_handle_id, char **out_events_json) {
	return ((easynet_invocation_handle_events_fn)fn)(handle, invocation_handle_id, out_events_json);
}

static int32_t easynet_runtime_call_handle_free(void *fn, uint64_t handle, uint64_t invocation_handle_id) {
	return ((easynet_invocation_handle_free_fn)fn)(handle, invocation_handle_id);
}

static int32_t easynet_runtime_call_prepared_free(void *fn, uint64_t prepared_id) {
	return ((easynet_prepared_invocation_free_fn)fn)(prepared_id);
}

static int32_t easynet_runtime_call_signed_free(void *fn, uint64_t signed_id) {
	return ((easynet_signed_invocation_free_fn)fn)(signed_id);
}
*/
import "C"

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"sync"
	"unsafe"
)

type cabiRuntimeSymbols struct {
	abiVersion         unsafe.Pointer
	lastErrorJSON      unsafe.Pointer
	stringFree         unsafe.Pointer
	daemonStart        unsafe.Pointer
	daemonAttach       unsafe.Pointer
	daemonDiscover     unsafe.Pointer
	daemonStop         unsafe.Pointer
	daemonDetach       unsafe.Pointer
	daemonStatus       unsafe.Pointer
	daemonOpenClient   unsafe.Pointer
	shutdown           unsafe.Pointer
	runtimeHealth      unsafe.Pointer
	invocationInvoke   unsafe.Pointer
	invocationPrepare  unsafe.Pointer
	signPrepared       unsafe.Pointer
	submitSignedHandle unsafe.Pointer
	handleAwait        unsafe.Pointer
	handleCancel       unsafe.Pointer
	handleEvents       unsafe.Pointer
	handleFree         unsafe.Pointer
	preparedFree       unsafe.Pointer
	signedFree         unsafe.Pointer
}

// CABIDaemonTransport is an optional daemon lifecycle transport over
// libeasynet_cli. It keeps C ABI handles private and exposes only SDK facade
// DTOs to product code.
type CABIDaemonTransport struct {
	mu       sync.Mutex
	library  unsafe.Pointer
	symbols  cabiRuntimeSymbols
	handles  map[string]uint64
	runtimes map[*CABIRuntimeTransport]struct{}
	closed   bool
}

// OpenCABIDaemonTransport loads libeasynet_cli and exposes daemon lifecycle
// operations through the existing Go SDK facade interfaces.
func OpenCABIDaemonTransport(path string) (*CABIDaemonTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIRuntimeSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_runtime_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionIncompatible,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	return &CABIDaemonTransport{
		library:  library,
		symbols:  symbols,
		handles:  map[string]uint64{},
		runtimes: map[*CABIRuntimeTransport]struct{}{},
	}, nil
}

// NewCABIDaemonControl creates a daemon control facade over libeasynet_cli.
// The returned transport owns the dynamic library and C ABI handles; callers
// must close it when the facade is no longer needed.
func NewCABIDaemonControl(path string) (*DaemonControl, *CABIDaemonTransport, error) {
	transport, err := OpenCABIDaemonTransport(path)
	if err != nil {
		return nil, nil, err
	}
	control, err := NewDaemonControl(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return control, transport, nil
}

func (t *CABIDaemonTransport) Discover(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	raw, err := t.callDaemonDiscover(optionsJSON)
	if err != nil {
		return nil, err
	}
	status, err := daemonStatusFromCABI("0", raw)
	if err != nil {
		return nil, err
	}
	return json.Marshal(status["endpoints"])
}

func (t *CABIDaemonTransport) Start(ctx context.Context, configJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	projected, err := daemonStartConfigForCABI(configJSON)
	if err != nil {
		return nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(projected, func(cConfig *C.char) C.int32_t {
		return C.easynet_runtime_call_daemon_start(t.symbols.daemonStart, cConfig, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon start failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return nil, invalidCABIHandle("C ABI daemon start returned an invalid handle")
	}
	handleID := strconv.FormatUint(handle, 10)
	t.mu.Lock()
	t.handles[handleID] = handle
	t.mu.Unlock()
	raw, err := t.statusForHandle(handleID, handle)
	if err != nil {
		_ = t.detachCHandle(handle)
		t.mu.Lock()
		delete(t.handles, handleID)
		t.mu.Unlock()
		return nil, err
	}
	return raw, nil
}

func (t *CABIDaemonTransport) Attach(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(optionsJSON, func(cOptions *C.char) C.int32_t {
		return C.easynet_runtime_call_daemon_attach(t.symbols.daemonAttach, cOptions, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon attach failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return nil, invalidCABIHandle("C ABI daemon attach returned an invalid handle")
	}
	handleID := strconv.FormatUint(handle, 10)
	t.mu.Lock()
	t.handles[handleID] = handle
	t.mu.Unlock()
	raw, err := t.statusForHandle(handleID, handle)
	if err != nil {
		_ = t.detachCHandle(handle)
		t.mu.Lock()
		delete(t.handles, handleID)
		t.mu.Unlock()
		return nil, err
	}
	return raw, nil
}

func (t *CABIDaemonTransport) Status(ctx context.Context, handleID string) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	handle, err := t.requireDaemonHandle(handleID)
	if err != nil {
		return nil, err
	}
	return t.statusForHandle(handleID, handle)
}

func (t *CABIDaemonTransport) OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	_ = optionsJSON
	daemonHandle, err := t.requireDaemonHandle(handleID)
	if err != nil {
		return nil, nil, err
	}
	var out C.uint64_t
	code := int32(C.easynet_runtime_call_daemon_open_client(t.symbols.daemonOpenClient, C.uint64_t(daemonHandle), &out))
	if code != 0 {
		return nil, nil, t.lastErrorOrCode(code, "C ABI daemon open client failed")
	}
	runtimeHandle := uint64(out)
	if runtimeHandle == 0 {
		return nil, nil, invalidCABIHandle("C ABI daemon open client returned an invalid runtime handle")
	}
	runtime := newCABIRuntimeTransport(t.symbols, runtimeHandle, true)
	t.mu.Lock()
	t.runtimes[runtime] = struct{}{}
	t.mu.Unlock()
	return runtime, []byte(fmt.Sprintf(`{"ready":true,"abi_version":%d,"transport":"c_abi"}`, expectedCABIABIVersion)), nil
}

func (t *CABIDaemonTransport) Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	_ = optionsJSON
	handle, err := t.requireDaemonHandle(handleID)
	if err != nil {
		return nil, err
	}
	code := int32(C.easynet_runtime_call_daemon_stop(t.symbols.daemonStop, C.uint64_t(handle)))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon stop failed")
	}
	t.mu.Lock()
	delete(t.handles, handleID)
	t.mu.Unlock()
	return []byte(fmt.Sprintf(`{"handle_id":%q,"state":"Stopped","diagnostics":[]}`, handleID)), nil
}

func (t *CABIDaemonTransport) Detach(ctx context.Context, handleID string) error {
	if err := t.requireOpen(ctx); err != nil {
		return err
	}
	handle, err := t.requireDaemonHandle(handleID)
	if err != nil {
		return err
	}
	if err := t.detachCHandle(handle); err != nil {
		return err
	}
	t.mu.Lock()
	delete(t.handles, handleID)
	t.mu.Unlock()
	return nil
}

func (t *CABIDaemonTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	t.closed = true
	runtimes := make([]*CABIRuntimeTransport, 0, len(t.runtimes))
	for runtime := range t.runtimes {
		runtimes = append(runtimes, runtime)
	}
	handles := make([]uint64, 0, len(t.handles))
	for _, handle := range t.handles {
		handles = append(handles, handle)
	}
	t.runtimes = map[*CABIRuntimeTransport]struct{}{}
	t.handles = map[string]uint64{}
	library := t.library
	t.library = nil
	t.mu.Unlock()

	var first error
	for _, runtime := range runtimes {
		if err := runtime.Close(ctx); err != nil && first == nil {
			first = err
		}
	}
	for _, handle := range handles {
		if err := t.detachCHandle(handle); err != nil && first == nil {
			first = err
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIDaemonTransport) requireOpen(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return invalidRuntimeClient("C ABI daemon transport is closed")
	}
	return nil
}

func (t *CABIDaemonTransport) requireDaemonHandle(handleID string) (uint64, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	handle := t.handles[handleID]
	if handle == 0 {
		return 0, &SDKError{
			Code:      ErrInvalidHandle,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "daemon handle is not owned by this transport",
		}
	}
	return handle, nil
}

func (t *CABIDaemonTransport) statusForHandle(handleID string, handle uint64) ([]byte, error) {
	var out *C.char
	code := int32(C.easynet_runtime_call_daemon_status(t.symbols.daemonStatus, C.uint64_t(handle), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon status failed")
	}
	raw := cabiTakeCString(t.symbols.stringFree, out)
	status, err := daemonStatusFromCABI(handleID, raw)
	if err != nil {
		return nil, err
	}
	return json.Marshal(status)
}

func (t *CABIDaemonTransport) callDaemonDiscover(optionsJSON []byte) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(optionsJSON, func(cOptions *C.char) C.int32_t {
		return C.easynet_runtime_call_daemon_discover(t.symbols.daemonDiscover, cOptions, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon discover failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIDaemonTransport) detachCHandle(handle uint64) error {
	code := int32(C.easynet_runtime_call_daemon_detach(t.symbols.daemonDetach, C.uint64_t(handle)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI daemon detach failed")
	}
	return nil
}

func (t *CABIDaemonTransport) lastErrorOrCode(code int32, fallback string) error {
	return cabiRuntimeLastErrorOrCode(t.symbols, code, fallback)
}

// CABIRuntimeTransport is an optional non-stream Runtime Core transport over
// libeasynet_cli. Stream and bidi callbacks are implemented in a separate
// lifecycle slice.
type CABIRuntimeTransport struct {
	mu          sync.Mutex
	symbols     cabiRuntimeSymbols
	handle      uint64
	ownsHandle  bool
	preparedIDs map[string]uint64
	closed      bool
}

func newCABIRuntimeTransport(symbols cabiRuntimeSymbols, handle uint64, ownsHandle bool) *CABIRuntimeTransport {
	return &CABIRuntimeTransport{
		symbols:     symbols,
		handle:      handle,
		ownsHandle:  ownsHandle,
		preparedIDs: map[string]uint64{},
	}
}

func (t *CABIRuntimeTransport) RuntimeHealth(ctx context.Context) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(C.easynet_runtime_call_health(t.symbols.runtimeHealth, C.uint64_t(handle), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI runtime health failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_runtime_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation invoke failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	if _, err := t.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	_ = draftJSON
	return nil, nil, cabiNotImplemented("C ABI Go stream transport is not implemented in this slice")
}

func (t *CABIRuntimeTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	if _, err := t.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	_, _ = draftJSON, streamsJSON
	return nil, nil, cabiNotImplemented("C ABI Go bidi transport is not implemented in this slice")
}

func (t *CABIRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var outID C.uint64_t
	var out *C.char
	code := int32(cabiWithCStringPair(draftJSON, optionsJSON, func(cDraft *C.char, cOptions *C.char) C.int32_t {
		return C.easynet_runtime_call_prepare(t.symbols.invocationPrepare, C.uint64_t(handle), cDraft, cOptions, &outID, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation prepare failed")
	}
	raw := cabiTakeCString(t.symbols.stringFree, out)
	preparedID := uint64(outID)
	if preparedID == 0 {
		return nil, invalidCABIHandle("C ABI prepare returned an invalid prepared handle")
	}
	key, err := preparedKeyFromJSON(raw)
	if err != nil {
		_ = t.freePreparedID(preparedID)
		return nil, err
	}
	t.mu.Lock()
	if _, exists := t.preparedIDs[key]; exists {
		t.mu.Unlock()
		_ = t.freePreparedID(preparedID)
		return nil, &SDKError{
			Code:      ErrProtocol,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "C ABI prepare returned a duplicate prepared request id",
		}
	}
	t.preparedIDs[key] = preparedID
	t.mu.Unlock()
	return raw, nil
}

func (t *CABIRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	key, signatureJSON, err := signedInvocationCABIFields(signedJSON)
	if err != nil {
		return nil, err
	}
	t.mu.Lock()
	preparedID := t.preparedIDs[key]
	delete(t.preparedIDs, key)
	t.mu.Unlock()
	if preparedID == 0 {
		return nil, &SDKError{
			Code:      ErrInvalidHandle,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "prepared invocation handle is not owned by this transport",
		}
	}
	var signedID C.uint64_t
	var ignored *C.char
	code := int32(cabiWithCString(signatureJSON, func(cSignature *C.char) C.int32_t {
		return C.easynet_runtime_call_sign_prepared(t.symbols.signPrepared, C.uint64_t(preparedID), cSignature, &signedID, &ignored)
	}))
	if ignored != nil {
		_ = cabiTakeCString(t.symbols.stringFree, ignored)
	}
	if code != 0 {
		_ = t.freePreparedID(preparedID)
		return nil, t.lastErrorOrCode(code, "C ABI invocation sign prepared failed")
	}
	if signedID == 0 {
		_ = t.freePreparedID(preparedID)
		return nil, invalidCABIHandle("C ABI sign returned an invalid signed handle")
	}
	var outHandle C.uint64_t
	var out *C.char
	code = int32(C.easynet_runtime_call_submit_signed_handle(t.symbols.submitSignedHandle, C.uint64_t(handle), signedID, &outHandle, &out))
	if code != 0 {
		_ = t.freeSignedID(uint64(signedID))
		return nil, t.lastErrorOrCode(code, "C ABI invocation submit signed failed")
	}
	if outHandle == 0 {
		_ = t.freeSignedID(uint64(signedID))
		return nil, invalidCABIHandle("C ABI submit returned an invalid invocation handle")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) AwaitHandle(ctx context.Context, handleID uint64) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(C.easynet_runtime_call_handle_await(t.symbols.handleAwait, C.uint64_t(handle), C.uint64_t(handleID), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle await failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) CancelHandle(ctx context.Context, handleID uint64, reason string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(cabiWithCString([]byte(reason), func(cReason *C.char) C.int32_t {
		return C.easynet_runtime_call_handle_cancel(t.symbols.handleCancel, C.uint64_t(handle), C.uint64_t(handleID), cReason, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle cancel failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) HandleEvents(ctx context.Context, handleID uint64) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(C.easynet_runtime_call_handle_events(t.symbols.handleEvents, C.uint64_t(handle), C.uint64_t(handleID), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle events failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIRuntimeTransport) FreeHandle(ctx context.Context, handleID uint64) error {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return err
	}
	code := int32(C.easynet_runtime_call_handle_free(t.symbols.handleFree, C.uint64_t(handle), C.uint64_t(handleID)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI invocation handle free failed")
	}
	return nil
}

func (t *CABIRuntimeTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	t.closed = true
	preparedIDs := make([]uint64, 0, len(t.preparedIDs))
	for _, id := range t.preparedIDs {
		preparedIDs = append(preparedIDs, id)
	}
	t.preparedIDs = map[string]uint64{}
	handle := t.handle
	ownsHandle := t.ownsHandle
	t.handle = 0
	t.mu.Unlock()

	var first error
	for _, id := range preparedIDs {
		if err := t.freePreparedID(id); err != nil && first == nil {
			first = err
		}
	}
	if ownsHandle && handle != 0 {
		code := int32(C.easynet_runtime_call_shutdown(t.symbols.shutdown, C.uint64_t(handle)))
		if code != 0 && first == nil {
			first = t.lastErrorOrCode(code, "C ABI runtime shutdown failed")
		}
	}
	return first
}

func (t *CABIRuntimeTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI runtime transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("runtime transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIRuntimeTransport) freePreparedID(id uint64) error {
	code := int32(C.easynet_runtime_call_prepared_free(t.symbols.preparedFree, C.uint64_t(id)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI prepared invocation free failed")
	}
	return nil
}

func (t *CABIRuntimeTransport) freeSignedID(id uint64) error {
	code := int32(C.easynet_runtime_call_signed_free(t.symbols.signedFree, C.uint64_t(id)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI signed invocation free failed")
	}
	return nil
}

func (t *CABIRuntimeTransport) lastErrorOrCode(code int32, fallback string) error {
	return cabiRuntimeLastErrorOrCode(t.symbols, code, fallback)
}

func bindCABIRuntimeSymbols(library unsafe.Pointer) (cabiRuntimeSymbols, error) {
	var symbols cabiRuntimeSymbols
	bindings := []struct {
		name string
		out  *unsafe.Pointer
	}{
		{"easynet_abi_version", &symbols.abiVersion},
		{"easynet_last_error_json", &symbols.lastErrorJSON},
		{"easynet_string_free", &symbols.stringFree},
		{"easynet_daemon_start", &symbols.daemonStart},
		{"easynet_daemon_attach", &symbols.daemonAttach},
		{"easynet_daemon_discover", &symbols.daemonDiscover},
		{"easynet_daemon_stop", &symbols.daemonStop},
		{"easynet_daemon_detach", &symbols.daemonDetach},
		{"easynet_daemon_status", &symbols.daemonStatus},
		{"easynet_daemon_open_client", &symbols.daemonOpenClient},
		{"easynet_shutdown", &symbols.shutdown},
		{"easynet_runtime_health", &symbols.runtimeHealth},
		{"easynet_invocation_invoke", &symbols.invocationInvoke},
		{"easynet_invocation_prepare", &symbols.invocationPrepare},
		{"easynet_invocation_sign_prepared", &symbols.signPrepared},
		{"easynet_invocation_submit_signed_handle", &symbols.submitSignedHandle},
		{"easynet_invocation_handle_await", &symbols.handleAwait},
		{"easynet_invocation_handle_cancel", &symbols.handleCancel},
		{"easynet_invocation_handle_events", &symbols.handleEvents},
		{"easynet_invocation_handle_free", &symbols.handleFree},
		{"easynet_prepared_invocation_free", &symbols.preparedFree},
		{"easynet_signed_invocation_free", &symbols.signedFree},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiRuntimeSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiRuntimeLastErrorOrCode(symbols cabiRuntimeSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_runtime_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiTakeCString(symbols.stringFree, out)
		if decoded, err := DecodeDaemonErrorJSON(raw); err == nil && decoded != nil {
			return decoded
		}
	}
	return &SDKError{
		Code:      ErrGeneric,
		Stage:     "cabi",
		Retry:     RetryUnknown,
		Retryable: false,
		Message:   fmt.Sprintf("%s with code %d", fallback, code),
	}
}

func cabiWithCString(payload []byte, call func(*C.char) C.int32_t) C.int32_t {
	cPayload := C.CString(string(payload))
	defer C.free(unsafe.Pointer(cPayload))
	return call(cPayload)
}

func cabiWithCStringPair(left []byte, right []byte, call func(*C.char, *C.char) C.int32_t) C.int32_t {
	cLeft := C.CString(string(left))
	defer C.free(unsafe.Pointer(cLeft))
	cRight := C.CString(string(right))
	defer C.free(unsafe.Pointer(cRight))
	return call(cLeft, cRight)
}

func cabiTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_runtime_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}

func daemonStartConfigForCABI(configJSON []byte) ([]byte, error) {
	var config map[string]any
	if err := json.Unmarshal(configJSON, &config); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode daemon start config: %v", err), err)
	}
	unsupported := []string{}
	for _, key := range []string{"uds_path", "listen_tcp", "tls_cert_path", "tls_key_path", "hub_endpoint", "trust_path"} {
		if !emptyCABIConfigValue(config[key]) {
			unsupported = append(unsupported, key)
		}
	}
	if len(unsupported) > 0 {
		return nil, &SDKError{
			Code:      ErrNotImplemented,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "C ABI daemon start does not support fields: " + fmt.Sprint(unsupported),
		}
	}
	projected := map[string]any{}
	for _, key := range []string{"mode", "realm", "daemon_bin", "log_path", "env"} {
		if !emptyCABIConfigValue(config[key]) {
			projected[key] = config[key]
		}
	}
	if !emptyCABIConfigValue(config["device_id"]) {
		projected["node_id"] = config["device_id"]
	}
	if !emptyCABIConfigValue(config["node_id"]) {
		projected["node_id"] = config["node_id"]
	}
	if value, ok := config["detached"].(bool); ok && value {
		projected["detach"] = true
	}
	if value, ok := config["detach"].(bool); ok && value {
		projected["detach"] = true
	}
	return json.Marshal(projected)
}

func emptyCABIConfigValue(value any) bool {
	switch typed := value.(type) {
	case nil:
		return true
	case string:
		return typed == ""
	case bool:
		return !typed
	case map[string]any:
		return len(typed) == 0
	default:
		return false
	}
}

func daemonStatusFromCABI(handleID string, raw []byte) (map[string]any, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode daemon status: %v", err), err)
	}
	status := map[string]any{
		"state":       daemonStateFromCABI(decoded),
		"endpoints":   daemonEndpointsFromCABI(decoded),
		"diagnostics": stringListFromAny(decoded["diagnostics"]),
	}
	if handleID != "0" {
		status["handle_id"] = handleID
	}
	for _, key := range []string{"mode", "version", "message"} {
		if value, ok := decoded[key].(string); ok && value != "" {
			status[key] = value
		}
	}
	if value, ok := decoded["pid"].(float64); ok && value >= 0 {
		status["pid"] = int(value)
	}
	return status, nil
}

func daemonEndpointsFromCABI(decoded map[string]any) map[string]any {
	if endpoints, ok := decoded["endpoints"].(map[string]any); ok {
		return map[string]any{
			"control_endpoint":    stringField(endpoints, "control_endpoint"),
			"invocation_endpoint": stringField(endpoints, "invocation_endpoint"),
			"public_endpoint":     stringField(endpoints, "public_endpoint"),
		}
	}
	return map[string]any{
		"control_endpoint":    stringField(decoded, "control_endpoint"),
		"invocation_endpoint": stringField(decoded, "invocation_endpoint"),
		"public_endpoint":     stringField(decoded, "public_endpoint"),
	}
}

func daemonStateFromCABI(decoded map[string]any) string {
	if state, ok := decoded["state"].(string); ok && state != "" {
		return state
	}
	if ready, ok := decoded["invocation_accepting"].(bool); ok && ready {
		return string(DaemonRunning)
	}
	if ready, ok := decoded["control_accepting"].(bool); ok && ready {
		return string(DaemonControlOnly)
	}
	if alive, ok := decoded["pid_alive"].(bool); ok && alive {
		return string(DaemonControlReady)
	}
	return string(DaemonStopped)
}

func stringField(values map[string]any, key string) string {
	if value, ok := values[key].(string); ok {
		return value
	}
	return ""
}

func stringListFromAny(value any) []string {
	raw, ok := value.([]any)
	if !ok {
		return []string{}
	}
	out := make([]string, 0, len(raw))
	for _, item := range raw {
		if value, ok := item.(string); ok {
			out = append(out, value)
		}
	}
	return out
}

func preparedKeyFromJSON(raw []byte) (string, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return "", invalidRuntimePayload(fmt.Sprintf("decode prepared invocation: %v", err), err)
	}
	return preparedKeyFromMap(decoded)
}

func signedInvocationCABIFields(raw []byte) (string, []byte, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return "", nil, invalidRuntimePayload(fmt.Sprintf("decode signed invocation: %v", err), err)
	}
	prepared, ok := decoded["prepared"].(map[string]any)
	if !ok {
		return "", nil, invalidRuntimePayload("signed invocation prepared object is required", nil)
	}
	signature, ok := decoded["signature"].(map[string]any)
	if !ok {
		return "", nil, invalidRuntimePayload("signed invocation signature object is required", nil)
	}
	key, err := preparedKeyFromMap(prepared)
	if err != nil {
		return "", nil, err
	}
	signatureJSON, err := json.Marshal(signature)
	if err != nil {
		return "", nil, invalidRuntimePayload(fmt.Sprintf("encode signed invocation signature: %v", err), err)
	}
	return key, signatureJSON, nil
}

func preparedKeyFromMap(decoded map[string]any) (string, error) {
	for _, key := range []string{"prepared_id", "request_id"} {
		if value, ok := decoded[key].(string); ok && value != "" {
			return value, nil
		}
	}
	return "", invalidRuntimePayload("prepared_id or request_id is required", nil)
}

func invalidCABIHandle(message string) error {
	return &SDKError{
		Code:      ErrInvalidHandle,
		Stage:     "cabi",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}

func cabiNotImplemented(message string) error {
	return &SDKError{
		Code:      ErrNotImplemented,
		Stage:     "cabi",
		Retry:     RetryNever,
		Retryable: false,
		Message:   message,
	}
}
