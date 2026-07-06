package easynet

import (
	"context"
	"testing"
)

func TestRuntimeProfileBundleBuildsRuntimeBackedClients(t *testing.T) {
	ctx := context.Background()
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identity, err := NewIdentityClient(IdentityTransportFunc{})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	bundle, err := NewRuntimeProfileBundle(runtime, identity, RuntimeProfileBundleOptions{})
	if err != nil {
		t.Fatalf("NewRuntimeProfileBundle: %v", err)
	}

	if got, err := bundle.Runtime(ctx); err != nil || got != runtime {
		t.Fatalf("Runtime() = %v, %v", got, err)
	}
	if got, err := bundle.Identity(ctx); err != nil || got != identity {
		t.Fatalf("Identity() = %v, %v", got, err)
	}
	if _, err := bundle.Directory(ctx); err != nil {
		t.Fatalf("Directory: %v", err)
	}
	if _, err := bundle.Receipts(ctx); err != nil {
		t.Fatalf("Receipts: %v", err)
	}
	if _, err := bundle.Publication(ctx); err != nil {
		t.Fatalf("Publication: %v", err)
	}
	if _, err := bundle.Admin(ctx); err != nil {
		t.Fatalf("Admin: %v", err)
	}
	if _, err := bundle.Events(ctx); err != nil {
		t.Fatalf("Events: %v", err)
	}
	if _, err := bundle.Missions(ctx); err != nil {
		t.Fatalf("Missions: %v", err)
	}
	if _, err := bundle.Surface(ctx); err != nil {
		t.Fatalf("Surface: %v", err)
	}
	if _, err := bundle.Compatibility(ctx); err != nil {
		t.Fatalf("Compatibility: %v", err)
	}
	if _, err := bundle.Wrappers(ctx); err != nil {
		t.Fatalf("Wrappers: %v", err)
	}
	if err := bundle.Close(ctx); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if _, err := bundle.Directory(ctx); err == nil {
		t.Fatal("Directory after close succeeded")
	}
	if err := bundle.Close(ctx); err != nil {
		t.Fatalf("second Close: %v", err)
	}
}

func TestRuntimeProfileBundleOwnsRuntimeWhenRequested(t *testing.T) {
	ctx := context.Background()
	closeCalls := 0
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{
		CloseFunc: func(context.Context) error {
			closeCalls++
			return nil
		},
	})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identity, err := NewIdentityClient(IdentityTransportFunc{})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	bundle, err := NewRuntimeProfileBundle(runtime, identity, RuntimeProfileBundleOptions{OwnRuntime: true})
	if err != nil {
		t.Fatalf("NewRuntimeProfileBundle: %v", err)
	}
	if err := bundle.Close(ctx); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if closeCalls != 1 {
		t.Fatalf("runtime close calls = %d, want 1", closeCalls)
	}
	if err := bundle.Close(ctx); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if closeCalls != 1 {
		t.Fatalf("runtime close calls after second close = %d, want 1", closeCalls)
	}
}

func TestRuntimeProfileBundleRejectsNilContext(t *testing.T) {
	runtime, err := NewRuntimeClient(RuntimeTransportFunc{})
	if err != nil {
		t.Fatalf("NewRuntimeClient: %v", err)
	}
	identity, err := NewIdentityClient(IdentityTransportFunc{})
	if err != nil {
		t.Fatalf("NewIdentityClient: %v", err)
	}
	bundle, err := NewRuntimeProfileBundle(runtime, identity, RuntimeProfileBundleOptions{})
	if err != nil {
		t.Fatalf("NewRuntimeProfileBundle: %v", err)
	}
	if _, err := bundle.Directory(nil); err == nil {
		t.Fatal("Directory accepted nil context")
	}
	if err := bundle.Close(nil); err == nil {
		t.Fatal("Close accepted nil context")
	}
}
