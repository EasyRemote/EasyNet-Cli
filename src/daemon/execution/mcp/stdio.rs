//! EasyNet-owned MCP stdio transport shared across product integration edges.
//!
//! MCP is a product edge adapter: it projects daemon abilities as tools and
//! translates tool calls back into daemon invocation. Axon remains responsible
//! for invocation and transport protocol semantics, not the MCP product API.

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};

use anyhow::anyhow;
use serde_json::{json, Map, Value};

type Result<T> = anyhow::Result<T>;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_SERVER_NAME: &str = "easynet-axon-remote-rust";
const DEFAULT_SERVER_VERSION: &str = "0.2.0";
const MAX_LINE_LENGTH: usize = 4 * 1024 * 1024;
const DEFAULT_MAX_STREAM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundedLineRead {
    Eof,
    Line,
    TooLong,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    max_length: usize,
) -> io::Result<BoundedLineRead> {
    line.clear();
    let mut saw_input = false;
    let mut too_long = false;
    let mut pending_cr = false;

    fn append_bounded(line: &mut Vec<u8>, bytes: &[u8], max_length: usize, too_long: &mut bool) {
        if *too_long {
            return;
        }
        let copied = bytes.len().min(max_length.saturating_sub(line.len()));
        line.extend_from_slice(&bytes[..copied]);
        *too_long = copied < bytes.len();
    }

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if pending_cr {
                append_bounded(line, b"\r", max_length, &mut too_long);
            }
            return Ok(if !saw_input {
                BoundedLineRead::Eof
            } else if too_long {
                BoundedLineRead::TooLong
            } else {
                BoundedLineRead::Line
            });
        }

        saw_input = true;
        if pending_cr {
            if available[0] == b'\n' {
                reader.consume(1);
                return Ok(if too_long {
                    BoundedLineRead::TooLong
                } else {
                    BoundedLineRead::Line
                });
            }
            append_bounded(line, b"\r", max_length, &mut too_long);
            pending_cr = false;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let mut payload_len = match newline {
            Some(index) if index > 0 && available[index - 1] == b'\r' => index - 1,
            Some(index) => index,
            None => available.len(),
        };
        if newline.is_none() && available.last() == Some(&b'\r') {
            payload_len -= 1;
            pending_cr = true;
        }
        let consumed = newline.map_or(payload_len, |index| index + 1);
        append_bounded(line, &available[..payload_len], max_length, &mut too_long);
        let consumed = if newline.is_none() && pending_cr {
            available.len()
        } else {
            consumed
        };
        reader.consume(consumed);

        if newline.is_some() {
            return Ok(if too_long {
                BoundedLineRead::TooLong
            } else {
                BoundedLineRead::Line
            });
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ToolResult {
    pub payload: Value,
    pub is_error: bool,
}

pub(crate) trait McpToolStreamHandle {
    fn recv(&mut self) -> Result<Option<Vec<u8>>>;
    fn close(&mut self) -> Result<()>;
}

impl McpToolStreamHandle for easynet_axon::dendrite_bridge::OwnedDendriteServerStream {
    fn recv(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(Self::recv(self)?)
    }

    fn close(&mut self) -> Result<()> {
        Ok(Self::close(self)?)
    }
}

pub(crate) trait McpToolProvider {
    fn tool_specs(&self) -> Vec<Value>;
    fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult;

    fn handle_tool_call_stream(
        &self,
        _name: &str,
        _args: &Map<String, Value>,
    ) -> Result<Option<Box<dyn McpToolStreamHandle>>> {
        Ok(None)
    }

    fn handle_tool_call_with_progress(
        &self,
        name: &str,
        args: &Map<String, Value>,
        _sink: &mut dyn ProgressSink,
    ) -> ToolResult {
        self.handle_tool_call(name, args)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportOutcome {
    Emitted,
    Throttled,
    RejectedNonIncreasing,
    RejectedNonFinite,
}

pub(crate) trait ProgressSink {
    fn report(
        &mut self,
        progress: f64,
        total: Option<f64>,
        message: Option<&str>,
    ) -> Result<ReportOutcome>;
}

struct WriterProgressSink<'w, W: Write> {
    writer: &'w mut W,
    token: Value,
    min_interval: std::time::Duration,
    last_report_at: Option<std::time::Instant>,
    last_progress: Option<f64>,
}

impl<'w, W: Write> WriterProgressSink<'w, W> {
    const DEFAULT_MIN_INTERVAL_MS: u64 = 100;

    fn new(writer: &'w mut W, token: Value) -> Self {
        Self {
            writer,
            token,
            min_interval: std::time::Duration::from_millis(Self::DEFAULT_MIN_INTERVAL_MS),
            last_report_at: None,
            last_progress: None,
        }
    }

    #[cfg(test)]
    fn with_min_interval(mut self, interval: std::time::Duration) -> Self {
        self.min_interval = interval;
        self
    }
}

impl<W: Write> ProgressSink for WriterProgressSink<'_, W> {
    fn report(
        &mut self,
        progress: f64,
        total: Option<f64>,
        message: Option<&str>,
    ) -> Result<ReportOutcome> {
        if !progress.is_finite() || total.is_some_and(|value| !value.is_finite()) {
            return Ok(ReportOutcome::RejectedNonFinite);
        }
        if self
            .last_progress
            .is_some_and(|previous| progress <= previous)
        {
            return Ok(ReportOutcome::RejectedNonIncreasing);
        }
        if self
            .last_report_at
            .is_some_and(|previous| previous.elapsed() < self.min_interval)
        {
            return Ok(ReportOutcome::Throttled);
        }

        write_json_line(
            self.writer,
            &build_progress_frame(&self.token, progress, total, message),
        )?;
        self.last_report_at = Some(std::time::Instant::now());
        self.last_progress = Some(progress);
        Ok(ReportOutcome::Emitted)
    }
}

fn extract_progress_token(params: &Map<String, Value>) -> Option<Value> {
    params
        .get("_meta")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("progressToken"))
        .cloned()
}

fn build_progress_frame(
    token: &Value,
    progress: f64,
    total: Option<f64>,
    message: Option<&str>,
) -> Value {
    let mut params = Map::new();
    params.insert("progressToken".to_string(), token.clone());
    params.insert(
        "progress".to_string(),
        Value::Number(
            serde_json::Number::from_f64(progress)
                .expect("caller contract: progress must be finite (see report() boundary guard)"),
        ),
    );
    if let Some(total) = total {
        params.insert(
            "total".to_string(),
            Value::Number(
                serde_json::Number::from_f64(total)
                    .expect("caller contract: total must be finite (see report() boundary guard)"),
            ),
        );
    }
    if let Some(message) = message {
        params.insert("message".to_string(), Value::String(message.to_string()));
    }
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": params,
    })
}

pub(crate) struct StdioMcpServer<T> {
    provider: T,
    protocol_version: String,
    server_name: String,
    server_version: String,
}

impl<T> StdioMcpServer<T> {
    pub(crate) fn new(provider: T) -> Self {
        Self {
            provider,
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_string(),
            server_name: DEFAULT_SERVER_NAME.to_string(),
            server_version: DEFAULT_SERVER_VERSION.to_string(),
        }
    }

    pub(crate) fn with_server_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.server_name = name;
        }
        self
    }

    pub(crate) fn with_server_version(mut self, version: impl Into<String>) -> Self {
        let version = version.into();
        if !version.trim().is_empty() {
            self.server_version = version;
        }
        self
    }

    pub(crate) fn run<R, W>(&self, input: R, output: &mut W) -> Result<()>
    where
        T: McpToolProvider,
        R: io::Read,
        W: Write,
    {
        let mut input = BufReader::new(input);
        let mut line = Vec::with_capacity(8 * 1024);

        loop {
            match read_bounded_line(&mut input, &mut line, MAX_LINE_LENGTH)
                .map_err(|error| anyhow!("mcp: read stdin failed: {error}"))?
            {
                BoundedLineRead::Eof => return Ok(()),
                BoundedLineRead::TooLong => {
                    write_json_line(
                        output,
                        &jsonrpc_error(
                            Value::Null,
                            -32600,
                            &format!("input line exceeds maximum length ({MAX_LINE_LENGTH} bytes)"),
                        ),
                    )?;
                    continue;
                }
                BoundedLineRead::Line => {}
            }
            let line = std::str::from_utf8(&line)
                .map_err(|error| anyhow!("mcp: stdin is not valid UTF-8: {error}"))?;
            if let Some(response) = self.handle_raw_line(line, output)? {
                write_json_line(output, &response)?;
            }
        }
    }

    fn handle_raw_line<W: Write>(&self, raw: &str, output: &mut W) -> Result<Option<Value>>
    where
        T: McpToolProvider,
    {
        let request = match parse_json_line(raw) {
            Ok(Some(value)) => value,
            Ok(None) => return Ok(None),
            Err(_) => return Ok(Some(jsonrpc_error(Value::Null, -32700, "parse error"))),
        };

        if !request.is_object() {
            return Ok(Some(jsonrpc_error(Value::Null, -32600, "invalid request")));
        }
        let response = self.handle_request(request, output);
        Ok((!response.is_null()).then_some(response))
    }

    fn handle_request<W: Write>(&self, request: Value, output: &mut W) -> Value
    where
        T: McpToolProvider,
    {
        let object = match request {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        let has_id = object.contains_key("id");
        let method = as_string(object.get("method"));
        let params = match object.get("params") {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };

        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") && has_id {
            return jsonrpc_error(id, -32600, "invalid request: missing jsonrpc version");
        }

        match method.as_str() {
            "notifications/initialized" => Value::Null,
            "initialize" => {
                if !has_id {
                    return Value::Null;
                }
                let negotiated = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .filter(|version| valid_protocol_version(version))
                    .map(str::to_string)
                    .unwrap_or_else(|| self.protocol_version.clone());
                jsonrpc_success(
                    id,
                    json!({
                        "protocolVersion": negotiated,
                        "capabilities": {"tools": {}},
                        "serverInfo": {
                            "name": self.server_name,
                            "version": self.server_version,
                        }
                    }),
                )
            }
            "tools/list" => {
                if !has_id {
                    return Value::Null;
                }
                jsonrpc_success(id, json!({"tools": self.provider.tool_specs()}))
            }
            "tools/call" => {
                if !has_id {
                    return Value::Null;
                }
                let name = as_string(params.get("name"));
                if name.trim().is_empty() {
                    return jsonrpc_error(id, -32602, "tool name is required");
                }
                let arguments = match params.get("arguments") {
                    Some(Value::Object(raw)) => raw.clone(),
                    _ => Map::new(),
                };

                match self.provider.handle_tool_call_stream(&name, &arguments) {
                    Ok(Some(handle)) => {
                        let max_bytes = resolve_max_bytes(arguments.get("max_bytes"));
                        return jsonrpc_success(
                            id.clone(),
                            tool_payload(stream_to_client(handle, &id, output, max_bytes)),
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return jsonrpc_success(
                            id,
                            tool_payload(ToolResult {
                                payload: json!({"ok": false, "error": error.to_string()}),
                                is_error: true,
                            }),
                        );
                    }
                }

                let response = match extract_progress_token(&params) {
                    Some(token) => {
                        let mut sink = WriterProgressSink::new(output, token);
                        self.provider
                            .handle_tool_call_with_progress(&name, &arguments, &mut sink)
                    }
                    None => self.provider.handle_tool_call(&name, &arguments),
                };
                jsonrpc_success(id, tool_payload(response))
            }
            "ping" => {
                if !has_id {
                    return Value::Null;
                }
                jsonrpc_success(id, json!({}))
            }
            _ => {
                if !has_id {
                    return Value::Null;
                }
                jsonrpc_error(id, -32601, &format!("method not found: {method}"))
            }
        }
    }
}

fn valid_protocol_version(version: &str) -> bool {
    if version.len() != 10 {
        return false;
    }
    let bytes = version.as_bytes();
    let structural = bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if !structural {
        return false;
    }
    let month: u8 = version[5..7].parse().unwrap_or(0);
    let day: u8 = version[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn resolve_max_bytes(raw: Option<&Value>) -> usize {
    match raw.and_then(Value::as_u64) {
        Some(value) if value > 0 => value as usize,
        _ => DEFAULT_MAX_STREAM_BYTES,
    }
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GiB", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

struct StreamProcessResult {
    chunk_count: u64,
    had_error: Option<String>,
    had_invalid_utf8: bool,
}

fn process_stream(
    handle: &mut dyn McpToolStreamHandle,
    max_bytes: usize,
    mut on_chunk: impl FnMut(&str, u64) -> Result<()>,
) -> StreamProcessResult {
    let mut chunk_count = 0;
    let mut total_bytes = 0usize;
    let mut had_error = None;
    let mut had_invalid_utf8 = false;

    loop {
        match handle.recv() {
            Ok(Some(bytes)) => {
                let decoded = String::from_utf8_lossy(&bytes);
                if !had_invalid_utf8
                    && !bytes.is_empty()
                    && matches!(decoded, std::borrow::Cow::Owned(_))
                {
                    had_invalid_utf8 = true;
                }
                total_bytes = total_bytes.saturating_add(bytes.len());
                if total_bytes > max_bytes {
                    had_error = Some(format!(
                        "stream output exceeded {} limit",
                        format_bytes(max_bytes)
                    ));
                    break;
                }
                if let Err(error) = on_chunk(&decoded, chunk_count) {
                    had_error = Some(error.to_string());
                    break;
                }
                chunk_count += 1;
            }
            Ok(None) => break,
            Err(error) => {
                had_error = Some(error.to_string());
                break;
            }
        }
    }

    StreamProcessResult {
        chunk_count,
        had_error,
        had_invalid_utf8,
    }
}

fn stream_to_client<W: Write>(
    mut handle: Box<dyn McpToolStreamHandle>,
    request_id: &Value,
    output: &mut W,
    max_bytes: usize,
) -> ToolResult {
    let result = process_stream(handle.as_mut(), max_bytes, |decoded, sequence| {
        write_json_line(
            output,
            &jsonrpc_notification(
                "axon/streamChunk",
                json!({
                    "requestId": request_id,
                    "seq": sequence,
                    "chunk": decoded,
                }),
            ),
        )
        .map_err(|error| anyhow!("mcp: write notification failed: {error}"))
    });

    if let Err(error) = handle.close() {
        crate::op_event!(
            component = mcp_stdio,
            kind = stream_close_failed,
            error = error,
        );
    }
    let mut summary = json!({
        "ok": result.had_error.is_none(),
        "chunk_count": result.chunk_count,
        "streamed": true,
    });
    if let Some(error) = &result.had_error {
        summary["error"] = json!(error);
    }
    if result.had_invalid_utf8 {
        summary["contains_invalid_utf8"] = json!(true);
    }
    ToolResult {
        payload: summary,
        is_error: result.had_error.is_some(),
    }
}

fn write_json_line<W: Write>(output: &mut W, payload: &Value) -> Result<()> {
    let serialized = serde_json::to_string(payload)
        .map_err(|error| anyhow!("mcp: serialize response failed: {error}"))?;
    output
        .write_all(serialized.as_bytes())
        .map_err(|error| anyhow!("mcp: write response failed: {error}"))?;
    output
        .write_all(b"\n")
        .map_err(|error| anyhow!("mcp: write response failed: {error}"))?;
    output
        .flush()
        .map_err(|error| anyhow!("mcp: flush response failed: {error}"))?;
    Ok(())
}

fn parse_json_line(raw: &str) -> std::result::Result<Option<Value>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(text)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn tool_payload(result: ToolResult) -> Value {
    let content = serde_json::to_string(&result.payload).unwrap_or_else(|_| {
        json!({"ok": false, "error": "tool response serialization failed"}).to_string()
    });
    let mut payload = Map::new();
    payload.insert(
        "content".to_string(),
        json!([{"type": "text", "text": content}]),
    );
    if result.is_error {
        payload.insert("isError".to_string(), json!(true));
    }
    Value::Object(payload)
}

fn jsonrpc_success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn jsonrpc_notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    })
}

fn as_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(raw)) => raw.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

impl<T> fmt::Debug for StdioMcpServer<T>
where
    T: McpToolProvider + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StdioMcpServer")
            .field("protocol_version", &self.protocol_version)
            .field("server_name", &self.server_name)
            .field("server_version", &self.server_version)
            .field("provider", &self.provider)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[derive(Debug)]
    struct Provider;

    impl McpToolProvider for Provider {
        fn tool_specs(&self) -> Vec<Value> {
            vec![json!({"name": "health", "inputSchema": {"type": "object"}})]
        }

        fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult {
            ToolResult {
                payload: json!({"name": name, "args": args}),
                is_error: false,
            }
        }
    }

    fn run(input: &str) -> Vec<Value> {
        let mut output = Vec::new();
        StdioMcpServer::new(Provider)
            .run(input.as_bytes(), &mut output)
            .unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn initialize_and_tool_call_preserve_mcp_wire_shape() {
        let frames = run(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n\
             {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"health\",\"arguments\":{\"probe\":true}}}\n",
        );
        assert_eq!(frames[0]["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(frames[0]["result"]["capabilities"], json!({"tools": {}}));
        assert_eq!(frames[1]["result"]["content"][0]["type"], "text");
        let text = frames[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({"name": "health", "args": {"probe": true}})
        );
    }

    #[test]
    fn progress_sink_rejects_invalid_values_without_advancing_state() {
        let mut output = Vec::new();
        let mut sink =
            WriterProgressSink::new(&mut output, json!("token")).with_min_interval(Duration::ZERO);

        assert_eq!(
            sink.report(f64::NAN, Some(1.0), None).unwrap(),
            ReportOutcome::RejectedNonFinite
        );
        assert_eq!(
            sink.report(0.5, Some(1.0), Some("working")).unwrap(),
            ReportOutcome::Emitted
        );
        assert_eq!(
            sink.report(0.5, Some(1.0), None).unwrap(),
            ReportOutcome::RejectedNonIncreasing
        );
        assert_eq!(String::from_utf8(output).unwrap().lines().count(), 1);
    }

    #[test]
    fn malformed_requests_keep_jsonrpc_error_codes() {
        let frames = run("not-json\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"missing\"}\n");
        assert_eq!(frames[0]["error"]["code"], -32700);
        assert_eq!(frames[1]["error"]["code"], -32601);
    }

    #[test]
    fn oversized_frame_is_discarded_before_the_next_request() {
        let oversized = format!(
            "{}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n",
            "x".repeat(MAX_LINE_LENGTH + 32)
        );
        let frames = run(&oversized);
        assert_eq!(frames[0]["error"]["code"], -32600);
        assert_eq!(frames[1]["id"], 2);
        assert!(frames[1]["result"].is_object());
    }

    #[test]
    fn bounded_reader_never_retains_more_than_the_declared_limit() {
        let input = format!("{}\n", "x".repeat(4096));
        let mut reader = BufReader::with_capacity(64, input.as_bytes());
        let mut line = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut line, 128).unwrap(),
            BoundedLineRead::TooLong
        );
        assert_eq!(line.len(), 128);
    }

    #[test]
    fn bounded_reader_rejects_oversized_eof_frame_without_retaining_extra_bytes() {
        let input = "x".repeat(4096);
        let mut reader = BufReader::with_capacity(64, input.as_bytes());
        let mut line = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut line, 128).unwrap(),
            BoundedLineRead::TooLong
        );
        assert_eq!(line.len(), 128);
    }

    #[test]
    fn bounded_reader_accepts_payload_at_limit_and_rejects_one_byte_over() {
        for (payload_len, expected) in [
            (127, BoundedLineRead::Line),
            (128, BoundedLineRead::Line),
            (129, BoundedLineRead::TooLong),
        ] {
            let input = format!("{}\n", "x".repeat(payload_len));
            let mut reader = BufReader::with_capacity(17, input.as_bytes());
            let mut line = Vec::new();
            assert_eq!(
                read_bounded_line(&mut reader, &mut line, 128).unwrap(),
                expected
            );
            assert_eq!(line.len(), payload_len.min(128));
        }
    }

    #[test]
    fn bounded_reader_handles_eof_chunking_and_crlf_without_retaining_delimiters() {
        let eof_input = "x".repeat(128);
        let mut eof_reader = BufReader::with_capacity(7, eof_input.as_bytes());
        let mut line = Vec::new();
        assert_eq!(
            read_bounded_line(&mut eof_reader, &mut line, 128).unwrap(),
            BoundedLineRead::Line
        );
        assert_eq!(line.len(), 128);

        let input = format!("{}\r\nnext\n", "y".repeat(128));
        // Capacity 129 forces the CR and LF onto different fill_buf chunks.
        let mut chunked_reader = BufReader::with_capacity(129, input.as_bytes());
        assert_eq!(
            read_bounded_line(&mut chunked_reader, &mut line, 128).unwrap(),
            BoundedLineRead::Line
        );
        assert_eq!(line, vec![b'y'; 128]);
        assert_eq!(
            read_bounded_line(&mut chunked_reader, &mut line, 128).unwrap(),
            BoundedLineRead::Line
        );
        assert_eq!(line, b"next");

        let cr_payload = "z".repeat(127) + "\r";
        let mut cr_eof_reader = BufReader::with_capacity(13, cr_payload.as_bytes());
        assert_eq!(
            read_bounded_line(&mut cr_eof_reader, &mut line, 128).unwrap(),
            BoundedLineRead::Line
        );
        assert_eq!(line, cr_payload.as_bytes());
    }
}
