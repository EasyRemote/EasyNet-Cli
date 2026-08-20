package easynet

import (
	"encoding/hex"
	"errors"
	"fmt"

	axoninv "axon.run/sdk/go/axon/invocation"
)

// uraProfileStrictV2 is the canonical strict URA profile pinned by
// descriptor-bound requests and receipts.
const uraProfileStrictV2 = "axon-strict-v2"

func canonicalInvocationURAProfile() (string, error) {
	profile, err := axoninv.ParseUraProfile(uraProfileStrictV2)
	if err != nil {
		return "", err
	}
	return string(profile), nil
}

// canonicalDescriptorBoundInvocationBytes projects the facade DTO into Axon's
// descriptor-bound proof model. It is internal signing infrastructure, not an
// alternate public proof API.
func canonicalDescriptorBoundInvocationBytes(envelope Envelope, ability string, args []byte) ([]byte, error) {
	bound, err := descriptorBoundInvocationEnvelope(envelope, ability, args)
	if err != nil {
		return nil, err
	}
	return bound.CanonicalBytes()
}

func descriptorBoundInvocationDraft(draft InvocationDraft) (axoninv.DescriptorBoundInvocationDraft, error) {
	nonce, err := decodeBase64Field(draft.NonceBase64(), "nonce_base64")
	if err != nil {
		return axoninv.DescriptorBoundInvocationDraft{}, err
	}
	causal, err := causalContextForInvocationDraft(draft.CausalContext())
	if err != nil {
		return axoninv.DescriptorBoundInvocationDraft{}, err
	}
	args, err := invocationDraftArgumentBytes(draft)
	if err != nil {
		return axoninv.DescriptorBoundInvocationDraft{}, err
	}
	return descriptorBoundInvocationEnvelope(Envelope{
		Caller:        AgentRef{URA: draft.CallerURA()},
		Callee:        AgentRef{URA: draft.CalleeURA()},
		Subject:       SubjectRef{URA: draft.SubjectURA()},
		Nonce:         nonce,
		CausalContext: causal,
	}, draft.DescriptorRef(), args)
}

func descriptorBoundInvocationEnvelope(
	envelope Envelope,
	ability string,
	args []byte,
) (axoninv.DescriptorBoundInvocationDraft, error) {
	if envelope.Caller.URA == "" {
		return axoninv.DescriptorBoundInvocationDraft{}, errors.New("canonical: empty Caller.URA")
	}
	if envelope.Callee.URA == "" {
		return axoninv.DescriptorBoundInvocationDraft{}, errors.New("canonical: empty Callee.URA")
	}
	if envelope.Subject.URA == "" {
		return axoninv.DescriptorBoundInvocationDraft{}, errors.New("canonical: empty Subject.URA")
	}
	if ability == "" {
		return axoninv.DescriptorBoundInvocationDraft{}, errors.New("canonical: empty ability")
	}
	if len(envelope.Nonce) != 16 {
		return axoninv.DescriptorBoundInvocationDraft{}, fmt.Errorf("canonical: nonce must be 16 bytes, got %d", len(envelope.Nonce))
	}

	var nonce [16]byte
	copy(nonce[:], envelope.Nonce)
	causalContext, err := canonicalCausalContext(envelope.CausalContext)
	if err != nil {
		return axoninv.DescriptorBoundInvocationDraft{}, fmt.Errorf("canonical: causal context: %w", err)
	}
	bound, err := axoninv.NewDescriptorBoundInvocationBuilder().
		WithCaller(axoninv.NewAgentIdentity(envelope.Caller.URA, axoninv.ProfileStrictV2)).
		WithCallee(axoninv.NewAgentIdentity(envelope.Callee.URA, axoninv.ProfileStrictV2)).
		WithSubject(axoninv.NewSubjectIdentity(envelope.Subject.URA, axoninv.ProfileStrictV2)).
		WithDescriptorRef(ability).
		WithNonce(nonce).
		WithCausalContext(causalContext).
		WithPayload(args).
		Build()
	if err != nil {
		return axoninv.DescriptorBoundInvocationDraft{}, fmt.Errorf("descriptor-bound canonical: %w", err)
	}
	return bound, nil
}

func canonicalCausalContext(cc CausalContext) (axoninv.CausalContext, error) {
	switch cc.Kind {
	case CausalContextNull:
		return axoninv.CausalNoneCtx(), nil
	case CausalContextScalar:
		hash, err := decodeReceiptHashHex(cc.Scalar.HashHex)
		if err != nil {
			return axoninv.CausalContext{}, fmt.Errorf("scalar receipt hash: %w", err)
		}
		if cc.Scalar.URA == "" {
			return axoninv.CausalContext{}, errors.New("scalar receipt URA empty")
		}
		return axoninv.CausalScalarCtx(axoninv.ReceiptRef{
			ReceiptHash: hash,
			ReceiptURA:  cc.Scalar.URA,
		}), nil
	case CausalContextVector:
		if len(cc.Vector) == 0 {
			return axoninv.CausalContext{}, errors.New("vector form requires non-empty predecessor list")
		}
		refs := make([]axoninv.ReceiptRef, 0, len(cc.Vector))
		for i, ref := range cc.Vector {
			hash, err := decodeReceiptHashHex(ref.HashHex)
			if err != nil {
				return axoninv.CausalContext{}, fmt.Errorf("vector[%d] receipt hash: %w", i, err)
			}
			if ref.URA == "" {
				return axoninv.CausalContext{}, fmt.Errorf("vector[%d] receipt URA empty", i)
			}
			refs = append(refs, axoninv.ReceiptRef{ReceiptHash: hash, ReceiptURA: ref.URA})
		}
		return axoninv.CausalListCtx(refs), nil
	case CausalContextDAG:
		root, err := decodeReceiptHashHex(cc.DAGRootHex)
		if err != nil {
			return axoninv.CausalContext{}, fmt.Errorf("DAG root: %w", err)
		}
		if cc.DAGProofURA == "" {
			return axoninv.CausalContext{}, errors.New("DAG proof URA empty")
		}
		return axoninv.CausalMerkleCtx(root, cc.DAGProofURA), nil
	default:
		return axoninv.CausalContext{}, fmt.Errorf("unknown CausalContextKind %d", cc.Kind)
	}
}

func decodeReceiptHashHex(raw string) ([32]byte, error) {
	var out [32]byte
	if raw == "" {
		return out, errors.New("receipt hash hex empty")
	}
	if len(raw) != 64 {
		return out, fmt.Errorf("receipt hash hex must be 64 chars (32 bytes), got %d", len(raw))
	}
	decoded, err := hex.DecodeString(raw)
	if err != nil {
		return out, err
	}
	copy(out[:], decoded)
	return out, nil
}
