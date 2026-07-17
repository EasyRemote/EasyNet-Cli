package easynet

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"

	axoninv "easynet.run/axon/sdk/go/easynet/invocation"
)

// UraProfileEasynetStrictV2 is the URA profile pinned by the EasyNet daemon
// admission path for signed Invocation material.
const UraProfileEasynetStrictV2 = "easynet-strict-v2"

// CanonicalInvocationBytes returns the descriptor-bound AXIOM
// caller-signature byte sequence for an SDK Invocation envelope. The byte
// layout is owned by Axon; this facade owns only SDK DTO validation and
// projection into Axon's canonical descriptor-bound encoder.
func CanonicalInvocationBytes(envelope Envelope, ability string, args []byte) ([]byte, error) {
	if envelope.Caller.URA == "" {
		return nil, errors.New("canonical: empty Caller.URA")
	}
	if envelope.Callee.URA == "" {
		return nil, errors.New("canonical: empty Callee.URA")
	}
	if envelope.Subject.URA == "" {
		return nil, errors.New("canonical: empty Subject.URA")
	}
	if ability == "" {
		return nil, errors.New("canonical: empty ability")
	}
	if len(envelope.Nonce) != 16 {
		return nil, fmt.Errorf("canonical: nonce must be 16 bytes, got %d", len(envelope.Nonce))
	}

	digest := sha256.Sum256(args)
	var nonce [16]byte
	copy(nonce[:], envelope.Nonce)
	causalContext, err := canonicalCausalContext(envelope.CausalContext)
	if err != nil {
		return nil, fmt.Errorf("canonical: causal context: %w", err)
	}
	env := axoninv.InvocationEnvelope{
		Caller:          axoninv.NewAgentIdentity(envelope.Caller.URA, axoninv.ProfileEasynetStrictV2),
		Callee:          axoninv.NewAgentIdentity(envelope.Callee.URA, axoninv.ProfileEasynetStrictV2),
		Subject:         axoninv.NewSubjectIdentity(envelope.Subject.URA, axoninv.ProfileEasynetStrictV2),
		Ability:         ability,
		ArgsDigest:      digest,
		InvocationNonce: nonce,
		CausalContext:   causalContext,
	}
	bound, err := axoninv.NewDescriptorBoundEnvelope(env)
	if err != nil {
		return nil, fmt.Errorf("descriptor-bound canonical: %w", err)
	}
	return axoninv.CanonicalDescriptorBoundInvocationBytes(bound)
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
