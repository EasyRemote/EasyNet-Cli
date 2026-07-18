package easynet

import "fmt"

// Envelope is the canonical runtime invocation identity and binding tuple used
// by SDK transport implementations. Authorization, idempotency, content
// metadata, timeout, and trace labels live outside this signed tuple.
type Envelope struct {
	RequestID string

	Caller  AgentRef
	Callee  AgentRef
	Subject SubjectRef

	CausalContext CausalContext
	Nonce         []byte

	PresignedCallerSignature *PresignedCallerSignature
}

type PresignedCallerSignature struct {
	Signature       []byte
	SignerPublicKey []byte
}

type AgentRef struct {
	URA string
}

type SubjectRef struct {
	URA string
}

type CausalContextKind int

const (
	CausalContextNull CausalContextKind = iota
	CausalContextScalar
	CausalContextVector
	CausalContextDAG
)

type CausalReceiptRef struct {
	HashHex string
	URA     string
}

type CausalContext struct {
	Kind CausalContextKind

	Reason string

	Scalar CausalReceiptRef
	Vector []CausalReceiptRef

	DAGRootHex  string
	DAGProofURA string
}

func CausalNullWithReason(reason string) CausalContext {
	return CausalContext{Kind: CausalContextNull, Reason: reason}
}

func CausalScalarRef(ref CausalReceiptRef) CausalContext {
	return CausalContext{Kind: CausalContextScalar, Scalar: ref}
}

func CausalVectorRefs(refs []CausalReceiptRef) CausalContext {
	cp := make([]CausalReceiptRef, len(refs))
	copy(cp, refs)
	return CausalContext{Kind: CausalContextVector, Vector: cp}
}

func CausalDAGRoot(rootHex string, proofURA string) CausalContext {
	return CausalContext{Kind: CausalContextDAG, DAGRootHex: rootHex, DAGProofURA: proofURA}
}

type ContentEncryptionAlgorithm string

const (
	ContentEncryptionUnspecified      ContentEncryptionAlgorithm = ""
	ContentEncryptionAES256GCM        ContentEncryptionAlgorithm = "aes-256-gcm"
	ContentEncryptionChaCha20Poly1305 ContentEncryptionAlgorithm = "chacha20-poly1305"
)

type ContentEnvelope struct {
	ContentType string
	Encoding    string
	SchemaURA   string
	Encryption  ContentEncryptionAlgorithm
	KeyID       string
}

type InvokeRequest struct {
	Envelope Envelope

	Ability       string
	AbilityURA    string
	ArgumentsJSON []byte

	ContentEnvelope  *ContentEnvelope
	Delegation       *DelegationProof
	SessionAuthority *SessionAuthority

	TimeoutMS      int
	IdempotencyKey string
}

type InvokeResponse struct {
	InvocationID      string
	State             string
	ResultJSON        []byte
	ResultContentType string
	AdmissionReceipt  []byte
	TerminalReceipt   []byte
	Error             *InvokeError
}

type InvokeError struct {
	Code          string
	Message       string
	Stage         string
	SecurityClass string
	Retryable     bool
	Context       map[string]string
}

func (e *InvokeError) Error() string {
	if e == nil {
		return "<nil InvokeError>"
	}
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}
