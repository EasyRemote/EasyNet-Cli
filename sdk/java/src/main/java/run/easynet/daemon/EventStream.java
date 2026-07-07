package run.easynet.daemon;

import java.util.Map;
import java.util.Objects;

public final class EventStream implements AutoCloseable {
  private final String stream;
  private final StreamHandle handle;
  private String state;
  private final String streamID;
  private final String resumeToken;
  private final Map<String, Object> metadata;

  public EventStream(String stream, StreamSource source) {
    this(stream, new StreamHandle(Objects.requireNonNull(source, "source")), "Live", "", "", Map.of("profile", EventsSupport.PROFILE));
  }

  public EventStream(
      String stream,
      StreamHandle handle,
      String state,
      String streamID,
      String resumeToken,
      Map<String, Object> metadata) {
    this.stream = EventsSupport.requiredStream(stream, "stream");
    this.handle = Objects.requireNonNull(handle, "handle");
    this.state = EventsSupport.cleanRequired(state, "state");
    this.streamID = streamID == null ? "" : streamID;
    this.resumeToken = resumeToken == null ? "" : resumeToken;
    this.metadata = EventsSupport.copyObject(metadata);
  }

  public EventFrame receive() {
    EventFrame frame =
        EventFrame.fromJSON(handle.next().payloadJson().getBytes(java.nio.charset.StandardCharsets.UTF_8));
    if (frame.terminal()) {
      state = "Terminal";
    }
    return frame;
  }

  public String stream() {
    return stream;
  }

  public String state() {
    return state;
  }

  public String streamID() {
    return streamID;
  }

  public String resumeToken() {
    return resumeToken;
  }

  public Map<String, Object> metadata() {
    return metadata;
  }

  public StreamHandle handle() {
    return handle;
  }

  public void cancel(String reason) {
    handle.cancel(reason);
    state = "Cancelled";
  }

  @Override
  public void close() {
    handle.close();
    state = "Closed";
  }
}
