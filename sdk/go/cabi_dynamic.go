//go:build runtime_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

*/
import "C"

import (
	"fmt"
	"runtime"
	"unsafe"
)

const expectedCABIABIVersion uint32 = 7

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
		Code:      ErrTransport,
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
		return []string{"libeasynet_cli.dylib"}
	default:
		return []string{"libeasynet_cli.so"}
	}
}
