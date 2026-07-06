package easynet

import axonsdk "easynet.run/axon/sdk/go/easynet"

const (
	InvokeRemoteRequestType = axonsdk.InvokeRemoteRequestType
	InvokeRemoteChunkType   = axonsdk.InvokeRemoteChunkType
	InvokeRemoteResultType  = axonsdk.InvokeRemoteResultType
	OriginCallerNonceSize   = axonsdk.OriginCallerNonceSize
)

type JSONByteSlice = axonsdk.JSONByteSlice
type InvokeRemoteContentEnvelope = axonsdk.InvokeRemoteContentEnvelope
type OriginCallerClaim = axonsdk.OriginCallerClaim
type InvokeRemoteUpRequest = axonsdk.InvokeRemoteUpRequest
type InvokeRemoteDownChunk = axonsdk.InvokeRemoteDownChunk
type InvokeRemoteDownResult = axonsdk.InvokeRemoteDownResult
type InvokeRemoteDownFrame = axonsdk.InvokeRemoteDownFrame

func PlainInvokeRemoteContentEnvelope() InvokeRemoteContentEnvelope {
	return axonsdk.PlainInvokeRemoteContentEnvelope()
}

func NewOriginCallerClaim(callerURA, ability string, signature, signerPubkey, nonce []byte) (OriginCallerClaim, error) {
	return axonsdk.NewOriginCallerClaim(callerURA, ability, signature, signerPubkey, nonce)
}

func MarshalInvokeRemoteUpRequest(request InvokeRemoteUpRequest) ([]byte, error) {
	return axonsdk.MarshalInvokeRemoteUpRequest(request)
}

func UnmarshalInvokeRemoteUpRequest(data []byte) (InvokeRemoteUpRequest, error) {
	return axonsdk.UnmarshalInvokeRemoteUpRequest(data)
}

func DecodeInvokeRemoteDown(data []byte) (InvokeRemoteDownFrame, error) {
	return axonsdk.DecodeInvokeRemoteDown(data)
}
