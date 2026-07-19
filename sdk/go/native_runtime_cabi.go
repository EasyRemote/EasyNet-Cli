//go:build easynet_cabi && cgo && !windows

package easynet

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// OpenNativeRuntime opens the SDK-owned native runtime provider.
func OpenNativeRuntime(ctx context.Context, options NativeRuntimeOptions) (*NativeRuntimeHandle, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	transport, err := OpenCABIRuntimeLifecycleTransport(options.LibraryPath)
	if err != nil {
		return nil, err
	}
	runtime, health, lifecycleClose, err := connectNativeRuntime(ctx, transport, options)
	if err != nil {
		_ = transport.Close(ctx)
		return nil, err
	}
	closeFn := func(closeCtx context.Context) error {
		return errors.Join(lifecycleClose(closeCtx), transport.Close(closeCtx))
	}
	handle, err := newNativeRuntimeHandle(runtime, health, closeFn)
	if err != nil {
		_ = runtime.Close(ctx)
		_ = closeFn(ctx)
		return nil, err
	}
	return handle, nil
}

func connectNativeRuntime(ctx context.Context, transport RuntimeLifecycleTransport, options NativeRuntimeOptions) (*RuntimeClient, *HealthClient, func(context.Context) error, error) {
	host, err := NewRuntimeHost(transport)
	if err != nil {
		return nil, nil, nil, err
	}
	if options.StartRequest != nil {
		return startNativeRuntime(ctx, host, options)
	}
	endpoints, err := host.DiscoverRuntime(ctx, RuntimeHostDiscoverOptions{ControlPath: options.ControlPath})
	if err != nil {
		return nil, nil, nil, err
	}
	runtimeEndpoint := endpoints.InvocationEndpoint
	if options.Endpoint != "" {
		runtimeEndpoint = options.Endpoint
	}
	if runtimeEndpoint == "" {
		return nil, nil, nil, invalidRuntimePayload("invocation_endpoint is required", nil)
	}
	handle, err := host.AttachRuntime(ctx, RuntimeHostAttachOptions{
		ControlEndpoint:    endpoints.ControlEndpoint,
		InvocationEndpoint: runtimeEndpoint,
		ControlPath:        options.ControlPath,
	})
	if err != nil {
		return nil, nil, nil, err
	}
	runtime, health, err := openNativeRuntimeClients(ctx, handle, nativeConnectOptions(options, runtimeEndpoint), options.Signer)
	detachErr := handle.Detach(ctx)
	if err != nil {
		return nil, nil, nil, errors.Join(err, detachErr)
	}
	if detachErr != nil {
		return nil, nil, nil, detachErr
	}
	return runtime, health, func(context.Context) error { return nil }, nil
}

func startNativeRuntime(ctx context.Context, host *RuntimeHost, options NativeRuntimeOptions) (*RuntimeClient, *HealthClient, func(context.Context) error, error) {
	handle, err := host.StartRuntime(ctx, options.StartRequest)
	if err != nil {
		return nil, nil, nil, err
	}
	runtimeEndpoint := handle.Endpoints().InvocationEndpoint
	if options.Endpoint != "" {
		runtimeEndpoint = options.Endpoint
	}
	if runtimeEndpoint == "" {
		_ = handle.Detach(ctx)
		return nil, nil, nil, invalidRuntimePayload("invocation_endpoint is required", nil)
	}
	runtime, health, err := openNativeRuntimeClients(ctx, handle, nativeConnectOptions(options, runtimeEndpoint), options.Signer)
	if err != nil {
		_ = handle.Detach(ctx)
		return nil, nil, nil, err
	}
	if options.StopDaemonOnClose {
		return runtime, health, func(closeCtx context.Context) error {
			return handle.StopRuntime(closeCtx, RuntimeHostStopOptions{})
		}, nil
	}
	detachErr := handle.Detach(ctx)
	if detachErr != nil {
		_ = runtime.Close(ctx)
		return nil, nil, nil, detachErr
	}
	return runtime, health, func(context.Context) error { return nil }, nil
}

func nativeConnectOptions(options NativeRuntimeOptions, runtimeEndpoint string) ConnectOptions {
	return ConnectOptions{
		Endpoint:        runtimeEndpoint,
		ControlPath:     options.ControlPath,
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
	}
}

func openNativeRuntimeClients(ctx context.Context, handle *RuntimeHandle, openOptions ConnectOptions, signer *Signer) (*RuntimeClient, *HealthClient, error) {
	optionsJSON, err := json.Marshal(openOptions)
	if err != nil {
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode runtime options: %v", err), err)
	}
	runtimeTransport, _, err := handle.transport.OpenRuntime(ctx, handle.handleID, optionsJSON)
	if err != nil {
		return nil, nil, wrapRuntimeLifecycleTransportError("daemon open runtime failed", err)
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
	if signer != nil {
		runtimeTransport, err = NewRuntimeSigningTransport(runtimeTransport, *signer)
		if err != nil {
			_ = runtimeTransport.Close(ctx)
			return nil, nil, err
		}
	}
	runtime, err := NewRuntimeClient(runtimeTransport)
	if err != nil {
		_ = runtimeTransport.Close(ctx)
		return nil, nil, err
	}
	return runtime, health, nil
}
