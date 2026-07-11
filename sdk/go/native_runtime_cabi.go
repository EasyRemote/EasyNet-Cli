//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// OpenNativeRuntime opens the SDK-owned native daemon Runtime provider.
func OpenNativeRuntime(ctx context.Context, options NativeRuntimeOptions) (*NativeRuntimeHandle, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	transport, err := OpenCABIDaemonTransport(options.LibraryPath)
	if err != nil {
		return nil, err
	}
	runtime, health, err := connectNativeRuntime(ctx, transport, options)
	if err != nil {
		_ = transport.Close(ctx)
		return nil, err
	}
	identity, identityTransport, err := NewCABIIdentityClient(options.LibraryPath, options.ControlPath)
	if err != nil {
		_ = runtime.Close(ctx)
		_ = transport.Close(ctx)
		return nil, err
	}
	handle, err := newNativeRuntimeHandleWithIdentity(runtime, health, identity, transport.Close)
	if err != nil {
		_ = identityTransport.Close(ctx)
		_ = runtime.Close(ctx)
		_ = transport.Close(ctx)
		return nil, err
	}
	return handle, nil
}

func connectNativeRuntime(ctx context.Context, transport DaemonTransport, options NativeRuntimeOptions) (*RuntimeClient, *HealthClient, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return nil, nil, err
	}
	endpoints, err := control.Discover(ctx, DiscoverOptions{ControlPath: options.ControlPath})
	if err != nil {
		return nil, nil, err
	}
	runtimeEndpoint := endpoints.InvocationEndpoint
	if options.Endpoint != "" {
		runtimeEndpoint = options.Endpoint
	}
	if runtimeEndpoint == "" {
		return nil, nil, invalidRuntimePayload("invocation_endpoint is required", nil)
	}
	handle, err := control.Attach(ctx, AttachOptions{
		ControlEndpoint:    endpoints.ControlEndpoint,
		InvocationEndpoint: runtimeEndpoint,
		ControlPath:        options.ControlPath,
	})
	if err != nil {
		return nil, nil, err
	}
	openOptions := ConnectOptions{
		Endpoint:        runtimeEndpoint,
		ControlPath:     options.ControlPath,
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
	}
	optionsJSON, err := json.Marshal(openOptions)
	if err != nil {
		_ = handle.Detach(ctx)
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode runtime options: %v", err), err)
	}
	runtimeTransport, _, err := handle.transport.OpenRuntime(ctx, handle.handleID, optionsJSON)
	detachErr := handle.Detach(ctx)
	if err != nil {
		return nil, nil, errors.Join(wrapDaemonTransportError("daemon open runtime failed", err), detachErr)
	}
	if detachErr != nil {
		return nil, nil, detachErr
	}
	if runtimeTransport == nil {
		return nil, nil, invalidRuntimeClient("runtime transport is required")
	}
	healthTransport, ok := runtimeTransport.(HealthTransport)
	if !ok {
		_ = runtimeTransport.Close(ctx)
		return nil, nil, invalidRuntimeClient("native runtime transport must expose health")
	}
	health, err := NewHealthClient(healthTransport)
	if err != nil {
		_ = runtimeTransport.Close(ctx)
		return nil, nil, err
	}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		_ = runtimeTransport.Close(ctx)
		return nil, nil, err
	}
	return runtime, health, nil
}
