//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_admin_abi_version_fn)(void);
typedef int32_t (*easynet_admin_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_admin_string_free_fn)(char *s);
typedef int32_t (*easynet_admin_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_admin_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_admin_daemon_attach_fn)(const char *options_json, uint64_t *out_daemon_handle);
typedef int32_t (*easynet_admin_daemon_detach_fn)(uint64_t daemon_handle);
typedef int32_t (*easynet_admin_daemon_status_fn)(uint64_t daemon_handle, char **out_status_json);
typedef int32_t (*easynet_admin_invoke_fn)(uint64_t handle, const char *invocation_json, char **out_result_json);
typedef int32_t (*easynet_admin_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_admin_call_abi_version(void *fn) {
	return ((easynet_admin_abi_version_fn)fn)();
}

static int32_t easynet_admin_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_admin_last_error_json_fn)fn)(out_error_json);
}

static void easynet_admin_call_string_free(void *fn, char *s) {
	((easynet_admin_string_free_fn)fn)(s);
}

static int32_t easynet_admin_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_admin_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_admin_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_admin_shutdown_fn)fn)(handle);
}

static int32_t easynet_admin_call_daemon_attach(void *fn, const char *options_json, uint64_t *out_daemon_handle) {
	return ((easynet_admin_daemon_attach_fn)fn)(options_json, out_daemon_handle);
}

static int32_t easynet_admin_call_daemon_detach(void *fn, uint64_t daemon_handle) {
	return ((easynet_admin_daemon_detach_fn)fn)(daemon_handle);
}

static int32_t easynet_admin_call_daemon_status(void *fn, uint64_t daemon_handle, char **out_status_json) {
	return ((easynet_admin_daemon_status_fn)fn)(daemon_handle, out_status_json);
}

static int32_t easynet_admin_call_invoke(void *fn, uint64_t handle, const char *invocation_json, char **out_result_json) {
	return ((easynet_admin_invoke_fn)fn)(handle, invocation_json, out_result_json);
}

static int32_t easynet_admin_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_admin_json_fn)fn)(handle, request_json, out_json);
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

type cabiAdminSymbols struct {
	abiVersion                  unsafe.Pointer
	lastErrorJSON               unsafe.Pointer
	stringFree                  unsafe.Pointer
	init                        unsafe.Pointer
	shutdown                    unsafe.Pointer
	daemonAttach                unsafe.Pointer
	daemonDetach                unsafe.Pointer
	daemonStatus                unsafe.Pointer
	invocationInvoke            unsafe.Pointer
	buildAgentListInvocation    unsafe.Pointer
	buildAgentStartInvocation   unsafe.Pointer
	buildAgentStopInvocation    unsafe.Pointer
	buildAgentRefreshInvocation unsafe.Pointer
	buildSessionListInvocation  unsafe.Pointer
	projectGatewayStatus        unsafe.Pointer
	projectAgentRecords         unsafe.Pointer
	projectAgentLifecycleResult unsafe.Pointer
	projectDeviceSessionPage    unsafe.Pointer
}

// CABIAdminTransport is an optional Admin + Gateway profile transport over
// libeasynet_cli. It delegates exported admin carriers and projections to the
// Rust-owned daemon SDK contract while leaving unexported trust/pairing
// operations explicitly unsupported.
type CABIAdminTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiAdminSymbols
	handle  uint64
	daemon  uint64
	closed  bool
}

var _ AdminTransport = (*CABIAdminTransport)(nil)

// OpenCABIAdminTransport loads libeasynet_cli and opens an Admin + Gateway profile transport.
func OpenCABIAdminTransport(path string, controlPath string) (*CABIAdminTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIAdminSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_admin_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionIncompatible,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiAdminInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	daemon, err := cabiAdminAttach(symbols, controlPath)
	if err != nil {
		_ = cabiAdminShutdown(symbols, handle)
		C.dlclose(library)
		return nil, err
	}
	return &CABIAdminTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
		daemon:  daemon,
	}, nil
}

// NewCABIAdminClient creates an AdminClient over libeasynet_cli.
func NewCABIAdminClient(path string, controlPath string) (*AdminClient, *CABIAdminTransport, error) {
	transport, err := OpenCABIAdminTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewAdminClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIAdminTransport) BuildAgentListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildAgentListInvocation, requestJSON, "C ABI admin agent-list invocation build failed")
}

func (t *CABIAdminTransport) BuildAgentStartInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildAgentStartInvocation, requestJSON, "C ABI admin agent-start invocation build failed")
}

func (t *CABIAdminTransport) BuildAgentStopInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildAgentStopInvocation, requestJSON, "C ABI admin agent-stop invocation build failed")
}

func (t *CABIAdminTransport) BuildAgentRefreshInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildAgentRefreshInvocation, requestJSON, "C ABI admin agent-refresh invocation build failed")
}

func (t *CABIAdminTransport) BuildSessionListInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildSessionListInvocation, requestJSON, "C ABI admin session-list invocation build failed")
}

func (t *CABIAdminTransport) GatewayStatus(ctx context.Context, requestJSON []byte) ([]byte, error) {
	handle, daemon, err := t.requireOpenHandles(ctx)
	if err != nil {
		return nil, err
	}
	rawStatus, err := t.daemonStatus(daemon)
	if err != nil {
		return nil, err
	}
	projectionInput, err := adminGatewayStatusProjectionInput(rawStatus, requestJSON)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.projectGatewayStatus, projectionInput, "C ABI admin gateway status projection failed")
}

func (t *CABIAdminTransport) ListAgents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildAgentListInvocation, t.symbols.projectAgentRecords, "C ABI admin list agents failed")
}

func (t *CABIAdminTransport) AgentStart(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildAgentStartInvocation, t.symbols.projectAgentLifecycleResult, "C ABI admin agent start failed")
}

func (t *CABIAdminTransport) AgentStop(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildAgentStopInvocation, t.symbols.projectAgentLifecycleResult, "C ABI admin agent stop failed")
}

func (t *CABIAdminTransport) AgentRefresh(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildAgentRefreshInvocation, t.symbols.projectAgentLifecycleResult, "C ABI admin agent refresh failed")
}

func (t *CABIAdminTransport) ListDeviceSessions(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProject(ctx, requestJSON, t.symbols.buildSessionListInvocation, t.symbols.projectDeviceSessionPage, "C ABI admin list device sessions failed")
}

func (t *CABIAdminTransport) JoinHub(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin join hub carrier is not exported yet")
}

func (t *CABIAdminTransport) LeaveHub(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin leave hub carrier is not exported yet")
}

func (t *CABIAdminTransport) PairingPreflight(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin pairing preflight carrier is not exported yet")
}

func (t *CABIAdminTransport) ValidatePairing(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin validate pairing carrier is not exported yet")
}

func (t *CABIAdminTransport) VerifyDeviceCredential(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin verify device credential carrier is not exported yet")
}

func (t *CABIAdminTransport) CreatePairing(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin create pairing carrier is not exported yet")
}

func (t *CABIAdminTransport) RevokeDevice(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin revoke device carrier is not exported yet")
}

func (t *CABIAdminTransport) CreateDeviceSession(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin create device session carrier is not exported yet")
}

func (t *CABIAdminTransport) DeleteDeviceSession(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(adminGatewayProfile, "C ABI admin delete device session carrier is not exported yet")
}

func (t *CABIAdminTransport) ProjectGatewayStatus(ctx context.Context, statusJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectGatewayStatus, statusJSON, "C ABI admin gateway status projection failed")
}

func (t *CABIAdminTransport) ProjectAgentRecords(ctx context.Context, agentsJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectAgentRecords, agentsJSON, "C ABI admin agent records projection failed")
}

func (t *CABIAdminTransport) ProjectAgentLifecycleResult(ctx context.Context, resultJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectAgentLifecycleResult, resultJSON, "C ABI admin agent lifecycle projection failed")
}

func (t *CABIAdminTransport) ProjectDeviceSessionPage(ctx context.Context, sessionsJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectDeviceSessionPage, sessionsJSON, "C ABI admin device session projection failed")
}

func (t *CABIAdminTransport) Close(ctx context.Context) error {
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
	daemon := t.daemon
	t.daemon = 0
	library := t.library
	t.library = nil
	symbols := t.symbols
	t.mu.Unlock()

	var first error
	if daemon != 0 {
		code := int32(C.easynet_admin_call_daemon_detach(symbols.daemonDetach, C.uint64_t(daemon)))
		if code != 0 {
			first = cabiAdminLastErrorOrCode(symbols, code, "C ABI admin daemon detach failed")
		}
	}
	if handle != 0 {
		if err := cabiAdminShutdown(symbols, handle); err != nil && first == nil {
			first = err
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIAdminTransport) invokeAndProject(ctx context.Context, requestJSON []byte, buildSymbol unsafe.Pointer, projectSymbol unsafe.Pointer, fallback string) ([]byte, error) {
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
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, adminGatewayProfile)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, projectSymbol, outputJSON, fallback)
}

func (t *CABIAdminTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABIAdminTransport) requireOpen(ctx context.Context) (uint64, error) {
	handle, _, err := t.requireOpenHandles(ctx)
	return handle, err
}

func (t *CABIAdminTransport) requireOpenHandles(ctx context.Context) (uint64, uint64, error) {
	if ctx == nil {
		return 0, 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, 0, invalidRuntimeClient("C ABI admin transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, 0, invalidRuntimeClient("C ABI admin transport is closed")
	}
	if t.handle == 0 {
		return 0, 0, invalidCABIHandle("C ABI admin transport handle is invalid")
	}
	if t.daemon == 0 {
		return 0, 0, invalidCABIHandle("C ABI admin daemon handle is invalid")
	}
	return t.handle, t.daemon, nil
}

func (t *CABIAdminTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_admin_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiAdminLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiAdminTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIAdminTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_admin_call_invoke(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiAdminLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiAdminTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIAdminTransport) daemonStatus(daemon uint64) ([]byte, error) {
	var out *C.char
	code := int32(C.easynet_admin_call_daemon_status(t.symbols.daemonStatus, C.uint64_t(daemon), &out))
	if code != 0 {
		return nil, cabiAdminLastErrorOrCode(t.symbols, code, "C ABI admin daemon status failed")
	}
	return cabiAdminTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIAdminSymbols(library unsafe.Pointer) (cabiAdminSymbols, error) {
	var symbols cabiAdminSymbols
	bindings := []struct {
		name string
		out  *unsafe.Pointer
	}{
		{"easynet_abi_version", &symbols.abiVersion},
		{"easynet_last_error_json", &symbols.lastErrorJSON},
		{"easynet_string_free", &symbols.stringFree},
		{"easynet_init", &symbols.init},
		{"easynet_shutdown", &symbols.shutdown},
		{"easynet_daemon_attach", &symbols.daemonAttach},
		{"easynet_daemon_detach", &symbols.daemonDetach},
		{"easynet_daemon_status", &symbols.daemonStatus},
		{"easynet_invocation_invoke", &symbols.invocationInvoke},
		{"easynet_admin_build_agent_list_invocation", &symbols.buildAgentListInvocation},
		{"easynet_admin_build_agent_start_invocation", &symbols.buildAgentStartInvocation},
		{"easynet_admin_build_agent_stop_invocation", &symbols.buildAgentStopInvocation},
		{"easynet_admin_build_agent_refresh_invocation", &symbols.buildAgentRefreshInvocation},
		{"easynet_admin_build_session_list_invocation", &symbols.buildSessionListInvocation},
		{"easynet_admin_project_gateway_status", &symbols.projectGatewayStatus},
		{"easynet_admin_project_agent_records", &symbols.projectAgentRecords},
		{"easynet_admin_project_agent_lifecycle_result", &symbols.projectAgentLifecycleResult},
		{"easynet_admin_project_device_session_page", &symbols.projectDeviceSessionPage},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiAdminSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiAdminInit(symbols cabiAdminSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_admin_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_admin_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiAdminLastErrorOrCode(symbols, int32(code), "C ABI admin init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI admin init returned an invalid handle")
	}
	return handle, nil
}

func cabiAdminAttach(symbols cabiAdminSymbols, controlPath string) (uint64, error) {
	options := map[string]any{}
	if controlPath != "" {
		options["control_path"] = controlPath
	}
	optionsJSON, err := json.Marshal(options)
	if err != nil {
		return 0, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("encode daemon attach options: %v", err), err)
	}
	var out C.uint64_t
	code := C.int32_t(cabiWithCString(optionsJSON, func(cOptions *C.char) C.int32_t {
		return C.easynet_admin_call_daemon_attach(symbols.daemonAttach, cOptions, &out)
	}))
	if int32(code) != 0 {
		return 0, cabiAdminLastErrorOrCode(symbols, int32(code), "C ABI admin daemon attach failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI admin daemon attach returned an invalid handle")
	}
	return handle, nil
}

func cabiAdminShutdown(symbols cabiAdminSymbols, handle uint64) error {
	code := int32(C.easynet_admin_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
	if code != 0 {
		return cabiAdminLastErrorOrCode(symbols, code, "C ABI admin shutdown failed")
	}
	return nil
}

func adminGatewayStatusProjectionInput(statusJSON []byte, requestJSON []byte) ([]byte, error) {
	var status map[string]any
	if err := json.Unmarshal(statusJSON, &status); err != nil {
		return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("decode daemon status JSON: %v", err), err)
	}
	var request map[string]any
	if len(requestJSON) > 0 {
		if err := json.Unmarshal(requestJSON, &request); err != nil {
			return nil, invalidProfilePayload(adminGatewayProfile, fmt.Sprintf("decode gateway status request JSON: %v", err), err)
		}
	}
	projection := map[string]any{
		"runtime_status": daemonStateFromCABI(status),
		"daemon":         status,
	}
	if value, ok := request["require_public_listener"].(bool); ok {
		projection["require_public_listener"] = value
	}
	if metadata, ok := request["metadata"].(map[string]any); ok {
		projection["metadata"] = metadata
	}
	return json.Marshal(projection)
}

func cabiAdminLastErrorOrCode(symbols cabiAdminSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_admin_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiAdminTakeCString(symbols.stringFree, out)
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

func cabiAdminTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_admin_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
