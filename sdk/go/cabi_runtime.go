//go:build runtime_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*runtime_cabi_abi_version_fn)(void);
typedef int32_t (*runtime_cabi_last_error_json_fn)(char **out_error_json);
typedef void (*runtime_cabi_string_free_fn)(char *s);
typedef int32_t (*runtime_host_start_fn)(const char *config_json, uint64_t *out_host_handle);
typedef int32_t (*runtime_host_attach_fn)(const char *options_json, uint64_t *out_host_handle);
typedef int32_t (*runtime_host_discover_fn)(const char *options_json, char **out_discovery_json);
typedef int32_t (*runtime_host_stop_fn)(uint64_t handle);
typedef int32_t (*runtime_host_detach_fn)(uint64_t handle);
typedef int32_t (*runtime_host_status_fn)(uint64_t handle, char **out_status_json);
typedef int32_t (*runtime_host_open_client_fn)(uint64_t host_handle, uint64_t *out_handle);
typedef int32_t (*runtime_shutdown_fn)(uint64_t handle);
typedef int32_t (*runtime_health_fn)(uint64_t handle, char **out_health_json);
typedef int32_t (*runtime_diagnostics_fn)(uint64_t handle, char **out_diagnostics_json);
typedef int32_t (*runtime_resolve_descriptor_ref_fn)(uint64_t handle, const char *request_json, char **out_descriptor_json);
typedef int32_t (*runtime_invocation_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*runtime_invocation_prepare_fn)(uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json);
typedef int32_t (*runtime_invocation_sign_prepared_fn)(uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json);
typedef int32_t (*runtime_invocation_sign_prepared_local_fn)(uint64_t prepared_id, uint64_t *out_signed_id, char **out_signed_json);
typedef int32_t (*runtime_invocation_submit_signed_handle_fn)(uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json);
typedef int32_t (*runtime_invocation_handle_await_fn)(uint64_t handle, uint64_t invocation_handle_id, char **out_result_json);
typedef int32_t (*runtime_invocation_handle_cancel_fn)(uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json);
typedef int32_t (*runtime_invocation_handle_events_fn)(uint64_t handle, uint64_t invocation_handle_id, char **out_events_json);
typedef int32_t (*runtime_invocation_handle_free_fn)(uint64_t handle, uint64_t invocation_handle_id);
typedef int32_t (*runtime_prepared_invocation_free_fn)(uint64_t prepared_id);
typedef int32_t (*runtime_signed_invocation_free_fn)(uint64_t signed_id);
typedef void (*runtime_stream_callback_fn)(void *user_data, const char *chunk_json);
typedef void (*runtime_bidi_callback_fn)(void *user_data, const char *frame_json);
typedef int32_t (*runtime_invocation_stream_open_fn)(uint64_t handle, const char *invocation_json, runtime_stream_callback_fn on_chunk, void *user_data, uint64_t *out_stream_id);
typedef int32_t (*runtime_invocation_stream_cancel_fn)(uint64_t handle, uint64_t stream_id);
typedef int32_t (*runtime_invocation_stream_close_fn)(uint64_t handle, uint64_t stream_id);
typedef int32_t (*runtime_invocation_bidi_open_fn)(uint64_t handle, const char *invocation_json, runtime_bidi_callback_fn on_frame, void *user_data, uint64_t *out_bidi_id);
typedef int32_t (*runtime_invocation_bidi_send_fn)(uint64_t handle, uint64_t bidi_id, const char *frame_json);
typedef int32_t (*runtime_invocation_bidi_close_send_fn)(uint64_t handle, uint64_t bidi_id);
typedef int32_t (*runtime_invocation_bidi_close_fn)(uint64_t handle, uint64_t bidi_id);
typedef int32_t (*runtime_invocation_bidi_cancel_fn)(uint64_t handle, uint64_t bidi_id);

extern void easynetGoStreamCallback(void *user_data, const char *chunk_json);
extern void easynetGoBidiCallback(void *user_data, const char *frame_json);

static uint32_t runtime_cabi_call_abi_version(void *fn) {
	return ((runtime_cabi_abi_version_fn)fn)();
}

static int32_t runtime_cabi_call_last_error_json(void *fn, char **out_error_json) {
	return ((runtime_cabi_last_error_json_fn)fn)(out_error_json);
}

static void runtime_cabi_call_string_free(void *fn, char *s) {
	((runtime_cabi_string_free_fn)fn)(s);
}

static int32_t runtime_cabi_call_host_start(void *fn, const char *config_json, uint64_t *out_host_handle) {
	return ((runtime_host_start_fn)fn)(config_json, out_host_handle);
}

static int32_t runtime_cabi_call_host_attach(void *fn, const char *options_json, uint64_t *out_host_handle) {
	return ((runtime_host_attach_fn)fn)(options_json, out_host_handle);
}

static int32_t runtime_cabi_call_host_discover(void *fn, const char *options_json, char **out_discovery_json) {
	return ((runtime_host_discover_fn)fn)(options_json, out_discovery_json);
}

static int32_t runtime_cabi_call_host_stop(void *fn, uint64_t handle) {
	return ((runtime_host_stop_fn)fn)(handle);
}

static int32_t runtime_cabi_call_host_detach(void *fn, uint64_t handle) {
	return ((runtime_host_detach_fn)fn)(handle);
}

static int32_t runtime_cabi_call_host_status(void *fn, uint64_t handle, char **out_status_json) {
	return ((runtime_host_status_fn)fn)(handle, out_status_json);
}

static int32_t runtime_cabi_call_host_open_client(void *fn, uint64_t host_handle, uint64_t *out_handle) {
	return ((runtime_host_open_client_fn)fn)(host_handle, out_handle);
}

static int32_t runtime_cabi_call_shutdown(void *fn, uint64_t handle) {
	return ((runtime_shutdown_fn)fn)(handle);
}

static int32_t runtime_cabi_call_health(void *fn, uint64_t handle, char **out_health_json) {
	return ((runtime_health_fn)fn)(handle, out_health_json);
}

static int32_t runtime_cabi_call_diagnostics(void *fn, uint64_t handle, char **out_diagnostics_json) {
	return ((runtime_diagnostics_fn)fn)(handle, out_diagnostics_json);
}

static int32_t runtime_cabi_call_resolve_descriptor_ref(void *fn, uint64_t handle, const char *request_json, char **out_descriptor_json) {
	return ((runtime_resolve_descriptor_ref_fn)fn)(handle, request_json, out_descriptor_json);
}

static int32_t runtime_cabi_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((runtime_invocation_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t runtime_cabi_call_prepare(void *fn, uint64_t handle, const char *invocation_json, const char *options_json, uint64_t *out_prepared_id, char **out_prepared_json) {
	return ((runtime_invocation_prepare_fn)fn)(handle, invocation_json, options_json, out_prepared_id, out_prepared_json);
}

static int32_t runtime_cabi_call_sign_prepared(void *fn, uint64_t prepared_id, const char *signature_json, uint64_t *out_signed_id, char **out_signed_json) {
	return ((runtime_invocation_sign_prepared_fn)fn)(prepared_id, signature_json, out_signed_id, out_signed_json);
}

static int32_t runtime_cabi_call_sign_prepared_local(void *fn, uint64_t prepared_id, uint64_t *out_signed_id, char **out_signed_json) {
	return ((runtime_invocation_sign_prepared_local_fn)fn)(prepared_id, out_signed_id, out_signed_json);
}

static int32_t runtime_cabi_call_submit_signed_handle(void *fn, uint64_t handle, uint64_t signed_id, uint64_t *out_invocation_handle_id, char **out_submitted_json) {
	return ((runtime_invocation_submit_signed_handle_fn)fn)(handle, signed_id, out_invocation_handle_id, out_submitted_json);
}

static int32_t runtime_cabi_call_handle_await(void *fn, uint64_t handle, uint64_t invocation_handle_id, char **out_result_json) {
	return ((runtime_invocation_handle_await_fn)fn)(handle, invocation_handle_id, out_result_json);
}

static int32_t runtime_cabi_call_handle_cancel(void *fn, uint64_t handle, uint64_t invocation_handle_id, const char *reason_json, char **out_cancel_json) {
	return ((runtime_invocation_handle_cancel_fn)fn)(handle, invocation_handle_id, reason_json, out_cancel_json);
}

static int32_t runtime_cabi_call_handle_events(void *fn, uint64_t handle, uint64_t invocation_handle_id, char **out_events_json) {
	return ((runtime_invocation_handle_events_fn)fn)(handle, invocation_handle_id, out_events_json);
}

static int32_t runtime_cabi_call_handle_free(void *fn, uint64_t handle, uint64_t invocation_handle_id) {
	return ((runtime_invocation_handle_free_fn)fn)(handle, invocation_handle_id);
}

static int32_t runtime_cabi_call_prepared_free(void *fn, uint64_t prepared_id) {
	return ((runtime_prepared_invocation_free_fn)fn)(prepared_id);
}

static int32_t runtime_cabi_call_signed_free(void *fn, uint64_t signed_id) {
	return ((runtime_signed_invocation_free_fn)fn)(signed_id);
}

static int32_t runtime_cabi_call_stream_open(void *fn, uint64_t handle, const char *invocation_json, void *user_data, uint64_t *out_stream_id) {
	return ((runtime_invocation_stream_open_fn)fn)(handle, invocation_json, easynetGoStreamCallback, user_data, out_stream_id);
}

static int32_t runtime_cabi_call_stream_cancel(void *fn, uint64_t handle, uint64_t stream_id) {
	return ((runtime_invocation_stream_cancel_fn)fn)(handle, stream_id);
}

static int32_t runtime_cabi_call_stream_close(void *fn, uint64_t handle, uint64_t stream_id) {
	return ((runtime_invocation_stream_close_fn)fn)(handle, stream_id);
}

static int32_t runtime_cabi_call_bidi_open(void *fn, uint64_t handle, const char *invocation_json, void *user_data, uint64_t *out_bidi_id) {
	return ((runtime_invocation_bidi_open_fn)fn)(handle, invocation_json, easynetGoBidiCallback, user_data, out_bidi_id);
}

static int32_t runtime_cabi_call_bidi_send(void *fn, uint64_t handle, uint64_t bidi_id, const char *frame_json) {
	return ((runtime_invocation_bidi_send_fn)fn)(handle, bidi_id, frame_json);
}

static int32_t runtime_cabi_call_bidi_close_send(void *fn, uint64_t handle, uint64_t bidi_id) {
	return ((runtime_invocation_bidi_close_send_fn)fn)(handle, bidi_id);
}

static int32_t runtime_cabi_call_bidi_close(void *fn, uint64_t handle, uint64_t bidi_id) {
	return ((runtime_invocation_bidi_close_fn)fn)(handle, bidi_id);
}

static int32_t runtime_cabi_call_bidi_cancel(void *fn, uint64_t handle, uint64_t bidi_id) {
	return ((runtime_invocation_bidi_cancel_fn)fn)(handle, bidi_id);
}
*/
import "C"

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
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
	runtimeDiagnostics unsafe.Pointer
	resolveDescriptor  unsafe.Pointer
	invocationInvoke   unsafe.Pointer
	invocationPrepare  unsafe.Pointer
	signPrepared       unsafe.Pointer
	signPreparedLocal  unsafe.Pointer
	submitSignedHandle unsafe.Pointer
	handleAwait        unsafe.Pointer
	handleCancel       unsafe.Pointer
	handleEvents       unsafe.Pointer
	handleFree         unsafe.Pointer
	preparedFree       unsafe.Pointer
	signedFree         unsafe.Pointer
	streamOpen         unsafe.Pointer
	streamCancel       unsafe.Pointer
	streamClose        unsafe.Pointer
	bidiOpen           unsafe.Pointer
	bidiSend           unsafe.Pointer
	bidiCloseSend      unsafe.Pointer
	bidiClose          unsafe.Pointer
	bidiCancel         unsafe.Pointer
}

// cabiRuntimeLifecycleTransport is the package-private native provider binding
// over libeasynet_cli. It keeps C ABI handles private and exposes only generic
// runtime lifecycle DTOs through RuntimeLifecycleTransport.
type cabiRuntimeLifecycleTransport struct {
	mu       sync.Mutex
	library  unsafe.Pointer
	symbols  cabiRuntimeSymbols
	handles  map[string]uint64
	runtimes map[*cabiRuntimeTransport]struct{}
	closed   bool
}

// openCABIRuntimeLifecycleTransport loads libeasynet_cli and assembles the
// package-private native provider transport.
func openCABIRuntimeLifecycleTransport(path string) (*cabiRuntimeLifecycleTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIRuntimeSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.runtime_cabi_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	return &cabiRuntimeLifecycleTransport{
		library:  library,
		symbols:  symbols,
		handles:  map[string]uint64{},
		runtimes: map[*cabiRuntimeTransport]struct{}{},
	}, nil
}

func (t *cabiRuntimeLifecycleTransport) Discover(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	raw, err := t.callRuntimeHostDiscover(optionsJSON)
	if err != nil {
		return nil, err
	}
	status, err := runtimeHostStatusFromCABI("0", raw)
	if err != nil {
		return nil, err
	}
	return json.Marshal(status["endpoints"])
}

func (t *cabiRuntimeLifecycleTransport) Start(ctx context.Context, configJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	projected, err := runtimeHostStartConfigForCABI(configJSON)
	if err != nil {
		return nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(projected, func(cConfig *C.char) C.int32_t {
		return C.runtime_cabi_call_host_start(t.symbols.daemonStart, cConfig, &out)
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

func (t *cabiRuntimeLifecycleTransport) Attach(ctx context.Context, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(optionsJSON, func(cOptions *C.char) C.int32_t {
		return C.runtime_cabi_call_host_attach(t.symbols.daemonAttach, cOptions, &out)
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

func (t *cabiRuntimeLifecycleTransport) Status(ctx context.Context, handleID string) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	handle, err := t.requireRuntimeHostHandle(handleID)
	if err != nil {
		return nil, err
	}
	return t.statusForHandle(handleID, handle)
}

func (t *cabiRuntimeLifecycleTransport) OpenRuntime(ctx context.Context, handleID string, optionsJSON []byte) (RuntimeTransport, []byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, nil, err
	}
	_ = optionsJSON
	runtimeHostHandle, err := t.requireRuntimeHostHandle(handleID)
	if err != nil {
		return nil, nil, err
	}
	runtimeHandle, err := t.openClientHandle(runtimeHostHandle, "runtime")
	if err != nil {
		return nil, nil, err
	}
	runtime := newCABIRuntimeTransport(t.symbols, runtimeHandle, true)
	t.mu.Lock()
	t.runtimes[runtime] = struct{}{}
	t.mu.Unlock()
	return runtime, []byte(fmt.Sprintf(`{"ready":true,"abi_version":%d,"transport":"c_abi"}`, expectedCABIABIVersion)), nil
}

func (t *cabiRuntimeLifecycleTransport) Stop(ctx context.Context, handleID string, optionsJSON []byte) ([]byte, error) {
	if err := t.requireOpen(ctx); err != nil {
		return nil, err
	}
	_ = optionsJSON
	handle, err := t.requireRuntimeHostHandle(handleID)
	if err != nil {
		return nil, err
	}
	code := int32(C.runtime_cabi_call_host_stop(t.symbols.daemonStop, C.uint64_t(handle)))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon stop failed")
	}
	t.mu.Lock()
	delete(t.handles, handleID)
	t.mu.Unlock()
	return []byte(fmt.Sprintf(`{"handle_id":%q,"state":"Stopped","diagnostics":[]}`, handleID)), nil
}

func (t *cabiRuntimeLifecycleTransport) Detach(ctx context.Context, handleID string) error {
	if err := t.requireOpen(ctx); err != nil {
		return err
	}
	handle, err := t.requireRuntimeHostHandle(handleID)
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

func (t *cabiRuntimeLifecycleTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	t.closed = true
	runtimes := make([]*cabiRuntimeTransport, 0, len(t.runtimes))
	for runtime := range t.runtimes {
		runtimes = append(runtimes, runtime)
	}
	handles := make([]uint64, 0, len(t.handles))
	for _, handle := range t.handles {
		handles = append(handles, handle)
	}
	t.runtimes = map[*cabiRuntimeTransport]struct{}{}
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

func (t *cabiRuntimeLifecycleTransport) requireOpen(ctx context.Context) error {
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

func (t *cabiRuntimeLifecycleTransport) requireRuntimeHostHandle(handleID string) (uint64, error) {
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

func (t *cabiRuntimeLifecycleTransport) statusForHandle(handleID string, handle uint64) ([]byte, error) {
	var out *C.char
	code := int32(C.runtime_cabi_call_host_status(t.symbols.daemonStatus, C.uint64_t(handle), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon status failed")
	}
	raw := cabiTakeCString(t.symbols.stringFree, out)
	status, err := runtimeHostStatusFromCABI(handleID, raw)
	if err != nil {
		return nil, err
	}
	return json.Marshal(status)
}

func (t *cabiRuntimeLifecycleTransport) callRuntimeHostDiscover(optionsJSON []byte) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(optionsJSON, func(cOptions *C.char) C.int32_t {
		return C.runtime_cabi_call_host_discover(t.symbols.daemonDiscover, cOptions, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI daemon discover failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeLifecycleTransport) detachCHandle(handle uint64) error {
	code := int32(C.runtime_cabi_call_host_detach(t.symbols.daemonDetach, C.uint64_t(handle)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI daemon detach failed")
	}
	return nil
}

func (t *cabiRuntimeLifecycleTransport) lastErrorOrCode(code int32, fallback string) error {
	return cabiRuntimeLastErrorOrCode(t.symbols, code, fallback)
}

func (t *cabiRuntimeLifecycleTransport) openClientHandle(runtimeHostHandle uint64, profile string) (uint64, error) {
	var out C.uint64_t
	code := int32(C.runtime_cabi_call_host_open_client(t.symbols.daemonOpenClient, C.uint64_t(runtimeHostHandle), &out))
	if code != 0 {
		return 0, t.lastErrorOrCode(code, "C ABI daemon open "+profile+" client failed")
	}
	clientHandle := uint64(out)
	if clientHandle == 0 {
		return 0, invalidCABIHandle("C ABI daemon open " + profile + " returned an invalid client handle")
	}
	return clientHandle, nil
}

// cabiRuntimeTransport is the package-private native provider implementation of
// RuntimeTransport and HealthTransport over libeasynet_cli.
type cabiRuntimeTransport struct {
	mu              sync.Mutex
	symbols         cabiRuntimeSymbols
	handle          uint64
	ownsHandle      bool
	preparedHandles *cabiPreparedHandleRegistry
	streams         map[*cabiStreamTransport]struct{}
	bidis           map[*cabiBidiTransport]struct{}
	closed          bool
}

func newCABIRuntimeTransport(symbols cabiRuntimeSymbols, handle uint64, ownsHandle bool) *cabiRuntimeTransport {
	return &cabiRuntimeTransport{
		symbols:         symbols,
		handle:          handle,
		ownsHandle:      ownsHandle,
		preparedHandles: newCABIPreparedHandleRegistry(),
		streams:         map[*cabiStreamTransport]struct{}{},
		bidis:           map[*cabiBidiTransport]struct{}{},
	}
}

type cabiPreparedHandleState uint8

const (
	cabiPreparedHandleReady cabiPreparedHandleState = iota
	cabiPreparedHandleSigning
)

type cabiPreparedHandle struct {
	nativeID uint64
	state    cabiPreparedHandleState
}

type cabiPreparedHandleRegistry struct {
	mu      sync.Mutex
	handles map[string]cabiPreparedHandle
}

func newCABIPreparedHandleRegistry() *cabiPreparedHandleRegistry {
	return &cabiPreparedHandleRegistry{handles: map[string]cabiPreparedHandle{}}
}

func (r *cabiPreparedHandleRegistry) register(key string, preparedID uint64, freePrepared func(uint64) error) error {
	r.mu.Lock()
	_, exists := r.handles[key]
	if !exists {
		r.handles[key] = cabiPreparedHandle{nativeID: preparedID, state: cabiPreparedHandleReady}
	}
	r.mu.Unlock()
	if !exists {
		return nil
	}
	_ = freePrepared(preparedID)
	return &SDKError{
		Code:      ErrProtocol,
		Stage:     "cabi",
		Retry:     RetryNever,
		Retryable: false,
		Message:   "C ABI prepare returned a duplicate prepared handle id",
	}
}

func (r *cabiPreparedHandleRegistry) claimForSigning(key string) (uint64, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	handle, ok := r.handles[key]
	if !ok || handle.nativeID == 0 {
		return 0, &SDKError{
			Code:      ErrInvalidHandle,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "prepared invocation handle is not owned by this transport",
		}
	}
	if handle.state != cabiPreparedHandleReady {
		return 0, &SDKError{
			Code:      ErrInvalidHandle,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   "prepared invocation handle is already being signed",
		}
	}
	handle.state = cabiPreparedHandleSigning
	r.handles[key] = handle
	return handle.nativeID, nil
}

func (r *cabiPreparedHandleRegistry) releaseSigningClaim(key string, preparedID uint64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	handle, ok := r.handles[key]
	if ok && handle.nativeID == preparedID && handle.state == cabiPreparedHandleSigning {
		handle.state = cabiPreparedHandleReady
		r.handles[key] = handle
	}
}

func (r *cabiPreparedHandleRegistry) consumeSigningClaim(key string, preparedID uint64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	handle, ok := r.handles[key]
	if ok && handle.nativeID == preparedID && handle.state == cabiPreparedHandleSigning {
		delete(r.handles, key)
	}
}

func (r *cabiPreparedHandleRegistry) drain() []uint64 {
	r.mu.Lock()
	defer r.mu.Unlock()
	preparedIDs := make([]uint64, 0, len(r.handles))
	for _, handle := range r.handles {
		preparedIDs = append(preparedIDs, handle.nativeID)
	}
	r.handles = map[string]cabiPreparedHandle{}
	return preparedIDs
}

func (t *cabiRuntimeTransport) RuntimeHealth(ctx context.Context) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(C.runtime_cabi_call_health(t.symbols.runtimeHealth, C.uint64_t(handle), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI runtime health failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) RuntimeDiagnostics(ctx context.Context) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(C.runtime_cabi_call_diagnostics(t.symbols.runtimeDiagnostics, C.uint64_t(handle), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI runtime diagnostics failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) ResolveDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(cabiWithCString(requestJSON, func(cRequest *C.char) C.int32_t {
		return C.runtime_cabi_call_resolve_descriptor_ref(
			t.symbols.resolveDescriptor,
			C.uint64_t(handle),
			cRequest,
			&out,
		)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI runtime descriptor_ref resolver failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.runtime_cabi_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation invoke failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	inbox := newCABICallbackInbox(MaxStreamBufferedEvents)
	registration, err := registerCABICallbackInbox(inbox)
	if err != nil {
		return nil, nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.runtime_cabi_call_stream_open(t.symbols.streamOpen, C.uint64_t(handle), cDraft, registration.userData, &out)
	}))
	if code != 0 {
		releaseCABICallbackInbox(registration)
		return nil, nil, t.lastErrorOrCode(code, "C ABI invocation stream open failed")
	}
	streamID := uint64(out)
	if streamID == 0 {
		releaseCABICallbackInbox(registration)
		return nil, nil, invalidCABIHandle("C ABI stream open returned an invalid stream id")
	}
	stream := &cabiStreamTransport{
		owner:        t,
		streamID:     streamID,
		registration: registration,
		inbox:        inbox,
		nextRecvSeq:  1,
	}
	t.mu.Lock()
	t.streams[stream] = struct{}{}
	t.mu.Unlock()
	return stream, []byte(fmt.Sprintf(`{"stream_id":%q,"state":"Open","max_buffered_events":%d}`, strconv.FormatUint(streamID, 10), MaxStreamBufferedEvents)), nil
}

func (t *cabiRuntimeTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, nil, err
	}
	invocationJSON, err := mergeBidiStreamsForCABI(draftJSON, streamsJSON)
	if err != nil {
		return nil, nil, err
	}
	inbox := newCABICallbackInbox(MaxBidiBufferedFrames)
	registration, err := registerCABICallbackInbox(inbox)
	if err != nil {
		return nil, nil, err
	}
	var out C.uint64_t
	code := int32(cabiWithCString(invocationJSON, func(cDraft *C.char) C.int32_t {
		return C.runtime_cabi_call_bidi_open(t.symbols.bidiOpen, C.uint64_t(handle), cDraft, registration.userData, &out)
	}))
	if code != 0 {
		releaseCABICallbackInbox(registration)
		return nil, nil, t.lastErrorOrCode(code, "C ABI invocation bidi open failed")
	}
	bidiID := uint64(out)
	if bidiID == 0 {
		releaseCABICallbackInbox(registration)
		return nil, nil, invalidCABIHandle("C ABI bidi open returned an invalid session id")
	}
	bidi := &cabiBidiTransport{
		owner:        t,
		bidiID:       bidiID,
		registration: registration,
		inbox:        inbox,
		nextRecvSeq:  1,
	}
	t.mu.Lock()
	t.bidis[bidi] = struct{}{}
	t.mu.Unlock()
	openJSON, err := runtimeBidiOpenJSON(strconv.FormatUint(bidiID, 10), MaxBidiBufferedFrames)
	if err != nil {
		_ = bidi.Close(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode C ABI bidi open JSON: %v", err), err)
	}
	return bidi, openJSON, nil
}

func (t *cabiRuntimeTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	var outID C.uint64_t
	var out *C.char
	code := int32(cabiWithCStringPair(draftJSON, optionsJSON, func(cDraft *C.char, cOptions *C.char) C.int32_t {
		return C.runtime_cabi_call_prepare(t.symbols.invocationPrepare, C.uint64_t(handle), cDraft, cOptions, &outID, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation prepare failed")
	}
	raw := cabiTakeCString(t.symbols.stringFree, out)
	preparedID := uint64(outID)
	var options struct {
		MaterialOnly bool `json:"material_only"`
	}
	if err := json.Unmarshal(optionsJSON, &options); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode prepare options: %v", err), err)
	}
	if options.MaterialOnly {
		if preparedID != 0 {
			_ = t.freePreparedID(preparedID)
			return nil, invalidCABIHandle("C ABI material-only prepare retained a prepared handle")
		}
		return raw, nil
	}
	if preparedID == 0 {
		return nil, invalidCABIHandle("C ABI prepare returned an invalid prepared handle")
	}
	key, err := preparedKeyFromJSON(raw)
	if err != nil {
		_ = t.freePreparedID(preparedID)
		return nil, err
	}
	if err := t.preparedHandles.register(key, preparedID, t.freePreparedID); err != nil {
		return nil, err
	}
	return raw, nil
}

func (t *cabiRuntimeTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	fields, err := signedInvocationCABIFields(signedJSON)
	if err != nil {
		return nil, err
	}
	preparedID, err := t.preparedHandles.claimForSigning(fields.key)
	if err != nil {
		return nil, err
	}
	var signedID C.uint64_t
	var ignored *C.char
	var code int32
	if fields.localDaemonSigning {
		code = int32(C.runtime_cabi_call_sign_prepared_local(t.symbols.signPreparedLocal, C.uint64_t(preparedID), &signedID, &ignored))
	} else {
		code = int32(cabiWithCString(fields.signatureJSON, func(cSignature *C.char) C.int32_t {
			return C.runtime_cabi_call_sign_prepared(t.symbols.signPrepared, C.uint64_t(preparedID), cSignature, &signedID, &ignored)
		}))
	}
	if ignored != nil {
		_ = cabiTakeCString(t.symbols.stringFree, ignored)
	}
	if code != 0 {
		t.preparedHandles.releaseSigningClaim(fields.key, preparedID)
		return nil, t.lastErrorOrCode(code, "C ABI invocation sign prepared transition failed")
	}
	if signedID == 0 {
		t.preparedHandles.releaseSigningClaim(fields.key, preparedID)
		return nil, invalidCABIHandle("C ABI sign returned an invalid signed handle")
	}
	t.preparedHandles.consumeSigningClaim(fields.key, preparedID)
	var outHandle C.uint64_t
	var out *C.char
	code = int32(C.runtime_cabi_call_submit_signed_handle(t.symbols.submitSignedHandle, C.uint64_t(handle), signedID, &outHandle, &out))
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

func (t *cabiRuntimeTransport) AwaitHandle(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	handleID := control.adapterHandleID()
	var out *C.char
	code := int32(C.runtime_cabi_call_handle_await(t.symbols.handleAwait, C.uint64_t(handle), C.uint64_t(handleID), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle await failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) CancelHandle(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	handleID := control.adapterHandleID()
	var out *C.char
	code := int32(cabiWithCString([]byte(reason), func(cReason *C.char) C.int32_t {
		return C.runtime_cabi_call_handle_cancel(t.symbols.handleCancel, C.uint64_t(handle), C.uint64_t(handleID), cReason, &out)
	}))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle cancel failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) HandleEvents(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	handleID := control.adapterHandleID()
	var out *C.char
	code := int32(C.runtime_cabi_call_handle_events(t.symbols.handleEvents, C.uint64_t(handle), C.uint64_t(handleID), &out))
	if code != 0 {
		return nil, t.lastErrorOrCode(code, "C ABI invocation handle events failed")
	}
	return cabiTakeCString(t.symbols.stringFree, out), nil
}

func (t *cabiRuntimeTransport) FreeHandle(ctx context.Context, control InvocationControlCapability) error {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return err
	}
	handleID := control.adapterHandleID()
	code := int32(C.runtime_cabi_call_handle_free(t.symbols.handleFree, C.uint64_t(handle), C.uint64_t(handleID)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI invocation handle free failed")
	}
	return nil
}

func (t *cabiRuntimeTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidRuntimeClient("context is required")
	}
	t.mu.Lock()
	if t.closed {
		t.mu.Unlock()
		return nil
	}
	t.closed = true
	streams := make([]*cabiStreamTransport, 0, len(t.streams))
	for stream := range t.streams {
		streams = append(streams, stream)
	}
	bidis := make([]*cabiBidiTransport, 0, len(t.bidis))
	for bidi := range t.bidis {
		bidis = append(bidis, bidi)
	}
	preparedIDs := t.preparedHandles.drain()
	t.streams = map[*cabiStreamTransport]struct{}{}
	t.bidis = map[*cabiBidiTransport]struct{}{}
	handle := t.handle
	ownsHandle := t.ownsHandle
	t.handle = 0
	t.mu.Unlock()

	var first error
	for _, stream := range streams {
		if err := stream.closeFromOwner(handle); err != nil && first == nil {
			first = err
		}
	}
	for _, bidi := range bidis {
		if err := bidi.closeFromOwner(handle); err != nil && first == nil {
			first = err
		}
	}
	for _, id := range preparedIDs {
		if err := t.freePreparedID(id); err != nil && first == nil {
			first = err
		}
	}
	if ownsHandle && handle != 0 {
		code := int32(C.runtime_cabi_call_shutdown(t.symbols.shutdown, C.uint64_t(handle)))
		if code != 0 && first == nil {
			first = t.lastErrorOrCode(code, "C ABI runtime shutdown failed")
		}
	}
	return first
}

func (t *cabiRuntimeTransport) requireOpen(ctx context.Context) (uint64, error) {
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

func (t *cabiRuntimeTransport) freePreparedID(id uint64) error {
	code := int32(C.runtime_cabi_call_prepared_free(t.symbols.preparedFree, C.uint64_t(id)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI prepared invocation free failed")
	}
	return nil
}

func (t *cabiRuntimeTransport) freeSignedID(id uint64) error {
	code := int32(C.runtime_cabi_call_signed_free(t.symbols.signedFree, C.uint64_t(id)))
	if code != 0 {
		return t.lastErrorOrCode(code, "C ABI signed invocation free failed")
	}
	return nil
}

func (t *cabiRuntimeTransport) removeStream(stream *cabiStreamTransport) {
	t.mu.Lock()
	delete(t.streams, stream)
	t.mu.Unlock()
}

func (t *cabiRuntimeTransport) removeBidi(bidi *cabiBidiTransport) {
	t.mu.Lock()
	delete(t.bidis, bidi)
	t.mu.Unlock()
}

func (t *cabiRuntimeTransport) lastErrorOrCode(code int32, fallback string) error {
	return cabiRuntimeLastErrorOrCode(t.symbols, code, fallback)
}

type cabiStreamTransport struct {
	mu           sync.Mutex
	owner        *cabiRuntimeTransport
	streamID     uint64
	registration *cabiCallbackRegistration
	inbox        *cabiCallbackInbox
	nextRecvSeq  uint64
	closed       bool
	cancelSent   bool
	cancelErr    error
}

func (s *cabiStreamTransport) Recv(ctx context.Context) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if s == nil || s.inbox == nil {
		return nil, invalidRuntimeClient("C ABI stream transport is not initialized")
	}
	raw, err := s.inbox.recv(ctx)
	if err != nil {
		return nil, err
	}
	return projectCABIOrderedEvent(raw, s.allocateSequence, true)
}

func (s *cabiStreamTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	handle, err := s.owner.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	_ = reason
	if err := s.cancelWithHandle(handle); err != nil {
		return nil, err
	}
	return []byte(fmt.Sprintf(`{"stream_id":%q,"cancel_requested":true,"cancelled":false,"state":"CancelRequested","terminal":false}`, strconv.FormatUint(s.streamID, 10))), nil
}

func (s *cabiStreamTransport) Close(ctx context.Context) error {
	handle, err := s.owner.requireOpen(ctx)
	if err != nil {
		return err
	}
	return s.closeWithHandle(handle)
}

func (s *cabiStreamTransport) closeFromOwner(handle uint64) error {
	return s.closeWithHandle(handle)
}

func (s *cabiStreamTransport) cancelWithHandle(handle uint64) error {
	if s == nil || s.owner == nil {
		return invalidRuntimeClient("C ABI stream transport is not initialized")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return nil
	}
	if s.cancelSent {
		return s.cancelErr
	}
	streamID := s.streamID

	code := int32(C.runtime_cabi_call_stream_cancel(s.owner.symbols.streamCancel, C.uint64_t(handle), C.uint64_t(streamID)))
	s.cancelSent = true
	if code != 0 {
		s.cancelErr = s.owner.lastErrorOrCode(code, "C ABI invocation stream cancel failed")
		return s.cancelErr
	}
	return nil
}

func (s *cabiStreamTransport) closeWithHandle(handle uint64) error {
	if s == nil || s.owner == nil {
		return invalidRuntimeClient("C ABI stream transport is not initialized")
	}
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil
	}
	s.closed = true
	streamID := s.streamID
	registration := s.registration
	s.mu.Unlock()

	code := int32(C.runtime_cabi_call_stream_close(s.owner.symbols.streamClose, C.uint64_t(handle), C.uint64_t(streamID)))
	releaseCABICallbackInbox(registration)
	s.owner.removeStream(s)
	if code != 0 {
		return s.owner.lastErrorOrCode(code, "C ABI invocation stream close failed")
	}
	return nil
}

type cabiBidiTransport struct {
	mu           sync.Mutex
	owner        *cabiRuntimeTransport
	bidiID       uint64
	registration *cabiCallbackRegistration
	inbox        *cabiCallbackInbox
	nextRecvSeq  uint64
	closed       bool
	cancelSent   bool
	cancelErr    error
}

func (b *cabiBidiTransport) Send(ctx context.Context, frameJSON []byte) ([]byte, error) {
	handle, err := b.owner.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	if err := b.requireOpen(); err != nil {
		return nil, err
	}
	wireJSON, err := cabiBidiFrameJSON(frameJSON)
	if err != nil {
		return nil, err
	}
	code := int32(cabiWithCString(wireJSON, func(cFrame *C.char) C.int32_t {
		return C.runtime_cabi_call_bidi_send(b.owner.symbols.bidiSend, C.uint64_t(handle), C.uint64_t(b.bidiID), cFrame)
	}))
	if code != 0 {
		return nil, b.owner.lastErrorOrCode(code, "C ABI invocation bidi send failed")
	}
	return append([]byte(nil), frameJSON...), nil
}

func cabiBidiFrameJSON(frameJSON []byte) ([]byte, error) {
	frame, err := NewBidiFrameFromJSON(frameJSON)
	if err != nil {
		return nil, err
	}
	switch frame.Kind() {
	case "data":
		wire := map[string]any{
			"type":      "binary_chunk",
			"stream_id": frame.StreamID(),
		}
		if payload := frame.PayloadBase64(); payload != "" {
			wire["data_base64"] = payload
		} else if rawJSON := frame.PayloadJSON(); len(rawJSON) > 0 && string(rawJSON) != "null" {
			encoded, err := json.Marshal(json.RawMessage(rawJSON))
			if err != nil {
				return nil, invalidRuntimePayload(fmt.Sprintf("encode bidi JSON payload: %v", err), err)
			}
			wire["data_base64"] = base64.StdEncoding.EncodeToString(encoded)
		} else {
			wire["data_base64"] = ""
		}
		return json.Marshal(wire)
	case "eof", "close_send":
		return json.Marshal(map[string]any{"type": "control", "eof": true})
	case "control":
		wire := map[string]any{"type": "control"}
		if rawJSON := frame.PayloadJSON(); len(rawJSON) > 0 && string(rawJSON) != "null" {
			var payload map[string]any
			if err := json.Unmarshal(rawJSON, &payload); err != nil {
				return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi control payload_json: %v", err), err)
			}
			for key, value := range payload {
				wire[key] = value
			}
		}
		return json.Marshal(wire)
	default:
		return nil, invalidRuntimePayload(fmt.Sprintf("unsupported C ABI bidi frame kind: %s", frame.Kind()), nil)
	}
}

func (b *cabiBidiTransport) Recv(ctx context.Context) ([]byte, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	if b == nil || b.inbox == nil {
		return nil, invalidRuntimeClient("C ABI bidi transport is not initialized")
	}
	raw, err := b.inbox.recv(ctx)
	if err != nil {
		return nil, err
	}
	return projectCABIOrderedEvent(raw, b.allocateSequence, false)
}

func (b *cabiBidiTransport) CloseSend(ctx context.Context) ([]byte, error) {
	handle, err := b.owner.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	if err := b.requireOpen(); err != nil {
		return nil, err
	}
	code := int32(C.runtime_cabi_call_bidi_close_send(b.owner.symbols.bidiCloseSend, C.uint64_t(handle), C.uint64_t(b.bidiID)))
	if code != 0 {
		return nil, b.owner.lastErrorOrCode(code, "C ABI invocation bidi close-send failed")
	}
	return []byte(fmt.Sprintf(`{"session_id":%q,"state":"HalfClosedLocal","terminal":false}`, strconv.FormatUint(b.bidiID, 10))), nil
}

func (b *cabiBidiTransport) Close(ctx context.Context) error {
	handle, err := b.owner.requireOpen(ctx)
	if err != nil {
		return err
	}
	return b.closeWithHandle(handle)
}

func (b *cabiBidiTransport) Cancel(ctx context.Context, reason string) ([]byte, error) {
	handle, err := b.owner.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	_ = reason
	if err := b.cancelWithHandle(handle); err != nil {
		return nil, err
	}
	return []byte(fmt.Sprintf(`{"session_id":%q,"state":"CancelRequested","terminal":false,"reason":"cancelled"}`, strconv.FormatUint(b.bidiID, 10))), nil
}

func (b *cabiBidiTransport) closeFromOwner(handle uint64) error {
	return b.closeWithHandle(handle)
}

func (b *cabiBidiTransport) closeWithHandle(handle uint64) error {
	if b == nil || b.owner == nil {
		return invalidRuntimeClient("C ABI bidi transport is not initialized")
	}
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return nil
	}
	b.closed = true
	bidiID := b.bidiID
	registration := b.registration
	b.mu.Unlock()

	code := int32(C.runtime_cabi_call_bidi_close(b.owner.symbols.bidiClose, C.uint64_t(handle), C.uint64_t(bidiID)))
	releaseCABICallbackInbox(registration)
	b.owner.removeBidi(b)
	if code != 0 {
		return b.owner.lastErrorOrCode(code, "C ABI invocation bidi close failed")
	}
	return nil
}

func (b *cabiBidiTransport) cancelWithHandle(handle uint64) error {
	if b == nil || b.owner == nil {
		return invalidRuntimeClient("C ABI bidi transport is not initialized")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return nil
	}
	if b.cancelSent {
		return b.cancelErr
	}
	bidiID := b.bidiID

	code := int32(C.runtime_cabi_call_bidi_cancel(b.owner.symbols.bidiCancel, C.uint64_t(handle), C.uint64_t(bidiID)))
	b.cancelSent = true
	if code != 0 {
		b.cancelErr = b.owner.lastErrorOrCode(code, "C ABI invocation bidi cancel failed")
		return b.cancelErr
	}
	return nil
}

func (b *cabiBidiTransport) requireOpen() error {
	if b == nil || b.owner == nil {
		return invalidRuntimeClient("C ABI bidi transport is not initialized")
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return invalidRuntimeClient("C ABI bidi transport is closed")
	}
	return nil
}

func (s *cabiStreamTransport) allocateSequence(observed *uint64) uint64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	if observed != nil && *observed > 0 {
		if s.nextRecvSeq <= *observed {
			s.nextRecvSeq = *observed + 1
		}
		return *observed
	}
	sequence := s.nextRecvSeq
	s.nextRecvSeq++
	return sequence
}

func (b *cabiBidiTransport) allocateSequence(observed *uint64) uint64 {
	b.mu.Lock()
	defer b.mu.Unlock()
	if observed != nil && *observed > 0 {
		if b.nextRecvSeq <= *observed {
			b.nextRecvSeq = *observed + 1
		}
		return *observed
	}
	sequence := b.nextRecvSeq
	b.nextRecvSeq++
	return sequence
}

func projectCABIOrderedEvent(raw []byte, allocateSequence func(*uint64) uint64, useObservedSequence bool) ([]byte, error) {
	var event map[string]any
	if err := json.Unmarshal(raw, &event); err != nil {
		return raw, nil
	}
	var observed *uint64
	if useObservedSequence {
		if sequence, ok := cabiPositiveJSONInteger(event["sequence"]); ok {
			observed = &sequence
		}
	}
	event["sequence"] = allocateSequence(observed)
	if state, ok := cabiJSONInteger(event["state"]); ok {
		event["state"] = cabiInvocationStateName(state)
	}
	if _, ok := event["error"]; !ok {
		if _, hasCode := event["code"]; hasCode {
			event["error"] = cabiCallbackError(event)
		} else if _, hasMessage := event["message"]; hasMessage {
			event["error"] = cabiCallbackError(event)
		}
	}
	projected, err := json.Marshal(event)
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode projected C ABI callback frame: %v", err), err)
	}
	return projected, nil
}

func cabiPositiveJSONInteger(value any) (uint64, bool) {
	number, ok := cabiJSONInteger(value)
	if !ok || number <= 0 {
		return 0, false
	}
	return uint64(number), true
}

func cabiJSONInteger(value any) (int64, bool) {
	switch typed := value.(type) {
	case float64:
		if math.Trunc(typed) != typed {
			return 0, false
		}
		return int64(typed), true
	case int64:
		return typed, true
	case int:
		return int64(typed), true
	case uint64:
		if typed > math.MaxInt64 {
			return 0, false
		}
		return int64(typed), true
	default:
		return 0, false
	}
}

func cabiCallbackError(event map[string]any) map[string]any {
	return map[string]any{
		"code":    cabiStringOrEmpty(event["code"]),
		"message": cabiStringOrEmpty(event["message"]),
	}
}

func cabiStringOrEmpty(value any) string {
	if typed, ok := value.(string); ok {
		return typed
	}
	return ""
}

func resolveDescriptorRefFromDiagnostics(requestJSON []byte, diagnosticsJSON []byte) ([]byte, error) {
	var request map[string]any
	if len(requestJSON) == 0 {
		requestJSON = []byte(`{}`)
	}
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode descriptor_ref resolution request: %v", err), err)
	}
	calleeURA := strings.TrimSpace(cabiStringOrEmpty(request["callee_ura"]))
	ability := strings.TrimSpace(cabiStringOrEmpty(request["ability"]))
	callMode := strings.TrimSpace(cabiStringOrEmpty(request["call_mode"]))
	if calleeURA == "" || ability == "" {
		return nil, invalidRuntimePayload("callee_ura and ability are required for descriptor_ref resolution", nil)
	}
	if callMode == "" {
		return nil, invalidRuntimePayload("call_mode is required for descriptor_ref resolution", nil)
	}
	abilityIsURA := strings.HasPrefix(ability, URAScheme)

	var diagnostics map[string]any
	if err := json.Unmarshal(diagnosticsJSON, &diagnostics); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode runtime diagnostics: %v", err), err)
	}
	catalog, _ := diagnostics["descriptor_catalog"].(map[string]any)
	if catalog == nil {
		return nil, invalidRuntimePayload("runtime diagnostics omitted descriptor_catalog", nil)
	}
	rawEntries, _ := catalog["entries"].([]any)
	if rawEntries == nil {
		return nil, invalidRuntimePayload("runtime descriptor_catalog.entries must be an array", nil)
	}
	source := cabiStringOrEmpty(catalog["source"])
	for _, rawEntry := range rawEntries {
		entry, _ := rawEntry.(map[string]any)
		if entry == nil {
			continue
		}
		entryAbilityURA := strings.TrimSpace(cabiStringOrEmpty(entry["ability_ura"]))
		if abilityIsURA {
			if entryAbilityURA != ability {
				continue
			}
			entryCallMode, err := cabiRequiredCatalogString(entry, "call_mode", ability, source)
			if err != nil {
				return nil, err
			}
			if entryCallMode != callMode {
				continue
			}
			descriptorRef, err := cabiRequiredCatalogString(entry, "descriptor_ref", ability, source)
			if err != nil {
				return nil, err
			}
			entryOwnerURA, err := cabiRequiredCatalogString(entry, "owner_ura", ability, source)
			if err != nil {
				return nil, err
			}
			entryName, err := cabiRequiredCatalogString(entry, "name", ability, source)
			if err != nil {
				return nil, err
			}
			return json.Marshal(map[string]any{
				"descriptor_ref": descriptorRef,
				"ability_ura":    entryAbilityURA,
				"owner_ura":      entryOwnerURA,
				"name":           entryName,
				"call_mode":      callMode,
				"source":         source,
			})
		}
		entryOwnerURA := strings.TrimSpace(cabiStringOrEmpty(entry["owner_ura"]))
		entryName := strings.TrimSpace(cabiStringOrEmpty(entry["name"]))
		if entryOwnerURA != calleeURA || ability != entryName {
			continue
		}
		entryCallMode, err := cabiRequiredCatalogString(entry, "call_mode", ability, source)
		if err != nil {
			return nil, err
		}
		if entryCallMode != callMode {
			continue
		}
		descriptorRef, err := cabiRequiredCatalogString(entry, "descriptor_ref", ability, source)
		if err != nil {
			return nil, err
		}
		entryAbilityURA, err = cabiRequiredCatalogString(entry, "ability_ura", ability, source)
		if err != nil {
			return nil, err
		}
		return json.Marshal(map[string]any{
			"descriptor_ref": descriptorRef,
			"ability_ura":    entryAbilityURA,
			"owner_ura":      entryOwnerURA,
			"name":           entryName,
			"call_mode":      callMode,
			"source":         source,
		})
	}
	return nil, &SDKError{
		Code:      ErrDescriptorNotFound,
		Stage:     "cabi",
		Retry:     RetryNever,
		Retryable: RetryableForHint(RetryNever),
		Message: fmt.Sprintf(
			"descriptor_ref not found for callee_ura=%q ability=%q call_mode=%q",
			calleeURA,
			ability,
			callMode,
		),
	}
}

func cabiRequiredCatalogString(entry map[string]any, field string, ability string, source string) (string, error) {
	value := strings.TrimSpace(cabiStringOrEmpty(entry[field]))
	if value != "" {
		return value, nil
	}
	if strings.TrimSpace(source) == "" {
		source = "descriptor_catalog"
	}
	return "", invalidRuntimePayload(
		fmt.Sprintf("descriptor catalog row for ability %q from %s missing %s", ability, source, field),
		nil,
	)
}

func cabiInvocationStateName(state int64) string {
	switch state {
	case 1:
		return "Accepted"
	case 2:
		return "Admitted"
	case 3:
		return "Dispatched"
	case 4:
		return "Running"
	case 5:
		return "Completed"
	case 6:
		return "Failed"
	case 7:
		return "TimedOut"
	case 8:
		return "Cancelled"
	default:
		return strconv.FormatInt(state, 10)
	}
}

func bindCABIRuntimeSymbols(library unsafe.Pointer) (cabiRuntimeSymbols, error) {
	var symbols cabiRuntimeSymbols
	bindings := []struct {
		name string
		out  *unsafe.Pointer
	}{
		{"runtime_abi_version", &symbols.abiVersion},
		{"runtime_last_error_json", &symbols.lastErrorJSON},
		{"runtime_string_free", &symbols.stringFree},
		{"runtime_host_start", &symbols.daemonStart},
		{"runtime_host_attach", &symbols.daemonAttach},
		{"runtime_host_discover", &symbols.daemonDiscover},
		{"runtime_host_stop", &symbols.daemonStop},
		{"runtime_host_detach", &symbols.daemonDetach},
		{"runtime_host_status", &symbols.daemonStatus},
		{"runtime_host_open_client", &symbols.daemonOpenClient},
		{"runtime_shutdown", &symbols.shutdown},
		{"runtime_health", &symbols.runtimeHealth},
		{"runtime_diagnostics", &symbols.runtimeDiagnostics},
		{"runtime_resolve_descriptor_ref", &symbols.resolveDescriptor},
		{"runtime_invocation_invoke", &symbols.invocationInvoke},
		{"runtime_invocation_prepare", &symbols.invocationPrepare},
		{"runtime_invocation_sign_prepared", &symbols.signPrepared},
		{"runtime_invocation_sign_prepared_local", &symbols.signPreparedLocal},
		{"runtime_invocation_submit_signed_handle", &symbols.submitSignedHandle},
		{"runtime_invocation_handle_await", &symbols.handleAwait},
		{"runtime_invocation_handle_cancel", &symbols.handleCancel},
		{"runtime_invocation_handle_events", &symbols.handleEvents},
		{"runtime_invocation_handle_free", &symbols.handleFree},
		{"runtime_prepared_invocation_free", &symbols.preparedFree},
		{"runtime_signed_invocation_free", &symbols.signedFree},
		{"runtime_invocation_stream_open", &symbols.streamOpen},
		{"runtime_invocation_stream_cancel", &symbols.streamCancel},
		{"runtime_invocation_stream_close", &symbols.streamClose},
		{"runtime_invocation_bidi_open", &symbols.bidiOpen},
		{"runtime_invocation_bidi_send", &symbols.bidiSend},
		{"runtime_invocation_bidi_close_send", &symbols.bidiCloseSend},
		{"runtime_invocation_bidi_close", &symbols.bidiClose},
		{"runtime_invocation_bidi_cancel", &symbols.bidiCancel},
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
	errCode := int32(C.runtime_cabi_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiTakeCString(symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON(raw, true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
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
	defer C.runtime_cabi_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}

func runtimeHostStartConfigForCABI(configJSON []byte) ([]byte, error) {
	var config map[string]any
	if err := json.Unmarshal(configJSON, &config); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode runtime host start config: %v", err), err)
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
			Message:   "C ABI runtime host start does not support fields: " + fmt.Sprint(unsupported),
		}
	}
	projected := map[string]any{}
	for _, key := range []string{"mode", "realm", "device_id", "daemon_bin", "working_dir", "log_path", "env"} {
		if !emptyCABIConfigValue(config[key]) {
			projected[key] = config[key]
		}
	}
	if value, ok := config["detached"].(bool); ok && value {
		projected["detached"] = true
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

func runtimeHostStatusFromCABI(handleID string, raw []byte) (map[string]any, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode runtime host status: %v", err), err)
	}
	status := map[string]any{
		"state":       runtimeHostStateFromCABI(decoded),
		"endpoints":   runtimeHostEndpointsFromCABI(decoded),
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

func runtimeHostEndpointsFromCABI(decoded map[string]any) map[string]any {
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

func runtimeHostStateFromCABI(decoded map[string]any) string {
	if state, ok := decoded["state"].(string); ok && state != "" {
		return state
	}
	if ready, ok := decoded["invocation_accepting"].(bool); ok && ready {
		return string(RuntimeRunning)
	}
	if ready, ok := decoded["control_accepting"].(bool); ok && ready {
		return string(RuntimeControlOnly)
	}
	if alive, ok := decoded["pid_alive"].(bool); ok && alive {
		return string(RuntimeControlReady)
	}
	return string(RuntimeStopped)
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

type cabiSignedInvocationFields struct {
	key                string
	signatureJSON      []byte
	localDaemonSigning bool
}

func signedInvocationCABIFields(raw []byte) (cabiSignedInvocationFields, error) {
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		return cabiSignedInvocationFields{}, invalidRuntimePayload(fmt.Sprintf("decode signed invocation: %v", err), err)
	}
	prepared, ok := decoded["prepared"].(map[string]any)
	if !ok {
		return cabiSignedInvocationFields{}, invalidRuntimePayload("signed invocation prepared object is required", nil)
	}
	key, err := preparedKeyFromMap(prepared)
	if err != nil {
		return cabiSignedInvocationFields{}, err
	}
	localSigning, err := signedInvocationUsesLocalDaemonSigning(decoded)
	if err != nil {
		return cabiSignedInvocationFields{}, err
	}
	if localSigning {
		return cabiSignedInvocationFields{key: key, localDaemonSigning: true}, nil
	}
	signature, ok := decoded["signature"].(map[string]any)
	if !ok {
		return cabiSignedInvocationFields{}, invalidRuntimePayload("signed invocation signature object is required", nil)
	}
	signatureJSON, err := json.Marshal(signature)
	if err != nil {
		return cabiSignedInvocationFields{}, invalidRuntimePayload(fmt.Sprintf("encode signed invocation signature: %v", err), err)
	}
	return cabiSignedInvocationFields{key: key, signatureJSON: signatureJSON}, nil
}

func signedInvocationUsesLocalDaemonSigning(decoded map[string]any) (bool, error) {
	value, ok := decoded["policy"]
	if !ok || value == nil {
		return false, nil
	}
	policy, ok := value.(map[string]any)
	if !ok {
		return false, invalidRuntimePayload("signed invocation policy object is required", nil)
	}
	mode, ok := policy["mode"]
	if !ok || mode == nil {
		return false, nil
	}
	modeString, ok := mode.(string)
	if !ok {
		return false, invalidRuntimePayload("signed invocation policy mode must be a string", nil)
	}
	return modeString == "local_daemon_signing", nil
}

func mergeBidiStreamsForCABI(draftJSON []byte, streamsJSON []byte) ([]byte, error) {
	var draft map[string]any
	if err := json.Unmarshal(draftJSON, &draft); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi invocation draft: %v", err), err)
	}
	var streams []any
	if err := json.Unmarshal(streamsJSON, &streams); err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("decode bidi stream descriptors: %v", err), err)
	}
	if len(streams) == 0 {
		return nil, invalidRuntimePayload("bidi_streams must be a non-empty array", nil)
	}
	for _, stream := range streams {
		if _, ok := stream.(map[string]any); !ok {
			return nil, invalidRuntimePayload("bidi_streams entries must be objects", nil)
		}
	}
	draft["bidi_streams"] = streams
	return json.Marshal(draft)
}

func preparedKeyFromMap(decoded map[string]any) (string, error) {
	if value, ok := decoded["prepared_id"].(string); ok && value != "" {
		return value, nil
	}
	return "", invalidRuntimePayload("prepared_id is required", nil)
}

type cabiCallbackInbox struct {
	mu               sync.Mutex
	ch               chan []byte
	closed           bool
	failure          []byte
	failureDelivered bool
}

func newCABICallbackInbox(maxItems int) *cabiCallbackInbox {
	if maxItems <= 0 {
		maxItems = 1
	}
	return &cabiCallbackInbox{ch: make(chan []byte, maxItems)}
}

func (i *cabiCallbackInbox) push(raw []byte) {
	i.mu.Lock()
	defer i.mu.Unlock()
	if i.closed {
		return
	}
	copied := append([]byte(nil), raw...)
	select {
	case i.ch <- copied:
	default:
		i.failure = cabiCallbackBackpressureFailure()
		i.closed = true
		close(i.ch)
	}
}

func (i *cabiCallbackInbox) recv(ctx context.Context) ([]byte, error) {
	i.mu.Lock()
	if i.failure != nil && !i.failureDelivered {
		i.failureDelivered = true
		failure := append([]byte(nil), i.failure...)
		i.mu.Unlock()
		return failure, nil
	}
	i.mu.Unlock()
	select {
	case raw, ok := <-i.ch:
		if ok {
			return raw, nil
		}
		return nil, invalidRuntimeClient("C ABI callback inbox is closed")
	case <-ctx.Done():
		return nil, cabiContextError(ctx)
	}
}

func cabiCallbackBackpressureFailure() []byte {
	return []byte(`{"kind":"error","state":"Failed","terminal":false,"transport_terminal":true,"error":{"code":"ADMISSION_DENIED","stage":"cabi_callback","message":"C ABI callback queue limit exceeded","retry":"after_backoff","details":{"wire_code":"RESOURCE_EXHAUSTED","reason":"callback_queue_overflow","bounded_queue":true}}}`)
}

func (i *cabiCallbackInbox) close() {
	i.mu.Lock()
	defer i.mu.Unlock()
	if i.closed {
		return
	}
	i.closed = true
	close(i.ch)
}

var cabiCallbackRegistry = struct {
	sync.Mutex
	next  uintptr
	inbox map[uintptr]*cabiCallbackInbox
}{next: 1, inbox: map[uintptr]*cabiCallbackInbox{}}

type cabiCallbackRegistration struct {
	token    uintptr
	userData unsafe.Pointer
}

func registerCABICallbackInbox(inbox *cabiCallbackInbox) (*cabiCallbackRegistration, error) {
	cabiCallbackRegistry.Lock()
	token := cabiCallbackRegistry.next
	cabiCallbackRegistry.next++
	cabiCallbackRegistry.inbox[token] = inbox
	cabiCallbackRegistry.Unlock()

	userData := C.malloc(C.size_t(unsafe.Sizeof(C.uintptr_t(0))))
	if userData == nil {
		cabiCallbackRegistry.Lock()
		delete(cabiCallbackRegistry.inbox, token)
		cabiCallbackRegistry.Unlock()
		return nil, &SDKError{
			Code:      ErrGeneric,
			Stage:     "cabi",
			Retry:     RetryAfterBackoff,
			Retryable: true,
			Message:   "allocate C ABI callback registration",
		}
	}
	*(*C.uintptr_t)(userData) = C.uintptr_t(token)
	return &cabiCallbackRegistration{token: token, userData: userData}, nil
}

func releaseCABICallbackInbox(registration *cabiCallbackRegistration) {
	if registration == nil {
		return
	}
	cabiCallbackRegistry.Lock()
	inbox := cabiCallbackRegistry.inbox[registration.token]
	delete(cabiCallbackRegistry.inbox, registration.token)
	cabiCallbackRegistry.Unlock()
	C.free(registration.userData)
	if inbox != nil {
		inbox.close()
	}
}

func pushCABICallbackPayload(token uintptr, raw []byte) {
	cabiCallbackRegistry.Lock()
	inbox := cabiCallbackRegistry.inbox[token]
	cabiCallbackRegistry.Unlock()
	if inbox != nil {
		inbox.push(raw)
	}
}

func cabiContextError(ctx context.Context) error {
	code := ErrCancelled
	retry := RetryNever
	retryable := false
	if ctx.Err() == context.DeadlineExceeded {
		code = ErrTimeout
		retry = RetrySafe
		retryable = true
	}
	return &SDKError{
		Code:      code,
		Stage:     "cabi",
		Retry:     retry,
		Retryable: retryable,
		Message:   ctx.Err().Error(),
		Cause:     ctx.Err(),
	}
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
