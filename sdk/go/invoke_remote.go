package easynet

import (
	"encoding/json"

	axonsdk "easynet.run/axon/sdk/go/easynet"
)

const (
	InvokeRemoteRequestType = axonsdk.InvokeRemoteRequestType
	InvokeRemoteChunkType   = axonsdk.InvokeRemoteChunkType
	InvokeRemoteResultType  = axonsdk.InvokeRemoteResultType
	OriginCallerNonceSize   = axonsdk.OriginCallerNonceSize
)

// JSONByteSlice is the SDK facade byte-array JSON shape used by invoke_remote.
// Encoding and validation delegate to Axon so this package does not own the
// wire grammar.
type JSONByteSlice []byte

func (b JSONByteSlice) MarshalJSON() ([]byte, error) {
	return json.Marshal(axonsdk.JSONByteSlice(b))
}

func (b *JSONByteSlice) UnmarshalJSON(data []byte) error {
	var out axonsdk.JSONByteSlice
	if err := json.Unmarshal(data, &out); err != nil {
		return err
	}
	*b = JSONByteSlice(out)
	return nil
}

type InvokeRemoteContentEnvelope struct {
	ContentType string `json:"content_type"`
	Encoding    string `json:"encoding"`
	SchemaURA   string `json:"schema_ura"`
	Encryption  int32  `json:"encryption"`
	KeyID       string `json:"key_id"`
}

func PlainInvokeRemoteContentEnvelope() InvokeRemoteContentEnvelope {
	return contentEnvelopeFromAxon(axonsdk.PlainInvokeRemoteContentEnvelope())
}

type OriginCallerClaim struct {
	CallerURA       string `json:"caller_ura"`
	Ability         string `json:"ability"`
	SignatureB64    string `json:"signature_b64"`
	SignerPubkeyB64 string `json:"signer_pubkey_b64"`
	NonceB64        string `json:"nonce_b64"`
}

type InvokeRemoteUpRequest struct {
	Type                string                      `json:"type"`
	SubjectDevice       string                      `json:"subject_device"`
	SubjectURA          string                      `json:"subject_ura,omitempty"`
	AbilityURA          string                      `json:"ability_ura"`
	Args                JSONByteSlice               `json:"args"`
	ArgsContentEnvelope InvokeRemoteContentEnvelope `json:"args_content_envelope"`
	Metadata            map[string]string           `json:"metadata,omitempty"`
	OriginCaller        *OriginCallerClaim          `json:"origin_caller,omitempty"`
}

type InvokeRemoteDownChunk struct {
	Type    string        `json:"type"`
	Payload JSONByteSlice `json:"payload"`
}

type InvokeRemoteDownResult struct {
	Type      string        `json:"type"`
	Payload   JSONByteSlice `json:"payload"`
	Error     *string       `json:"error"`
	RequestID string        `json:"request_id,omitempty"`
}

type InvokeRemoteDownFrame struct {
	ChunkPayload []byte
	Result       *InvokeRemoteDownResult
}

func NewOriginCallerClaim(callerURA, ability string, signature, signerPubkey, nonce []byte) (OriginCallerClaim, error) {
	claim, err := axonsdk.NewOriginCallerClaim(callerURA, ability, signature, signerPubkey, nonce)
	if err != nil {
		return OriginCallerClaim{}, err
	}
	return originCallerClaimFromAxon(claim), nil
}

func MarshalInvokeRemoteUpRequest(request InvokeRemoteUpRequest) ([]byte, error) {
	return axonsdk.MarshalInvokeRemoteUpRequest(request.toAxon())
}

func UnmarshalInvokeRemoteUpRequest(data []byte) (InvokeRemoteUpRequest, error) {
	request, err := axonsdk.UnmarshalInvokeRemoteUpRequest(data)
	if err != nil {
		return InvokeRemoteUpRequest{}, err
	}
	return invokeRemoteUpRequestFromAxon(request), nil
}

func DecodeInvokeRemoteDown(data []byte) (InvokeRemoteDownFrame, error) {
	frame, err := axonsdk.DecodeInvokeRemoteDown(data)
	if err != nil {
		return InvokeRemoteDownFrame{}, err
	}
	return invokeRemoteDownFrameFromAxon(frame), nil
}

func (r InvokeRemoteUpRequest) toAxon() axonsdk.InvokeRemoteUpRequest {
	return axonsdk.InvokeRemoteUpRequest{
		Type:                r.Type,
		SubjectDevice:       r.SubjectDevice,
		SubjectURA:          r.SubjectURA,
		AbilityURA:          r.AbilityURA,
		Args:                axonsdk.JSONByteSlice(r.Args),
		ArgsContentEnvelope: r.ArgsContentEnvelope.toAxon(),
		Metadata:            cloneStringMap(r.Metadata),
		OriginCaller:        originCallerClaimPtrToAxon(r.OriginCaller),
	}
}

func invokeRemoteUpRequestFromAxon(r axonsdk.InvokeRemoteUpRequest) InvokeRemoteUpRequest {
	return InvokeRemoteUpRequest{
		Type:                r.Type,
		SubjectDevice:       r.SubjectDevice,
		SubjectURA:          r.SubjectURA,
		AbilityURA:          r.AbilityURA,
		Args:                JSONByteSlice(r.Args),
		ArgsContentEnvelope: contentEnvelopeFromAxon(r.ArgsContentEnvelope),
		Metadata:            cloneStringMap(r.Metadata),
		OriginCaller:        originCallerClaimPtrFromAxon(r.OriginCaller),
	}
}

func (e InvokeRemoteContentEnvelope) toAxon() axonsdk.InvokeRemoteContentEnvelope {
	return axonsdk.InvokeRemoteContentEnvelope{
		ContentType: e.ContentType,
		Encoding:    e.Encoding,
		SchemaURA:   e.SchemaURA,
		Encryption:  e.Encryption,
		KeyID:       e.KeyID,
	}
}

func contentEnvelopeFromAxon(e axonsdk.InvokeRemoteContentEnvelope) InvokeRemoteContentEnvelope {
	return InvokeRemoteContentEnvelope{
		ContentType: e.ContentType,
		Encoding:    e.Encoding,
		SchemaURA:   e.SchemaURA,
		Encryption:  e.Encryption,
		KeyID:       e.KeyID,
	}
}

func (c OriginCallerClaim) toAxon() axonsdk.OriginCallerClaim {
	return axonsdk.OriginCallerClaim{
		CallerURA:       c.CallerURA,
		Ability:         c.Ability,
		SignatureB64:    c.SignatureB64,
		SignerPubkeyB64: c.SignerPubkeyB64,
		NonceB64:        c.NonceB64,
	}
}

func originCallerClaimFromAxon(c axonsdk.OriginCallerClaim) OriginCallerClaim {
	return OriginCallerClaim{
		CallerURA:       c.CallerURA,
		Ability:         c.Ability,
		SignatureB64:    c.SignatureB64,
		SignerPubkeyB64: c.SignerPubkeyB64,
		NonceB64:        c.NonceB64,
	}
}

func originCallerClaimPtrToAxon(c *OriginCallerClaim) *axonsdk.OriginCallerClaim {
	if c == nil {
		return nil
	}
	out := c.toAxon()
	return &out
}

func originCallerClaimPtrFromAxon(c *axonsdk.OriginCallerClaim) *OriginCallerClaim {
	if c == nil {
		return nil
	}
	out := originCallerClaimFromAxon(*c)
	return &out
}

func invokeRemoteDownFrameFromAxon(frame axonsdk.InvokeRemoteDownFrame) InvokeRemoteDownFrame {
	return InvokeRemoteDownFrame{
		ChunkPayload: append([]byte(nil), frame.ChunkPayload...),
		Result:       invokeRemoteDownResultPtrFromAxon(frame.Result),
	}
}

func invokeRemoteDownResultPtrFromAxon(result *axonsdk.InvokeRemoteDownResult) *InvokeRemoteDownResult {
	if result == nil {
		return nil
	}
	return &InvokeRemoteDownResult{
		Type:      result.Type,
		Payload:   JSONByteSlice(result.Payload),
		Error:     cloneStringPtr(result.Error),
		RequestID: result.RequestID,
	}
}

func cloneStringMap(in map[string]string) map[string]string {
	if len(in) == 0 {
		return nil
	}
	out := make(map[string]string, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}

func cloneStringPtr(in *string) *string {
	if in == nil {
		return nil
	}
	out := *in
	return &out
}
