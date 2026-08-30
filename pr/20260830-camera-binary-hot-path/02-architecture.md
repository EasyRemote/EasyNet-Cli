# Architecture

```text
AVFoundation VideoDataOutput
  -> JPEG Arc<[u8]>
  -> watch latest-value slot (capacity one logical frame)
  -> typed bounded Runtime carrier (capacity one)
  -> Axon progress payload bytes + content type
  -> SDK v8 raw frame

AVFoundation VideoDataOutput (native NV12 / 420v)
  -> AVAssetWriterInput (realtime, no intermediate queue)
  -> system H.264 encoder
  -> AVAssetWriter QuickTime finalization
  -> finalized temporary MOV
  -> Context same-volume atomic commit
  -> append capture index
  -> stop receipt
```

The media module owns capture/encoding/drop policy. `StreamSource` owns only
the typed bounded adapter into Runtime. Axon remains the sole owner of stream
lifecycle. Context persistence owns artifact naming and durable publication.

Still capture is a separate native state machine:

```text
AVCaptureDevice configuration lock
  -> supported center-point continuous AF / AE / AWB
  -> bounded 3A warm-up and stability observation
  -> AVCapturePhotoOutput (Balanced quality)
  -> native JPEG delegate terminal callback
  -> typed unary Runtime result (`image/jpeg`, exact JPEG bytes)
  -> signed terminal receipt and gRPC `InvokeResponse.result`
```

Unsupported controls are skipped explicitly, which is required for fixed-focus
Mac cameras; no unsupported mode is forced onto the device.

Capture metadata that is needed after invocation is persisted in Context and
identified by the request/subject. It is not wrapped around the JPEG on the
data plane: the Runtime result already carries the media type and the receipt
binds the exact bytes.

Asymptotic bounds per stream are O(1) queued frames and O(P) unavoidable JPEG
work for P pixels. The strided BGRA encoder reads the locked pixel buffer
directly and does not allocate a second full RGB frame. Recording appends native
NV12 sample buffers directly to the system encoder; Runtime-owned encoder state
is bounded independently of movie duration, and recording stop adds O(1)
media-buffer memory rather than O(movie size).
