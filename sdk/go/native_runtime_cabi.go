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

func connectNativeRuntime(ctx context.Context, transport DaemonTransport, options NativeRuntimeOptions) (*RuntimeClient, *HealthClient, func(context.Context) error, error) {
	control, err := NewDaemonControl(transport)
	if err != nil {
		return nil, nil, nil, err
	}
	if options.StartConfig != nil {
		return startNativeRuntime(ctx, control, transport, options)
	}
	endpoints, err := control.Discover(ctx, DiscoverOptions{ControlPath: options.ControlPath})
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
	handle, err := control.Attach(ctx, AttachOptions{
		ControlEndpoint:    endpoints.ControlEndpoint,
		InvocationEndpoint: runtimeEndpoint,
		ControlPath:        options.ControlPath,
	})
	if err != nil {
		return nil, nil, nil, err
	}
	runtime, health, err := openNativeRuntimeClients(ctx, handle, nativeConnectOptions(options, runtimeEndpoint))
	detachErr := handle.Detach(ctx)
	if err != nil {
		return nil, nil, nil, errors.Join(err, detachErr)
	}
	if detachErr != nil {
		return nil, nil, nil, detachErr
	}
	return runtime, health, func(context.Context) error { return nil }, nil
}

func startNativeRuntime(ctx context.Context, control *DaemonControl, _ DaemonTransport, options NativeRuntimeOptions) (*RuntimeClient, *HealthClient, func(context.Context) error, error) {
	startConfig := *options.StartConfig
	handle, err := control.Start(ctx, startConfig)
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
	runtime, health, err := openNativeRuntimeClients(ctx, handle, nativeConnectOptions(options, runtimeEndpoint))
	if err != nil {
		_ = handle.Detach(ctx)
		return nil, nil, nil, err
	}
	if options.StopDaemonOnClose {
		return runtime, health, func(closeCtx context.Context) error {
			return handle.Stop(closeCtx, StopOptions{})
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

func openNativeRuntimeClients(ctx context.Context, handle *DaemonHandle, openOptions ConnectOptions) (*RuntimeClient, *HealthClient, error) {
	optionsJSON, err := json.Marshal(openOptions)
	if err != nil {
		return nil, nil, invalidRuntimePayload(fmt.Sprintf("encode runtime options: %v", err), err)
	}
	runtimeTransport, _, err := handle.transport.OpenRuntime(ctx, handle.handleID, optionsJSON)
	if err != nil {
		return nil, nil, wrapDaemonTransportError("daemon open runtime failed", err)
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
