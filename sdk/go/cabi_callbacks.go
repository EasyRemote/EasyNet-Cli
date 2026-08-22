//go:build runtime_cabi && cgo && !windows

package easynet

/*
#include <stdint.h>
#include <stdlib.h>
*/
import "C"

import "unsafe"

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

//export easynetGoStreamV8Callback
func easynetGoStreamV8Callback(userData unsafe.Pointer, metadataJSON *C.char, payload *C.uint8_t, payloadLen C.size_t) {
	if userData == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	if metadataJSON == nil {
		closeCABICallbackInbox(token)
		return
	}
	var payloadBytes []byte
	if payload != nil && payloadLen != 0 {
		payloadBytes = C.GoBytes(unsafe.Pointer(payload), C.int(payloadLen))
	}
	pushCABICallbackRawPayload(token, []byte(C.GoString(metadataJSON)), payloadBytes)
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
