//go:build easynet_cabi && cgo && !windows

package easynet

/*
#include <stdint.h>
*/
import "C"

import "unsafe"

//export easynetGoStreamCallback
func easynetGoStreamCallback(userData unsafe.Pointer, chunkJSON *C.char) {
	if chunkJSON == nil {
		return
	}
	pushCABICallbackPayload(uintptr(userData), []byte(C.GoString(chunkJSON)))
}

//export easynetGoBidiCallback
func easynetGoBidiCallback(userData unsafe.Pointer, frameJSON *C.char) {
	if frameJSON == nil {
		return
	}
	pushCABICallbackPayload(uintptr(userData), []byte(C.GoString(frameJSON)))
}
