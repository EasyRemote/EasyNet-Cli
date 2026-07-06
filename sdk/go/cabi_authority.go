//go:build easynet_cabi && cgo && !windows

package easynet

/*
#cgo linux LDFLAGS: -ldl
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>

typedef uint32_t (*easynet_authority_abi_version_fn)(void);
typedef int32_t (*easynet_authority_last_error_json_fn)(char **out_error_json);
typedef void (*easynet_authority_string_free_fn)(char *s);
typedef int32_t (*easynet_authority_prepare_fn)(const char *request_json, char **out_material_json);
typedef int32_t (*easynet_authority_materialize_fn)(const char *request_json, const char *signature_json, char **out_metadata_json);

static uint32_t easynet_authority_call_abi_version(void *fn) {
	return ((easynet_authority_abi_version_fn)fn)();
}

static int32_t easynet_authority_call_last_error_json(void *fn, char **out_error_json) {
	return ((easynet_authority_last_error_json_fn)fn)(out_error_json);
}

static void easynet_authority_call_string_free(void *fn, char *s) {
	((easynet_authority_string_free_fn)fn)(s);
}

static int32_t easynet_authority_call_prepare(void *fn, const char *request_json, char **out_material_json) {
	return ((easynet_authority_prepare_fn)fn)(request_json, out_material_json);
}

static int32_t easynet_authority_call_materialize(void *fn, const char *request_json, const char *signature_json, char **out_metadata_json) {
	return ((easynet_authority_materialize_fn)fn)(request_json, signature_json, out_metadata_json);
}
*/
import "C"

import (
	"context"
	"fmt"
	"sync"
	"unsafe"
)

type cabiAuthoritySymbols struct {
	abiVersion            unsafe.Pointer
	lastErrorJSON         unsafe.Pointer
	stringFree            unsafe.Pointer
	prepareDelegation     unsafe.Pointer
	materializeDelegation unsafe.Pointer
	prepareSession        unsafe.Pointer
	materializeSession    unsafe.Pointer
}

// CABIAuthorityTransport is an optional AuthorityTransport backed by the
// daemon SDK C ABI authority core and an explicit external signer.
type CABIAuthorityTransport struct {
	mu      sync.Mutex
	library unsafe.Pointer
	symbols cabiAuthoritySymbols
	signer  AuthoritySignatureProvider
	closed  bool
}

var _ AuthorityTransport = (*CABIAuthorityTransport)(nil)

// OpenCABIAuthorityTransport loads libeasynet_cli authority symbols. The
// signer owns key access; this transport only prepares canonical material and
// materializes signed metadata through the runtime core.
func OpenCABIAuthorityTransport(path string, signer AuthoritySignatureProvider) (*CABIAuthorityTransport, error) {
	if signer == nil {
		return nil, invalidProfileClient(authorityProfile, "authority signature provider is required")
	}
	library, resolved, err := openCABIDynamicLibrary(path)
	if err != nil {
		return nil, err
	}
	symbols, err := bindCABIAuthoritySymbols(library)
	if err != nil {
		C.dlclose(library)
		return nil, fmt.Errorf("bind %s: %w", resolved, err)
	}
	if actual := C.easynet_authority_call_abi_version(symbols.abiVersion); uint32(actual) != expectedCABIABIVersion {
		C.dlclose(library)
		return nil, &SDKError{
			Code:      ErrVersionMismatch,
			Stage:     "cabi",
			Retry:     RetryNever,
			Retryable: false,
			Message:   fmt.Sprintf("libeasynet_cli ABI version %d does not match expected %d", actual, expectedCABIABIVersion),
		}
	}
	return &CABIAuthorityTransport{
		library: library,
		symbols: symbols,
		signer:  signer,
	}, nil
}

// NewCABIAuthorityClient creates an AuthorityClient over libeasynet_cli.
func NewCABIAuthorityClient(path string, signer AuthoritySignatureProvider) (*AuthorityClient, *CABIAuthorityTransport, error) {
	transport, err := OpenCABIAuthorityTransport(path, signer)
	if err != nil {
		return nil, nil, err
	}
	client, err := NewAuthorityClient(transport)
	if err != nil {
		_ = transport.Close(context.Background())
		return nil, nil, err
	}
	return client, transport, nil
}

func (t *CABIAuthorityTransport) MintDelegationProof(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.mint(ctx, requestJSON, cabiAuthorityMintOptions{
		prepare:     t.symbols.prepareDelegation,
		materialize: t.symbols.materializeDelegation,
		metadataKey: DelegationMetadataKey,
		kind:        AuthorityKindDelegation,
	})
}

func (t *CABIAuthorityTransport) MintSessionAuthority(ctx context.Context, requestJSON []byte) ([]byte, error) {
	return t.mint(ctx, requestJSON, cabiAuthorityMintOptions{
		prepare:     t.symbols.prepareSession,
		materialize: t.symbols.materializeSession,
		metadataKey: SessionAuthorityMetadataKey,
		kind:        AuthorityKindSessionAuthority,
	})
}

func (t *CABIAuthorityTransport) Close(ctx context.Context) error {
	if ctx == nil {
		return invalidProfileClient(authorityProfile, "context is required")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return nil
	}
	t.closed = true
	if t.library != nil {
		C.dlclose(t.library)
		t.library = nil
	}
	return nil
}

type cabiAuthorityMintOptions struct {
	prepare     unsafe.Pointer
	materialize unsafe.Pointer
	metadataKey string
	kind        AuthorityKind
}

func (t *CABIAuthorityTransport) mint(ctx context.Context, requestJSON []byte, opts cabiAuthorityMintOptions) ([]byte, error) {
	if ctx == nil {
		return nil, invalidProfileClient(authorityProfile, "context is required")
	}
	if len(requestJSON) == 0 {
		return nil, invalidProfilePayload(authorityProfile, "authority request JSON is required", nil)
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	materialJSON, err := t.callAuthorityPrepare(opts.prepare, requestJSON)
	if err != nil {
		return nil, err
	}
	material, err := newAuthoritySigningMaterial(materialJSON, opts.metadataKey, opts.kind)
	if err != nil {
		return nil, err
	}
	signature, err := t.signer.SignAuthority(ctx, material)
	if err != nil {
		return nil, transportProfileError(authorityProfile, "authority signing failed", err)
	}
	signatureJSON, err := authoritySignatureJSON(signature)
	if err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	return t.callAuthorityMaterialize(opts.materialize, requestJSON, signatureJSON)
}

func (t *CABIAuthorityTransport) callAuthorityPrepare(symbol unsafe.Pointer, payload []byte) ([]byte, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return nil, invalidProfileClient(authorityProfile, "C ABI authority transport is closed")
	}
	var out *C.char
	code := int32(cabiWithCString(payload, func(cPayload *C.char) C.int32_t {
		return C.easynet_authority_call_prepare(symbol, cPayload, &out)
	}))
	return t.cabiOutput(out, code, "C ABI authority prepare failed")
}

func (t *CABIAuthorityTransport) callAuthorityMaterialize(symbol unsafe.Pointer, request []byte, signature []byte) ([]byte, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return nil, invalidProfileClient(authorityProfile, "C ABI authority transport is closed")
	}
	var out *C.char
	code := int32(cabiWithCString(request, func(cRequest *C.char) C.int32_t {
		return cabiWithCString(signature, func(cSignature *C.char) C.int32_t {
			return C.easynet_authority_call_materialize(symbol, cRequest, cSignature, &out)
		})
	}))
	return t.cabiOutput(out, code, "C ABI authority materialize failed")
}

func (t *CABIAuthorityTransport) cabiOutput(out *C.char, code int32, fallback string) ([]byte, error) {
	if code != 0 {
		return nil, t.lastErrorOrCode(code, fallback)
	}
	if out == nil {
		return []byte{}, nil
	}
	defer C.easynet_authority_call_string_free(t.symbols.stringFree, out)
	return []byte(C.GoString(out)), nil
}

func (t *CABIAuthorityTransport) lastErrorOrCode(code int32, fallback string) error {
	var out *C.char
	errCode := int32(C.easynet_authority_call_last_error_json(t.symbols.lastErrorJSON, &out))
	if errCode == 0 && out != nil {
		defer C.easynet_authority_call_string_free(t.symbols.stringFree, out)
		return cabiErrorFromLastErrorJSON([]byte(C.GoString(out)), true, code, fallback)
	}
	return cabiErrorFromLastErrorJSON(nil, false, code, fallback)
}

func bindCABIAuthoritySymbols(library unsafe.Pointer) (cabiAuthoritySymbols, error) {
	bindings := []struct {
		name string
		set  func(*cabiAuthoritySymbols, unsafe.Pointer)
	}{
		{"easynet_abi_version", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.abiVersion = ptr }},
		{"easynet_last_error_json", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.lastErrorJSON = ptr }},
		{"easynet_string_free", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.stringFree = ptr }},
		{"easynet_authority_prepare_delegation", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.prepareDelegation = ptr }},
		{"easynet_authority_materialize_delegation", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.materializeDelegation = ptr }},
		{"easynet_authority_prepare_session", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.prepareSession = ptr }},
		{"easynet_authority_materialize_session", func(s *cabiAuthoritySymbols, ptr unsafe.Pointer) { s.materializeSession = ptr }},
	}
	var symbols cabiAuthoritySymbols
	for _, binding := range bindings {
		ptr, err := requireCABISymbol(library, binding.name)
		if err != nil {
			return cabiAuthoritySymbols{}, err
		}
		binding.set(&symbols, ptr)
	}
	return symbols, nil
}
