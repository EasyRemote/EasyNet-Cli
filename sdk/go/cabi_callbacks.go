//go:build runtime_cabi && cgo && !windows

package easynet

/*
#include <stdint.h>
#include <stdlib.h>
typedef struct runtime_bytes_view_v8 {
	const uint8_t *data;
	size_t len;
} runtime_bytes_view_v8;
typedef struct runtime_invocation_stream_frame_v8 {
	uint32_t struct_size;
	uint16_t abi_version;
	uint8_t kind;
	uint8_t state;
	uint32_t flags;
	uint64_t sequence;
	uint64_t elapsed_ms;
	runtime_bytes_view_v8 payload_content_type;
	runtime_bytes_view_v8 payload;
	runtime_bytes_view_v8 admission_receipt_json;
	runtime_bytes_view_v8 terminal_receipt_json;
	runtime_bytes_view_v8 error_json;
} runtime_invocation_stream_frame_v8;
typedef struct runtime_buffer_lease_v9 {
	uint64_t lease_id;
	const uint8_t *data;
	size_t len;
} runtime_buffer_lease_v9;
typedef struct runtime_invocation_stream_frame_v9 {
	uint32_t struct_size;
	uint16_t abi_version;
	uint8_t kind;
	uint8_t state;
	uint32_t flags;
	uint64_t sequence;
	uint64_t elapsed_ms;
	runtime_bytes_view_v8 payload_content_type;
	runtime_buffer_lease_v9 payload;
	runtime_bytes_view_v8 admission_receipt_json;
	runtime_bytes_view_v8 terminal_receipt_json;
	runtime_bytes_view_v8 error_json;
} runtime_invocation_stream_frame_v9;
*/
import "C"

import (
	"fmt"
	"unsafe"
)

const (
	streamV8ABIVersion                     = 8
	streamV9ABIVersion                     = 9
	streamV8FlagTerminal                   = 1 << 0
	streamV8FlagTransportTerminal          = 1 << 1
	streamV8FlagHasPayload                 = 1 << 2
	streamV8FlagHasContentType             = 1 << 3
	streamV8FlagHasAdmissionReceipt        = 1 << 4
	streamV8FlagHasTerminalReceipt         = 1 << 5
	streamV8FlagHasError                   = 1 << 6
	streamV8KnownFlags                     = (1 << 7) - 1
	maxStreamV8PayloadBytes         uint64 = 256 * 1024 * 1024
	maxStreamV8SidecarBytes         uint64 = 16 * 1024 * 1024
)

//export easynetGoStreamCallback
func easynetGoStreamCallback(userData unsafe.Pointer, chunkJSON *C.char) {
	if userData == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	if chunkJSON == nil {
		closeCABICallbackInbox(token)
		return
	}
	pushCABICallbackPayload(token, []byte(C.GoString(chunkJSON)))
}

//export easynetGoStreamV9Callback
func easynetGoStreamV9Callback(userData unsafe.Pointer, frame *C.runtime_invocation_stream_frame_v9) {
	if userData == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	if frame == nil {
		closeCABICallbackInbox(token)
		return
	}
	if uint32(frame.struct_size) < uint32(C.sizeof_runtime_invocation_stream_frame_v9) || uint16(frame.abi_version) != streamV9ABIVersion {
		failCABICallbackInbox(token, invalidRuntimePayload("v9 leased frame has an incompatible layout", nil))
		return
	}
	lease, err := newCABICallbackLeasedPayload(
		token,
		uint64(frame.payload.lease_id),
		unsafe.Pointer(frame.payload.data),
		uint64(frame.payload.len),
	)
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	releaseOnFailure := func(err error) {
		if lease != nil {
			_ = lease.Release()
		}
		failCABICallbackInbox(token, err)
	}
	flags := uint32(frame.flags)
	if frame.sequence == 0 || flags&^uint32(streamV8KnownFlags) != 0 {
		releaseOnFailure(invalidRuntimePayload("v9 leased frame has an invalid sequence or flags", nil))
		return
	}
	kind, ok := streamV8KindName(uint8(frame.kind))
	if !ok {
		releaseOnFailure(invalidRuntimePayload("v9 leased frame has an unknown kind", nil))
		return
	}
	state, ok := streamV8StateName(uint8(frame.state))
	if !ok {
		releaseOnFailure(invalidRuntimePayload("v9 leased frame has an unknown state", nil))
		return
	}
	contentType, err := copyStreamV8View(frame.payload_content_type, 4096, "payload_content_type")
	if err != nil {
		releaseOnFailure(err)
		return
	}
	admissionReceipt, err := copyStreamV8View(frame.admission_receipt_json, maxStreamV8SidecarBytes, "admission_receipt_json")
	if err != nil {
		releaseOnFailure(err)
		return
	}
	terminalReceipt, err := copyStreamV8View(frame.terminal_receipt_json, maxStreamV8SidecarBytes, "terminal_receipt_json")
	if err != nil {
		releaseOnFailure(err)
		return
	}
	errorJSON, err := copyStreamV8View(frame.error_json, maxStreamV8SidecarBytes, "error_json")
	if err != nil {
		releaseOnFailure(err)
		return
	}
	for _, presence := range []struct {
		flag    uint32
		present bool
		name    string
	}{
		{streamV8FlagHasContentType, len(contentType) != 0, "content type"},
		{streamV8FlagHasPayload, lease != nil, "payload"},
		{streamV8FlagHasAdmissionReceipt, len(admissionReceipt) != 0, "admission receipt"},
		{streamV8FlagHasTerminalReceipt, len(terminalReceipt) != 0, "terminal receipt"},
		{streamV8FlagHasError, len(errorJSON) != 0, "error"},
	} {
		if (flags&presence.flag != 0) != presence.present {
			releaseOnFailure(invalidRuntimePayload("v9 leased frame "+presence.name+" presence flag is inconsistent", nil))
			return
		}
	}
	pushCABICallbackLeasedFrame(token, leasedStreamPacket{
		sequence:             uint64(frame.sequence),
		kind:                 kind,
		state:                state,
		terminal:             flags&streamV8FlagTerminal != 0,
		transportTerminal:    flags&streamV8FlagTransportTerminal != 0,
		elapsedMS:            uint64(frame.elapsed_ms),
		payloadContentType:   string(contentType),
		payload:              lease,
		admissionReceiptJSON: admissionReceipt,
		terminalReceiptJSON:  terminalReceipt,
		errorJSON:            errorJSON,
	})
}

//export easynetGoStreamV8Callback
func easynetGoStreamV8Callback(userData unsafe.Pointer, frame *C.runtime_invocation_stream_frame_v8) {
	if userData == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	if frame == nil {
		closeCABICallbackInbox(token)
		return
	}
	if uint32(frame.struct_size) < uint32(C.sizeof_runtime_invocation_stream_frame_v8) || uint16(frame.abi_version) != streamV8ABIVersion {
		failCABICallbackInbox(token, invalidRuntimePayload("v8 binary frame has an incompatible layout", nil))
		return
	}
	flags := uint32(frame.flags)
	if frame.sequence == 0 || flags&^uint32(streamV8KnownFlags) != 0 {
		failCABICallbackInbox(token, invalidRuntimePayload("v8 binary frame has an invalid sequence or flags", nil))
		return
	}
	kind, ok := streamV8KindName(uint8(frame.kind))
	if !ok {
		failCABICallbackInbox(token, invalidRuntimePayload("v8 binary frame has an unknown kind", nil))
		return
	}
	state, ok := streamV8StateName(uint8(frame.state))
	if !ok {
		failCABICallbackInbox(token, invalidRuntimePayload("v8 binary frame has an unknown state", nil))
		return
	}
	contentType, err := copyStreamV8View(frame.payload_content_type, 4096, "payload_content_type")
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	payload, err := copyStreamV8View(frame.payload, maxStreamV8PayloadBytes, "payload")
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	admissionReceipt, err := copyStreamV8View(frame.admission_receipt_json, maxStreamV8SidecarBytes, "admission_receipt_json")
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	terminalReceipt, err := copyStreamV8View(frame.terminal_receipt_json, maxStreamV8SidecarBytes, "terminal_receipt_json")
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	errorJSON, err := copyStreamV8View(frame.error_json, maxStreamV8SidecarBytes, "error_json")
	if err != nil {
		failCABICallbackInbox(token, err)
		return
	}
	for _, presence := range []struct {
		flag  uint32
		bytes []byte
		name  string
	}{
		{streamV8FlagHasContentType, contentType, "content type"},
		{streamV8FlagHasPayload, payload, "payload"},
		{streamV8FlagHasAdmissionReceipt, admissionReceipt, "admission receipt"},
		{streamV8FlagHasTerminalReceipt, terminalReceipt, "terminal receipt"},
		{streamV8FlagHasError, errorJSON, "error"},
	} {
		if (flags&presence.flag != 0) != (len(presence.bytes) != 0) {
			failCABICallbackInbox(token, invalidRuntimePayload("v8 binary frame "+presence.name+" presence flag is inconsistent", nil))
			return
		}
	}
	pushCABICallbackBinaryFrame(token, rawStreamPacket{
		sequence:             uint64(frame.sequence),
		kind:                 kind,
		state:                state,
		terminal:             flags&streamV8FlagTerminal != 0,
		transportTerminal:    flags&streamV8FlagTransportTerminal != 0,
		elapsedMS:            uint64(frame.elapsed_ms),
		payloadContentType:   string(contentType),
		payload:              payload,
		admissionReceiptJSON: admissionReceipt,
		terminalReceiptJSON:  terminalReceipt,
		errorJSON:            errorJSON,
	})
}

func copyStreamV8View(view C.runtime_bytes_view_v8, maximum uint64, field string) ([]byte, error) {
	length := uint64(view.len)
	if length > maximum || length > uint64(^uint32(0)>>1) {
		return nil, invalidRuntimePayload(fmt.Sprintf("v8 binary frame %s exceeds its copy bound", field), nil)
	}
	if length == 0 {
		if view.data != nil {
			return nil, invalidRuntimePayload("v8 binary frame empty "+field+" must use a null pointer", nil)
		}
		return nil, nil
	}
	if view.data == nil {
		return nil, invalidRuntimePayload("v8 binary frame "+field+" pointer is null", nil)
	}
	return C.GoBytes(unsafe.Pointer(view.data), C.int(length)), nil
}

func streamV8KindName(kind uint8) (string, bool) {
	names := map[uint8]string{1: "data", 2: "terminal", 3: "error", 4: "cancelled", 5: "timeout", 6: "receipt_verification_error"}
	name, ok := names[kind]
	return name, ok
}

func streamV8StateName(state uint8) (string, bool) {
	names := map[uint8]string{1: "Accepted", 2: "Admitted", 3: "Dispatched", 4: "Running", 5: "Completed", 6: "Failed", 7: "TimedOut", 8: "Cancelled"}
	name, ok := names[state]
	return name, ok
}

//export easynetGoBidiCallback
func easynetGoBidiCallback(userData unsafe.Pointer, frameJSON *C.char) {
	if userData == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	if frameJSON == nil {
		closeCABICallbackInbox(token)
		return
	}
	pushCABICallbackPayload(token, []byte(C.GoString(frameJSON)))
}
