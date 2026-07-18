//go:build easynet_cabi && cgo && !windows

package easynet

/*
#include <stdint.h>
*/
import "C"

import "unsafe"

//export easynetGoStreamCallback
func easynetGoStreamCallback(userData unsafe.Pointer, chunkJSON *C.char) {
	if userData == nil || chunkJSON == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	pushCABICallbackPayload(token, []byte(C.GoString(chunkJSON)))
}

//export easynetGoBidiCallback
func easynetGoBidiCallback(userData unsafe.Pointer, frameJSON *C.char) {
	if userData == nil || frameJSON == nil {
		return
	}
	token := uintptr(*(*C.uintptr_t)(userData))
	pushCABICallbackPayload(token, []byte(C.GoString(frameJSON)))
}
