//go:build runtime_direct

package easynet

import (
	"context"
	"encoding/base64"
	"fmt"
	"math"
	"strings"
	"time"

	axoninv "axon.run/sdk/go/axon/invocation"
	"easynet.run/cli/sdk/go/internal/axonpb"
)

const directBidiContractVersion uint32 = 1

// directDescriptorBoundCodec maps SDK DTOs into Axon builders and lowers the
// resulting immutable Axon projection into protobuf carrier messages.
type directDescriptorBoundCodec struct {
	timeoutSeconds int32
}

type directDescriptorBoundRequest struct {
	draft      InvocationDraft
	projection axoninv.DescriptorBoundWireProjection
}

func newDirectDescriptorBoundCodec(invokeTimeout time.Duration) (*directDescriptorBoundCodec, error) {
	timeoutSeconds, err := directWireTimeoutSeconds(invokeTimeout)
	if err != nil {
		return nil, err
	}
	return &directDescriptorBoundCodec{timeoutSeconds: timeoutSeconds}, nil
}

func (c *directDescriptorBoundCodec) decode(
	ctx context.Context,
	raw []byte,
	callMode axoninv.CallMode,
) (directDescriptorBoundRequest, error) {
	draft, err := NewInvocationDraftFromJSON(raw)
	if err != nil {
		return directDescriptorBoundRequest{}, err
	}
	return c.build(ctx, draft, callMode)
}

func (c *directDescriptorBoundCodec) build(
	ctx context.Context,
	draft InvocationDraft,
	callMode axoninv.CallMode,
) (directDescriptorBoundRequest, error) {
	if c == nil {
		return directDescriptorBoundRequest{}, invalidRuntimeClient("direct runtime descriptor-bound codec is not initialized")
	}
	if ctx == nil {
		return directDescriptorBoundRequest{}, invalidRuntimeClient("context is required")
	}
	bound, err := descriptorBoundInvocationDraft(draft)
	if err != nil {
		return directDescriptorBoundRequest{}, invalidRuntimePayload(
			fmt.Sprintf("build Axon descriptor-bound invocation: %v", err),
			err,
		)
	}
	signature, err := directCallerSignatureForAxon(draft)
	if err != nil {
		return directDescriptorBoundRequest{}, err
	}
	request, err := bound.BindCallerSignature(callMode, signature)
	if err != nil {
		return directDescriptorBoundRequest{}, invalidRuntimePayload(
			fmt.Sprintf("bind Axon descriptor-bound invocation: %v", err),
			err,
		)
	}
	metadata, err := directMetadata(draft.Metadata())
	if err != nil {
		return directDescriptorBoundRequest{}, err
	}
	projection, err := axoninv.NewDescriptorBoundWireProjectionBuilder().Build(
		request,
		axoninv.DescriptorBoundWireOptions{
			ContentType:    draft.ContentType(),
			TimeoutSeconds: c.timeoutSeconds,
			Metadata:       metadata,
		},
	)
	if err != nil {
		return directDescriptorBoundRequest{}, invalidRuntimePayload(
			fmt.Sprintf("project Axon descriptor-bound wire request: %v", err),
			err,
		)
	}
	return directDescriptorBoundRequest{
		draft:      draft,
		projection: projection,
	}, nil
}

func (r directDescriptorBoundRequest) unary() (*axonpb.InvokeRequest, error) {
	if err := r.requireCallMode(axoninv.CallModeRPC); err != nil {
		return nil, err
	}
	envelope, err := r.wireEnvelope()
	if err != nil {
		return nil, err
	}
	return &axonpb.InvokeRequest{
		Envelope:        envelope,
		Target:          directInvocationTarget(r.projection.DescriptorRef(), r.projection.RouteName()),
		Arguments:       r.projection.Payload(),
		ContentType:     r.projection.ContentType(),
		TimeoutSeconds:  r.projection.TimeoutSeconds(),
		Metadata:        r.wireMetadata(),
		ContentEnvelope: r.wireContentEnvelope(),
	}, nil
}

func (r directDescriptorBoundRequest) stream() (*axonpb.InvokeServerStreamRequest, error) {
	if err := r.requireCallMode(axoninv.CallModeStream); err != nil {
		return nil, err
	}
	envelope, err := r.wireEnvelope()
	if err != nil {
		return nil, err
	}
	return &axonpb.InvokeServerStreamRequest{
		Envelope:        envelope,
		Target:          directInvocationTarget(r.projection.DescriptorRef(), r.projection.RouteName()),
		Arguments:       r.projection.Payload(),
		ContentType:     r.projection.ContentType(),
		TimeoutSeconds:  r.projection.TimeoutSeconds(),
		Metadata:        r.wireMetadata(),
		ContentEnvelope: r.wireContentEnvelope(),
	}, nil
}

func (r directDescriptorBoundRequest) bidi(streams []*axonpb.StreamDescriptor) (*axonpb.InvokeBidiUp, error) {
	if err := r.requireCallMode(axoninv.CallModeBidi); err != nil {
		return nil, err
	}
	signature := r.wireCallerSignature()
	envelope, err := r.wireEnvelope()
	if err != nil {
		return nil, err
	}
	return &axonpb.InvokeBidiUp{
		Sequence: 0,
		Mac:      append([]byte(nil), signature.GetSignature()...),
		Payload: &axonpb.InvokeBidiUp_EnvelopeOpen{EnvelopeOpen: &axonpb.EnvelopeOpen{
			Envelope:        envelope,
			Target:          directInvocationTarget(r.projection.DescriptorRef(), r.projection.RouteName()),
			InitialArgs:     r.projection.Payload(),
			ArgsContentType: r.projection.ContentType(),
			Streams:         cloneDirectStreamDescriptors(streams),
			Metadata:        r.wireMetadata(),
			ContentEnvelope: r.wireContentEnvelope(),
			SessionExt:      &axonpb.SessionOpenExt{ContractVersion: directBidiContractVersion},
		}},
	}, nil
}

func (r directDescriptorBoundRequest) wireEnvelope() (*axonpb.Envelope, error) {
	envelope := r.projection.Envelope()
	causal, err := directWireCausalContext(envelope.CausalContext())
	if err != nil {
		return nil, err
	}
	nonce := envelope.Nonce()
	return &axonpb.Envelope{
		RequestId:       envelope.RequestID(),
		Caller:          directAgentIdentity(envelope.Caller()),
		Callee:          directAgentIdentity(envelope.Callee()),
		Subject:         directSubjectIdentity(envelope.Subject()),
		InvocationNonce: append([]byte(nil), nonce[:]...),
		CausalContext:   causal,
		CallerSignature: r.wireCallerSignature(),
	}, nil
}

func (r directDescriptorBoundRequest) wireCallerSignature() *axonpb.CallerSignature {
	signature := r.projection.Envelope().CallerSignature()
	return &axonpb.CallerSignature{
		Algorithm: signature.Algorithm,
		Signature: append([]byte(nil), signature.Signature...),
		KeyIdHint: signature.KeyIDHint,
	}
}

func (r directDescriptorBoundRequest) wireContentEnvelope() *axonpb.ContentEnvelope {
	return &axonpb.ContentEnvelope{
		ContentType: r.projection.ContentType(),
		Encoding:    r.projection.ContentEncoding(),
	}
}

func (r directDescriptorBoundRequest) wireMetadata() map[string]string {
	return r.projection.Metadata()
}

func (r directDescriptorBoundRequest) requireCallMode(expected axoninv.CallMode) error {
	if r.projection.CallMode() != expected {
		return invalidRuntimePayload(
			fmt.Sprintf(
				"Axon wire projection call mode %q cannot lower as %q",
				r.projection.CallMode(),
				expected,
			),
			nil,
		)
	}
	return nil
}

func directInvocationTarget(descriptorRef string, routeName string) *axonpb.InvocationTarget {
	return &axonpb.InvocationTarget{
		TypedTarget: &axonpb.InvocationTarget_Ability{
			Ability: &axonpb.AbilityTarget{
				AbilityName:  descriptorRef,
				FunctionName: routeName,
			},
		},
	}
}

func directAgentIdentity(identity axoninv.AgentIdentity) *axonpb.AgentIdentity {
	return &axonpb.AgentIdentity{Ura: identity.URA, Profile: string(identity.Profile)}
}

func directSubjectIdentity(identity axoninv.SubjectIdentity) *axonpb.SubjectIdentity {
	return &axonpb.SubjectIdentity{Ura: identity.URA, Profile: string(identity.Profile)}
}

func directCallerSignatureForAxon(draft InvocationDraft) (axoninv.CallerSignature, error) {
	signature := draft.CallerSignature()
	if signature == nil {
		return axoninv.CallerSignature{}, invalidRuntimePayload(
			"direct runtime dispatch requires caller_signature",
			nil,
		)
	}
	decoded, err := base64.StdEncoding.Strict().DecodeString(signature.SignatureBase64)
	if err != nil {
		return axoninv.CallerSignature{}, invalidRuntimePayload(fmt.Sprintf("decode caller_signature.signature_base64: %v", err), err)
	}
	if strings.TrimSpace(signature.KeyIDHint) == "" {
		return axoninv.CallerSignature{}, invalidRuntimePayload("caller_signature.key_id_hint is required", nil)
	}
	return axoninv.CallerSignature{
		Algorithm: signature.Algorithm,
		Signature: decoded,
		KeyIDHint: signature.KeyIDHint,
	}, nil
}

func directWireCausalContext(causal axoninv.CausalContext) (*axonpb.CausalContext, error) {
	switch causal.Form {
	case axoninv.CausalNone:
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_None{None: &axonpb.Empty{}}}, nil
	case axoninv.CausalScalar:
		ref, err := directWireReceiptRef(causal.Scalar)
		if err != nil {
			return nil, err
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Scalar{Scalar: ref}}, nil
	case axoninv.CausalList:
		prior := make([]*axonpb.ReceiptRef, 0, len(causal.List))
		for index := range causal.List {
			ref, err := directWireReceiptRef(&causal.List[index])
			if err != nil {
				return nil, err
			}
			prior = append(prior, ref)
		}
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_List{List: &axonpb.ReceiptList{Prior: prior}}}, nil
	case axoninv.CausalMerkle:
		return &axonpb.CausalContext{Form: &axonpb.CausalContext_Merkle{Merkle: &axonpb.MerkleRoot{
			Root:     append([]byte(nil), causal.MerkleRoot[:]...),
			ProofUra: causal.MerkleProofURA,
		}}}, nil
	default:
		return nil, invalidRuntimePayload("Axon wire projection contains unknown causal_context form", nil)
	}
}

func directWireReceiptRef(ref *axoninv.ReceiptRef) (*axonpb.ReceiptRef, error) {
	if ref == nil {
		return nil, invalidRuntimePayload("Axon wire projection contains an empty causal receipt reference", nil)
	}
	return &axonpb.ReceiptRef{
		ReceiptUra:  ref.ReceiptURA,
		ReceiptHash: append([]byte(nil), ref.ReceiptHash[:]...),
	}, nil
}

func directMetadata(metadata map[string]any) (map[string]string, error) {
	result := map[string]string{}
	for key, value := range metadata {
		stringValue, ok, err := directMetadataValueString(key, value)
		if err != nil {
			return nil, err
		}
		if !ok {
			continue
		}
		result[key] = stringValue
	}
	return result, nil
}

func directMetadataValueString(key string, value any) (string, bool, error) {
	switch typed := value.(type) {
	case nil:
		return "", false, nil
	case string:
		return typed, true, nil
	default:
		return "", false, invalidRuntimePayload(fmt.Sprintf("metadata[%q] must be a string for Axon InvokeRequest", key), nil)
	}
}

func directWireTimeoutSeconds(timeout time.Duration) (int32, error) {
	if timeout <= 0 {
		return 0, invalidRuntimePayload("direct runtime invoke timeout must be positive", nil)
	}
	seconds := (timeout + time.Second - 1) / time.Second
	if seconds > time.Duration(math.MaxInt32) {
		return 0, invalidRuntimePayload("direct runtime invoke timeout exceeds Axon wire range", nil)
	}
	return int32(seconds), nil
}

func cloneDirectStreamDescriptors(streams []*axonpb.StreamDescriptor) []*axonpb.StreamDescriptor {
	cloned := make([]*axonpb.StreamDescriptor, 0, len(streams))
	for _, stream := range streams {
		if stream == nil {
			continue
		}
		cloned = append(cloned, &axonpb.StreamDescriptor{
			StreamId:    stream.GetStreamId(),
			ContentType: stream.GetContentType(),
			CodecParams: stream.GetCodecParams(),
			Ordering:    stream.GetOrdering(),
		})
	}
	return cloned
}
