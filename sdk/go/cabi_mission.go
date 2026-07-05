//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_mission_abi_version_fn)(void);
typedef int32_t (*easynet_mission_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_mission_string_free_fn)(char *s);
typedef int32_t (*easynet_mission_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_mission_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_mission_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_mission_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_mission_call_abi_version(void *fn) {
	return ((easynet_mission_abi_version_fn)fn)();
}

static int32_t easynet_mission_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_mission_last_error_json_fn)fn)(out_error_json);
}

static void easynet_mission_call_string_free(void *fn, char *s) {
	((easynet_mission_string_free_fn)fn)(s);
}

static int32_t easynet_mission_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_mission_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_mission_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_mission_shutdown_fn)fn)(handle);
}

static int32_t easynet_mission_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_mission_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_mission_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_mission_json_fn)fn)(handle, request_json, out_json);
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

type cabiMissionSymbols struct {
	abiVersion             unsafe.Pointer
	lastErrorJSON          unsafe.Pointer
	stringFree             unsafe.Pointer
	init                   unsafe.Pointer
	shutdown               unsafe.Pointer
	invocationInvoke       unsafe.Pointer
	buildRunEALInvocation  unsafe.Pointer
	buildRunFileInvocation unsafe.Pointer
	buildTrackInvocation   unsafe.Pointer
	buildCancelInvocation  unsafe.Pointer
	buildEventsInvocation  unsafe.Pointer
	projectStatus          unsafe.Pointer
	projectEvents          unsafe.Pointer
}

// CABIMissionTransport is an optional Mission profile transport over
// libeasynet_cli. It keeps C ABI handles private while delegating Mission
// carrier construction and status projection to the Rust-owned daemon SDK
// contract.
type CABIMissionTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiMissionSymbols
	handle  uint64
	closed  bool
}

var _ MissionTransport = (*CABIMissionTransport)(nil)

// OpenCABIMissionTransport loads libeasynet_cli and opens a Mission profile transport.
func OpenCABIMissionTransport(path string, controlPath string) (*CABIMissionTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIMissionSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_mission_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionIncompatible,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiMissionInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIMissionTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIMissionClient creates a MissionClient over libeasynet_cli.
func NewCABIMissionClient(path string, controlPath string) (*MissionClient, *CABIMissionTransport, error) {
	transport, err := OpenCABIMissionTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewMissionClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIMissionTransport) BuildRunEALInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildRunEALInvocation, requestJSON, "C ABI mission run invocation build failed")
}

func (t *CABIMissionTransport) BuildRunFileInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildRunFileInvocation, requestJSON, "C ABI mission run-file invocation build failed")
}

func (t *CABIMissionTransport) BuildTrackInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildTrackInvocation, requestJSON, "C ABI mission track invocation build failed")
}

func (t *CABIMissionTransport) BuildCancelInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildCancelInvocation, requestJSON, "C ABI mission cancel invocation build failed")
}

func (t *CABIMissionTransport) RunEAL(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProjectStatus(ctx, requestJSON, t.symbols.buildRunEALInvocation, "C ABI mission run failed")
}

func (t *CABIMissionTransport) RunFile(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProjectStatus(ctx, requestJSON, t.symbols.buildRunFileInvocation, "C ABI mission run-file failed")
}

func (t *CABIMissionTransport) Track(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProjectStatus(ctx, requestJSON, t.symbols.buildTrackInvocation, "C ABI mission track failed")
}

func (t *CABIMissionTransport) Cancel(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProjectStatus(ctx, requestJSON, t.symbols.buildCancelInvocation, "C ABI mission cancel failed")
}

func (t *CABIMissionTransport) Events(ctx context.Context, requestJSON []byte) ([]byte, error) {
	var request MissionEventListRequest
	if err := json.Unmarshal(requestJSON, &request); err != nil {
		return nil, invalidProfilePayload(missionProfile, fmt.Sprintf("decode mission events request JSON: %v", err), err)
	}
	if err := validateMissionEventListRequest(request); err != nil {
		return nil, err
	}
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := t.callJSON(handle, t.symbols.buildEventsInvocation, requestJSON, "C ABI mission events failed")
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.invoke(handle, draftJSON, "C ABI mission events failed")
	if err != nil {
		return nil, err
	}
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, missionProfile)
	if err != nil {
		return nil, err
	}
	projectionInput, err := missionEventsProjectionInput(outputJSON, request)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.projectEvents, projectionInput, "C ABI mission events failed")
}

func (t *CABIMissionTransport) ProjectStatus(ctx context.Context, statusJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectStatus, statusJSON, "C ABI mission status projection failed")
}

func (t *CABIMissionTransport) ProjectEvents(ctx context.Context, eventsJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectEvents, eventsJSON, "C ABI mission events projection failed")
}

func (t *CABIMissionTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_mission_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiMissionLastErrorOrCode(symbols, code, "C ABI mission shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIMissionTransport) invokeAndProjectStatus(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, fallback string) ([]byte, error) {
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
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, missionProfile)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.projectStatus, outputJSON, fallback)
}

func missionEventsProjectionInput(outputJSON []byte, request MissionEventListRequest) ([]byte, error) {
	var result map[string]any
	if err := json.Unmarshal(outputJSON, &result); err != nil {
		return nil, invalidProfilePayload(missionProfile, fmt.Sprintf("decode mission events output JSON: %v", err), err)
	}
	if result == nil {
		return nil, invalidProfilePayload(missionProfile, "mission events output must be an object", nil)
	}
	projection := map[string]any{
		"mission_id":      request.MissionID,
		"cursor_sequence": request.CursorSequence,
		"result":          result,
	}
	return json.Marshal(projection)
}

func (t *CABIMissionTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABIMissionTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI mission transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI mission transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI mission transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIMissionTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_mission_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiMissionLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiMissionTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIMissionTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_mission_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiMissionLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiMissionTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIMissionSymbols(library unsafe.Pointer) (cabiMissionSymbols, error) {
	var symbols cabiMissionSymbols
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
		{"easynet_mission_build_run_eal_invocation", &symbols.buildRunEALInvocation},
		{"easynet_mission_build_run_file_invocation", &symbols.buildRunFileInvocation},
		{"easynet_mission_build_track_invocation", &symbols.buildTrackInvocation},
		{"easynet_mission_build_cancel_invocation", &symbols.buildCancelInvocation},
		{"easynet_mission_build_events_invocation", &symbols.buildEventsInvocation},
		{"easynet_mission_project_status", &symbols.projectStatus},
		{"easynet_mission_project_events", &symbols.projectEvents},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiMissionSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiMissionInit(symbols cabiMissionSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_mission_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_mission_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiMissionLastErrorOrCode(symbols, int32(code), "C ABI mission init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI mission init returned an invalid handle")
	}
	return handle, nil
}

func cabiMissionLastErrorOrCode(symbols cabiMissionSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_mission_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiMissionTakeCString(symbols.stringFree, out)
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

func cabiMissionTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_mission_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
