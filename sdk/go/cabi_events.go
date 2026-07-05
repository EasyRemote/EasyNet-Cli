//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_events_abi_version_fn)(void);
typedef int32_t (*easynet_events_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_events_string_free_fn)(char *s);
typedef int32_t (*easynet_events_init_fn)(const char *control_path, uint64_t *out_handle);
typedef int32_t (*easynet_events_shutdown_fn)(uint64_t handle);
typedef int32_t (*easynet_events_json_fn)(uint64_t handle, const char *request_json, char **out_json);

static uint32_t easynet_events_call_abi_version(void *fn) {
	return ((easynet_events_abi_version_fn)fn)();
}

static int32_t easynet_events_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_events_last_error_json_fn)fn)(out_error_json);
}

static void easynet_events_call_string_free(void *fn, char *s) {
	((easynet_events_string_free_fn)fn)(s);
}

static int32_t easynet_events_call_init(void *fn, const char *control_path, uint64_t *out_handle) {
	return ((easynet_events_init_fn)fn)(control_path, out_handle);
}

static int32_t easynet_events_call_shutdown(void *fn, uint64_t handle) {
	return ((easynet_events_shutdown_fn)fn)(handle);
}

static int32_t easynet_events_call_json(void *fn, uint64_t handle, const char *request_json, char **out_json) {
	return ((easynet_events_json_fn)fn)(handle, request_json, out_json);
}
*/
import "C"

import (
	"context"
	"fmt"
	"sync"
	"unsafe"
)

type cabiEventsSymbols struct {
	abiVersion                            unsafe.Pointer
	lastErrorJSON                         unsafe.Pointer
	stringFree                            unsafe.Pointer
	init                                  unsafe.Pointer
	shutdown                              unsafe.Pointer
	invocationInvoke                      unsafe.Pointer
	buildDirectorySubscriptionInvocation  unsafe.Pointer
	buildDeviceSubscriptionInvocation     unsafe.Pointer
	buildSessionSubscriptionInvocation    unsafe.Pointer
	buildInvocationSubscriptionInvocation unsafe.Pointer
	buildDeviceEventHistoryInvocation     unsafe.Pointer
	projectDeviceEventPage                unsafe.Pointer
	projectDirectoryEvent                 unsafe.Pointer
	projectTerminal                       unsafe.Pointer
	projectDropReport                     unsafe.Pointer
}

// CABIEventsTransport is an optional Events profile projection over
// libeasynet_cli. Runtime Core still owns stream opening; this adapter exposes
// the daemon-owned event carrier and frame projection symbols exported by the C
// ABI.
type CABIEventsTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiEventsSymbols
	handle  uint64
	closed  bool
}

var _ EventTransport = (*CABIEventsTransport)(nil)

// OpenCABIEventsTransport loads libeasynet_cli and opens an Events profile transport.
func OpenCABIEventsTransport(path string, controlPath string) (*CABIEventsTransport, error) {
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIEventsSymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_events_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	handle, err := cabiEventsInit(symbols, controlPath)
	if err != nil {
		C.dlclose(library)
		return nil, err
	}
	return &CABIEventsTransport{
		library: library,
		symbols: symbols,
		handle:  handle,
	}, nil
}

// NewCABIEventClient creates an EventClient over libeasynet_cli.
func NewCABIEventClient(path string, controlPath string) (*EventClient, *CABIEventsTransport, error) {
	transport, err := OpenCABIEventsTransport(path, controlPath)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewEventClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIEventsTransport) BuildDirectorySubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildDirectorySubscriptionInvocation, requestJSON, "C ABI events directory subscription invocation build failed")
}

func (t *CABIEventsTransport) BuildDeviceSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildDeviceSubscriptionInvocation, requestJSON, "C ABI events device subscription invocation build failed")
}

func (t *CABIEventsTransport) BuildSessionSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildSessionSubscriptionInvocation, requestJSON, "C ABI events session subscription invocation build failed")
}

func (t *CABIEventsTransport) BuildInvocationSubscriptionInvocation(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.buildInvocationSubscriptionInvocation, requestJSON, "C ABI events invocation subscription invocation build failed")
}

func (t *CABIEventsTransport) SubscribeDirectory(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "C ABI events subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *CABIEventsTransport) SubscribeDevices(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "C ABI events subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *CABIEventsTransport) SubscribeSessions(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "C ABI events subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *CABIEventsTransport) SubscribeInvocations(context.Context, []byte) ([]byte, error) {
	return nil, sdkProfileNotImplemented(eventsProfile, "C ABI events subscriptions are opened through RuntimeClient.InvokeStream")
}

func (t *CABIEventsTransport) ListDeviceEvents(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.invokeAndProjectDeviceEvents(ctx, requestJSON, "C ABI events device event history failed")
}

func (t *CABIEventsTransport) ProjectDirectoryEvent(ctx context.Context, eventJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectDirectoryEvent, eventJSON, "C ABI events directory event projection failed")
}

func (t *CABIEventsTransport) ProjectDropReport(ctx context.Context, dropJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectDropReport, dropJSON, "C ABI events drop-report projection failed")
}

func (t *CABIEventsTransport) ProjectTerminal(ctx context.Context, terminalJSON []byte) ([]byte, error) {
	return t.callJSONWithOpenHandle(ctx, t.symbols.projectTerminal, terminalJSON, "C ABI events terminal projection failed")
}

func (t *CABIEventsTransport) Close(ctx context.Context) error {
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
		code := int32(C.easynet_events_call_shutdown(symbols.shutdown, C.uint64_t(handle)))
		if code != 0 {
			first = cabiEventsLastErrorOrCode(symbols, code, "C ABI events shutdown failed")
		}
	}
	if library != nil {
		C.dlclose(library)
	}
	return first
}

func (t *CABIEventsTransport) invokeAndProjectDeviceEvents(ctx context.Context, requestJSON []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	draftJSON, err := t.callJSON(handle, t.symbols.buildDeviceEventHistoryInvocation, requestJSON, fallback)
	if err != nil {
		return nil, err
	}
	resultJSON, err := t.invoke(handle, draftJSON, fallback)
	if err != nil {
		return nil, err
	}
	outputJSON, err := outputJSONFromProfileInvocationResult(resultJSON, eventsProfile)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, t.symbols.projectDeviceEventPage, outputJSON, fallback)
}

func (t *CABIEventsTransport) callJSONWithOpenHandle(ctx context.Context, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	handle, err := t.requireOpen(ctx)
	if err != nil {
		return nil, err
	}
	return t.callJSON(handle, symbol, payload, fallback)
}

func (t *CABIEventsTransport) requireOpen(ctx context.Context) (uint64, error) {
	if ctx == nil {
		return 0, invalidRuntimeClient("context is required")
	}
	if t == nil {
		return 0, invalidRuntimeClient("C ABI events transport is not initialized")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return 0, invalidRuntimeClient("C ABI events transport is closed")
	}
	if t.handle == 0 {
		return 0, invalidCABIHandle("C ABI events transport handle is invalid")
	}
	return t.handle, nil
}

func (t *CABIEventsTransport) callJSON(handle uint64, symbol unsafe.Pointer, payload []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_events_call_json(symbol, C.uint64_t(handle), cPayload, &out)
	}))
	if code != 0 {
		return nil, cabiEventsLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiEventsTakeCString(t.symbols.stringFree, out), nil
}

func (t *CABIEventsTransport) invoke(handle uint64, draftJSON []byte, fallback string) ([]byte, error) {
	var out *C.char
	code := int32(cabiWithCString(draftJSON, func(cDraft *C.char) C.int32_t {
		return C.easynet_events_call_json(t.symbols.invocationInvoke, C.uint64_t(handle), cDraft, &out)
	}))
	if code != 0 {
		return nil, cabiEventsLastErrorOrCode(t.symbols, code, fallback)
	}
	return cabiEventsTakeCString(t.symbols.stringFree, out), nil
}

func bindCABIEventsSymbols(library unsafe.Pointer) (cabiEventsSymbols, error) {
	var symbols cabiEventsSymbols
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
		{"easynet_events_build_directory_subscription_invocation", &symbols.buildDirectorySubscriptionInvocation},
		{"easynet_events_build_device_subscription_invocation", &symbols.buildDeviceSubscriptionInvocation},
		{"easynet_events_build_session_subscription_invocation", &symbols.buildSessionSubscriptionInvocation},
		{"easynet_events_build_invocation_subscription_invocation", &symbols.buildInvocationSubscriptionInvocation},
		{"easynet_events_build_device_event_history_invocation", &symbols.buildDeviceEventHistoryInvocation},
		{"easynet_events_project_device_event_page", &symbols.projectDeviceEventPage},
		{"easynet_events_project_directory_event", &symbols.projectDirectoryEvent},
		{"easynet_events_project_terminal", &symbols.projectTerminal},
		{"easynet_events_project_drop_report", &symbols.projectDropReport},
	}
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiEventsSymbols{}, err
		}
		*binding.out = ptr
	}
	return symbols, nil
}

func cabiEventsInit(symbols cabiEventsSymbols, controlPath string) (uint64, error) {
	var out C.uint64_t
	var code C.int32_t
	if controlPath == "" {
		code = C.easynet_events_call_init(symbols.init, nil, &out)
	} else {
		cControlPath := C.CString(controlPath)
		defer C.free(unsafe.Pointer(cControlPath))
		code = C.easynet_events_call_init(symbols.init, cControlPath, &out)
	}
	if int32(code) != 0 {
		return 0, cabiEventsLastErrorOrCode(symbols, int32(code), "C ABI events init failed")
	}
	handle := uint64(out)
	if handle == 0 {
		return 0, invalidCABIHandle("C ABI events init returned an invalid handle")
	}
	return handle, nil
}

func cabiEventsLastErrorOrCode(symbols cabiEventsSymbols, code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_events_call_last_error_json(symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		raw := cabiEventsTakeCString(symbols.stringFree, out)
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

func cabiEventsTakeCString(stringFree unsafe.Pointer, value *C.char) []byte {
	if value == nil {
		return []byte{}
	}
	defer C.easynet_events_call_string_free(stringFree, value)
	return []byte(C.GoString(value))
}
