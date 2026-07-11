package easynet

import (
	"context"
	"reflect"
	"testing"
)

func TestPrincipalLifecycleContractIsComplete(t *testing.T) {
	type required interface {
		Create(context.Context, CreatePrincipalRequest) (PrincipalSnapshot, error)
		BindFirstKey(context.Context, BindPrincipalKeyRequest) (PrincipalSnapshot, error)
		AddKey(context.Context, BindPrincipalKeyRequest) (PrincipalSnapshot, error)
		RotateKey(context.Context, RotatePrincipalKeyRequest) (PrincipalSnapshot, error)
		RevokeKey(context.Context, RevokePrincipalKeyRequest) (PrincipalSnapshot, error)
		ConfigureRecovery(context.Context, ConfigureRecoveryRequest) (PrincipalSnapshot, error)
		Recover(context.Context, RecoverPrincipalRequest) (PrincipalSnapshot, error)
		Suspend(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
		Reactivate(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
		Delete(context.Context, ChangePrincipalStateRequest) (PrincipalSnapshot, error)
		IssueGrant(context.Context, IssueGrantRequest) (PrincipalSnapshot, error)
		RevokeGrant(context.Context, RevokeGrantRequest) (PrincipalSnapshot, error)
		Get(context.Context, string) (PrincipalSnapshot, error)
	}

	principalType := reflect.TypeOf((*PrincipalLifecycle)(nil)).Elem()
	requiredType := reflect.TypeOf((*required)(nil)).Elem()
	if !principalType.Implements(requiredType) || !requiredType.Implements(principalType) {
		t.Fatal("PrincipalLifecycle drifted from the canonical transition contract")
	}
}

func TestPrincipalStatesPinTerminalVocabulary(t *testing.T) {
	if PrincipalStatePending != "pending" || PrincipalStateActive != "active" ||
		PrincipalStateSuspended != "suspended" || PrincipalStateDeleted != "deleted" {
		t.Fatal("principal lifecycle state vocabulary changed")
	}
	if PublicKeyBindingStateActive != "active" || PublicKeyBindingStateRotated != "rotated" ||
		PublicKeyBindingStateRevoked != "revoked" {
		t.Fatal("public-key binding state vocabulary changed")
	}
}
