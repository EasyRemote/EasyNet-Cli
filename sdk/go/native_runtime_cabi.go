//go:build easynet_cabi && cgo && !windows

package easynet

import "context"

// OpenNativeRuntime opens the SDK-owned native daemon Runtime provider.
func OpenNativeRuntime(ctx context.Context, options NativeRuntimeOptions) (*NativeRuntimeHandle, error) {
	if ctx == nil {
		return nil, invalidRuntimeClient("context is required")
	}
	transport, err := OpenCABIDaemonTransport(options.LibraryPath)
	if err != nil {
		return nil, err
	}
	control, err := NewDaemonControl(transport)
	if err != nil {
		_ = transport.Close(ctx)
		return nil, err
	}
	client, err := control.ConnectLocal(ctx, ConnectOptions{
		Endpoint:        options.Endpoint,
		ControlPath:     options.ControlPath,
		DialTimeoutMS:   options.DialTimeoutMS,
		InvokeTimeoutMS: options.InvokeTimeoutMS,
		MaxMessageBytes: options.MaxMessageBytes,
	})
	if err != nil {
		_ = transport.Close(ctx)
		return nil, err
	}
	return newNativeRuntimeHandle(client, transport.Close)
}
