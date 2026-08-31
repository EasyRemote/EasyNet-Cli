# Architecture

The EasyNet-Cli daemon continues to own RemoteApp plugin lifecycle. On macOS it
resolves the sibling application bundle and launches it through LaunchServices:

```text
<daemon-dir>/easynet-remoteapp-media-host.app
  Contents/Info.plist
  Contents/MacOS/easynet-remoteapp-media-host
```

The daemon creates a private Unix socket in a mode-0700 temporary directory.
The LaunchServices-started app connects once and receives the eight bounded
stdio, liveness, notification, and shared-media descriptors with `SCM_RIGHTS`.
It then enters the existing framed media-host protocol while the main thread
pumps the AppKit run loop required by application/TCC lifecycle.

Release signing binds the bundle identifier to the Developer ID designated
requirement after final strip mutation. Other platforms retain the flat binary
package layout.
