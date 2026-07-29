package easynet

import (
	"context"
	"fmt"
	"strings"
)

// RuntimeSigningTransport decorates a RuntimeTransport with caller signing.
// It signs complete Invocation drafts that do not already carry a
// caller_signature, then delegates to the wrapped transport. It never verifies
// signatures and never changes runtime admission policy; those remain Axon and
// runtime responsibilities.
type RuntimeSigningTransport struct {
	next   RuntimeTransport
	signer Signer
}

func NewRuntimeSigningTransport(next RuntimeTransport, signer Signer) (*RuntimeSigningTransport, error) {
	if next == nil {
		return nil, invalidRuntimeClient("runtime transport is required")
	}
	if err := validateSignerHandle(signer.handle); err != nil {
		return nil, err
	}
	if signer.provider == nil {
		return nil, invalidInvocation("signature provider is required", nil)
	}
	return &RuntimeSigningTransport{next: next, signer: signer}, nil
}

func (t *RuntimeSigningTransport) Invoke(ctx context.Context, draftJSON []byte) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	raw, err := t.signDraftJSON(draftJSON)
	if err != nil {
		return nil, err
	}
	return next.Invoke(ctx, raw)
}

func (t *RuntimeSigningTransport) OpenStream(ctx context.Context, draftJSON []byte) (StreamTransport, []byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, nil, err
	}
	raw, err := t.signDraftJSON(draftJSON)
	if err != nil {
		return nil, nil, err
	}
	return next.OpenStream(ctx, raw)
}

func (t *RuntimeSigningTransport) OpenBidi(ctx context.Context, draftJSON []byte, streamsJSON []byte) (BidiTransport, []byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, nil, err
	}
	raw, err := t.signDraftJSON(draftJSON)
	if err != nil {
		return nil, nil, err
	}
	return next.OpenBidi(ctx, raw, streamsJSON)
}

func (t *RuntimeSigningTransport) Prepare(ctx context.Context, draftJSON []byte, optionsJSON []byte) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	return next.Prepare(ctx, draftJSON, optionsJSON)
}

func (t *RuntimeSigningTransport) SubmitSigned(ctx context.Context, signedJSON []byte) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	return next.SubmitSigned(ctx, signedJSON)
}

func (t *RuntimeSigningTransport) AwaitHandle(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	return next.AwaitHandle(ctx, control)
}

func (t *RuntimeSigningTransport) CancelHandle(ctx context.Context, control InvocationControlCapability, reason string) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	return next.CancelHandle(ctx, control, reason)
}

func (t *RuntimeSigningTransport) HandleEvents(ctx context.Context, control InvocationControlCapability) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	return next.HandleEvents(ctx, control)
}

func (t *RuntimeSigningTransport) FreeHandle(ctx context.Context, control InvocationControlCapability) error {
	next, err := t.nextTransport()
	if err != nil {
		return err
	}
	return next.FreeHandle(ctx, control)
}

func (t *RuntimeSigningTransport) ResolveDescriptorRef(ctx context.Context, requestJSON []byte) ([]byte, error) {
	next, err := t.nextTransport()
	if err != nil {
		return nil, err
	}
	resolver, ok := next.(DescriptorResolverTransport)
	if !ok {
		return nil, invalidRuntimeClient("runtime transport does not expose descriptor resolution")
	}
	return resolver.ResolveDescriptorRef(ctx, requestJSON)
}

func (t *RuntimeSigningTransport) Close(ctx context.Context) error {
	next, err := t.nextTransport()
	if err != nil {
		return err
	}
	return next.Close(ctx)
}

func (t *RuntimeSigningTransport) signDraftJSON(raw []byte) ([]byte, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return nil, err
	}
	signed, err := t.signer.signInvocationDraft(draft)
	if err != nil {
		return nil, err
	}
	out, err := signed.MarshalJSON()
	if err != nil {
		return nil, invalidRuntimePayload(fmt.Sprintf("encode signed invocation draft: %v", err), err)
	}
	return out, nil
}

func (t *RuntimeSigningTransport) nextTransport() (RuntimeTransport, error) {
	if t == nil || t.next == nil {
		return nil, invalidRuntimeClient("runtime signing transport is not initialized")
	}
	return t.next, nil
}

func causalContextForInvocationDraft(value map[string]any) (CausalContext, error) {
	form := causalString(value, "form")
	switch strings.ToLower(strings.TrimSpace(form)) {
	case "none":
		if err := requireCausalExactKeys(value, "causal_context", "form"); err != nil {
			return CausalContext{}, err
		}
		return CausalRoot(), nil
	case "scalar":
		if err := requireCausalExactKeys(value, "causal_context", "form", "receipt_hash_hex", "receipt_ura"); err != nil {
			return CausalContext{}, err
		}
		ref, err := causalReceiptRefFromValue(value)
		if err != nil {
			return CausalContext{}, err
		}
		return CausalScalarRef(ref), nil
	case "list":
		if err := requireCausalExactKeys(value, "causal_context", "form", "prior"); err != nil {
			return CausalContext{}, err
		}
		raw, ok := value["prior"].([]any)
		if !ok || len(raw) == 0 {
			return CausalContext{}, invalidRuntimePayload("causal_context list requires non-empty prior receipts", nil)
		}
		refs := make([]CausalReceiptRef, 0, len(raw))
		for i, item := range raw {
			ref, err := causalReceiptRefFromValue(item)
			if err != nil {
				return CausalContext{}, invalidRuntimePayload(fmt.Sprintf("causal_context prior[%d]: %v", i, err), err)
			}
			refs = append(refs, ref)
		}
		return CausalVectorRefs(refs), nil
	case "merkle":
		if err := requireCausalExactKeys(value, "causal_context", "form", "root_hex", "proof_ura"); err != nil {
			return CausalContext{}, err
		}
		rootHex := causalString(value, "root_hex")
		proofURA := causalString(value, "proof_ura")
		if rootHex == "" || proofURA == "" {
			return CausalContext{}, invalidRuntimePayload("causal_context merkle requires root_hex and proof_ura", nil)
		}
		return CausalDAGRoot(rootHex, proofURA), nil
	case "":
		return CausalContext{}, invalidRuntimePayload("causal_context form is required", nil)
	default:
		return CausalContext{}, invalidRuntimePayload(fmt.Sprintf("unknown causal_context form: %s", form), nil)
	}
}

func causalReceiptRefFromValue(value any) (CausalReceiptRef, error) {
	item, ok := value.(map[string]any)
	if !ok {
		return CausalReceiptRef{}, invalidRuntimePayload("causal receipt ref must be an object", nil)
	}
	if err := requireCausalExactKeys(item, "causal receipt ref", "receipt_hash_hex", "receipt_ura"); err != nil {
		return CausalReceiptRef{}, err
	}
	ura := causalString(item, "receipt_ura")
	hashHex := causalString(item, "receipt_hash_hex")
	if ura == "" || hashHex == "" {
		return CausalReceiptRef{}, invalidRuntimePayload("causal receipt ref requires receipt_ura and receipt_hash_hex", nil)
	}
	return CausalReceiptRef{URA: ura, HashHex: hashHex}, nil
}

func requireCausalExactKeys(value map[string]any, label string, allowed ...string) error {
	allowedSet := make(map[string]struct{}, len(allowed))
	for _, key := range allowed {
		allowedSet[key] = struct{}{}
		if _, ok := value[key]; !ok {
			return invalidRuntimePayload(fmt.Sprintf("%s missing required field %s", label, key), nil)
		}
	}
	for key := range value {
		if _, ok := allowedSet[key]; !ok {
			return invalidRuntimePayload(fmt.Sprintf("%s contains unsupported field %s", label, key), nil)
		}
	}
	return nil
}

func causalString(value map[string]any, key string) string {
	raw, _ := value[key].(string)
	return strings.TrimSpace(raw)
}
