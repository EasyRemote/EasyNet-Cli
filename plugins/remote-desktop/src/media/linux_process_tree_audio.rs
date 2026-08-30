// EasyNet CLI — Linux RemoteApp process-tree audio backend
// ========================================================
//
// File: plugins/remote-desktop/src/media/linux_process_tree_audio.rs
// Description: PipeWire fan-in capture for every output node owned by one
// Linux process tree.
//
// Protocol Responsibility:
// - None. The RemoteDesktop session aggregate owns target authority,
//   transport epochs, rebind and terminal lifecycle.
//
// Implementation Approach:
// - Anchor the selected root PID to its kernel start time and derive the live
//   descendant set from `/proc`.
// - Resolve PipeWire output nodes through Client `pipewire.sec.pid`, then keep
//   an explicit link-factory fan-in for every eligible node.
// - Implement flexaudio's `CaptureBackend`; its existing bounded ring,
//   normalizer, watchdog and stream lifecycle remain the sole audio engine.
//
// Usage Contract:
// - The backend is Linux/native-media only.
// - PID reuse or an unreadable root identity yields an empty authorized set;
//   it never widens capture to the system mix.
// - Registry callbacks are panic-contained because they cross an FFI boundary.
//
// Architectural Position:
// - RemoteDesktop plugin Linux host adapter below `media/host_audio.rs` and
//   above PipeWire graph mechanics.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxProcessIdentity {
    pid: u32,
    start_time_ticks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxProcessRow {
    identity: LinuxProcessIdentity,
    parent_pid: u32,
}

fn parse_proc_stat(pid: u32, stat: &str) -> Option<LinuxProcessRow> {
    // `comm` is parenthesized and may itself contain spaces or `)`, so split
    // after its final close delimiter rather than using whitespace globally.
    let tail = stat.get(stat.rfind(')')?.saturating_add(1)..)?.trim();
    let fields = tail.split_ascii_whitespace().collect::<Vec<_>>();
    // tail[0] is field 3 (`state`), tail[1] is field 4 (`ppid`), and
    // tail[19] is field 22 (`starttime`).
    let parent_pid = fields.get(1)?.parse().ok()?;
    let start_time_ticks = fields.get(19)?.parse().ok()?;
    Some(LinuxProcessRow {
        identity: LinuxProcessIdentity {
            pid,
            start_time_ticks,
        },
        parent_pid,
    })
}

fn process_tree_from_rows(
    root: LinuxProcessIdentity,
    rows: impl IntoIterator<Item = LinuxProcessRow>,
) -> HashSet<u32> {
    let rows = rows.into_iter().collect::<Vec<_>>();
    if !rows.iter().any(|row| row.identity == root) {
        return HashSet::new();
    }

    let mut selected = HashSet::from([root.pid]);
    loop {
        let before = selected.len();
        for row in &rows {
            if selected.contains(&row.parent_pid) {
                selected.insert(row.identity.pid);
            }
        }
        if selected.len() == before {
            return selected;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinuxAudioNodeOwner {
    app_pid: Option<u32>,
    owning_client_id: Option<u32>,
}

fn eligible_audio_node_ids(
    allowed_pids: &HashSet<u32>,
    clients: &HashMap<u32, u32>,
    nodes: impl IntoIterator<Item = (u32, LinuxAudioNodeOwner)>,
) -> HashSet<u32> {
    nodes
        .into_iter()
        .filter_map(|(node_id, owner)| {
            let pid = resolve_audio_node_pid(owner, clients)?;
            allowed_pids.contains(&pid).then_some(node_id)
        })
        .collect()
}

fn resolve_audio_node_pid(owner: LinuxAudioNodeOwner, clients: &HashMap<u32, u32>) -> Option<u32> {
    let client_pid = owner
        .owning_client_id
        .and_then(|client_id| clients.get(&client_id).copied());
    match (owner.app_pid, client_pid) {
        (Some(node_pid), Some(client_pid)) if node_pid == client_pid => Some(node_pid),
        // Both are protocol-projected security identities. A contradiction is
        // not a precedence question; using either value could capture the
        // wrong process, so wait for a coherent registry projection.
        (Some(_), Some(_)) => None,
        (Some(node_pid), None) => Some(node_pid),
        (None, Some(client_pid)) => Some(client_pid),
        (None, None) => None,
    }
}

#[cfg(all(feature = "native-media", target_os = "linux"))]
mod platform {
    use super::{
        eligible_audio_node_ids, parse_proc_stat, process_tree_from_rows, LinuxAudioNodeOwner,
        LinuxProcessIdentity, LinuxProcessRow,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use flexaudio::core::backend::{CaptureBackend, RawSink};
    use flexaudio::core::clock::monotonic_now_ns;
    use flexaudio::core::types::{Error, Result};
    use pipewire as pw;
    use pw::properties::properties;
    use pw::spa;
    use pw::spa::param::format::{MediaSubtype, MediaType};
    use pw::spa::param::format_utils;
    use pw::spa::pod::Pod;
    use pw::stream::StreamFlags;

    const NATIVE_RATE: u32 = 48_000;
    const NATIVE_CHANNELS: u16 = 2;
    const PROCESS_SCRATCH_CAPACITY: usize = NATIVE_RATE as usize * NATIVE_CHANNELS as usize;
    static CAPTURE_INSTANCE: AtomicU64 = AtomicU64::new(1);

    thread_local! {
        static PROCESS_SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    #[derive(Debug, Clone)]
    struct PortEntry {
        node_id: u32,
        direction: String,
        channel: String,
    }

    struct CaptureUserData {
        format: spa::param::audio::AudioInfoRaw,
        sink: RawSink,
    }

    struct ProcessTreeKeep {
        _stream: pw::stream::StreamRc,
        _stream_listener: pw::stream::StreamListener<CaptureUserData>,
        _registry: pw::registry::RegistryRc,
        _registry_listener: pw::registry::Listener,
        _links: Rc<RefCell<HashMap<u32, Vec<pw::link::Link>>>>,
        _core: pw::core::CoreRc,
    }

    #[derive(Clone)]
    struct ReconcileState {
        core: pw::core::CoreRc,
        stream: pw::stream::StreamRc,
        self_node_id: Rc<Cell<Option<u32>>>,
        clients: Rc<RefCell<HashMap<u32, u32>>>,
        nodes: Rc<RefCell<HashMap<u32, LinuxAudioNodeOwner>>>,
        ports: Rc<RefCell<HashMap<u32, PortEntry>>>,
        links: Rc<RefCell<HashMap<u32, Vec<pw::link::Link>>>>,
    }

    impl ReconcileState {
        fn reconcile(&self, root: LinuxProcessIdentity) {
            reconcile_links(
                &self.core,
                &self.stream,
                root,
                &self.self_node_id,
                &self.clients,
                &self.nodes,
                &self.ports,
                &self.links,
            );
        }
    }

    struct Terminate;

    /// PipeWire capture backend for one start-time-anchored Linux process tree.
    pub(in crate::daemon::plugins::remote_desktop) struct LinuxProcessTreeAudioBackend {
        root: LinuxProcessIdentity,
        instance: u64,
        running: Arc<AtomicBool>,
        stop_tx: Option<pw::channel::Sender<Terminate>>,
        handle: Option<JoinHandle<()>>,
    }

    impl LinuxProcessTreeAudioBackend {
        pub(in crate::daemon::plugins::remote_desktop) fn new(root_pid: u32) -> Result<Self> {
            let root = read_process_row(root_pid)
                .map(|row| row.identity)
                .ok_or(Error::DeviceNotFound)?;
            Ok(Self {
                root,
                instance: CAPTURE_INSTANCE.fetch_add(1, Ordering::Relaxed),
                running: Arc::new(AtomicBool::new(false)),
                stop_tx: None,
                handle: None,
            })
        }
    }

    impl CaptureBackend for LinuxProcessTreeAudioBackend {
        fn native_format(&self) -> (u32, u16) {
            (NATIVE_RATE, NATIVE_CHANNELS)
        }

        fn start(&mut self, sink: RawSink) -> Result<()> {
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            if current_process_tree(self.root).is_empty() {
                return Err(Error::DeviceNotFound);
            }

            let (stop_tx, stop_rx) = pw::channel::channel::<Terminate>();
            let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
            let running = Arc::clone(&self.running);
            running.store(true, Ordering::SeqCst);
            let root = self.root;
            let instance = self.instance;
            let handle = thread::Builder::new()
                .name("easynet-rd-pipewire-tree".into())
                .spawn(move || run_pipewire_process_tree(root, instance, sink, stop_rx, ready_tx))
                .map_err(|error| {
                    Error::Backend(format!("spawn PipeWire process-tree thread: {error}"))
                })?;

            match ready_rx.recv() {
                Ok(Ok(())) => {
                    self.stop_tx = Some(stop_tx);
                    self.handle = Some(handle);
                    Ok(())
                }
                Ok(Err(reason)) => {
                    running.store(false, Ordering::SeqCst);
                    let _ = handle.join();
                    Err(Error::Backend(reason))
                }
                Err(_) => {
                    running.store(false, Ordering::SeqCst);
                    let _ = handle.join();
                    Err(Error::Backend(
                        "PipeWire process-tree thread ended before readiness".into(),
                    ))
                }
            }
        }

        fn stop(&mut self) {
            if !self.running.swap(false, Ordering::SeqCst) {
                if let Some(handle) = self.handle.take() {
                    let _ = handle.join();
                }
                self.stop_tx = None;
                return;
            }
            if let Some(stop_tx) = self.stop_tx.take() {
                let _ = stop_tx.send(Terminate);
            }
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for LinuxProcessTreeAudioBackend {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn read_process_row(pid: u32) -> Option<LinuxProcessRow> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        parse_proc_stat(pid, &stat)
    }

    fn current_process_tree(root: LinuxProcessIdentity) -> HashSet<u32> {
        let rows = match fs::read_dir("/proc") {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
                .filter_map(read_process_row)
                .collect::<Vec<_>>(),
            Err(_) => return HashSet::new(),
        };
        process_tree_from_rows(root, rows)
    }

    fn run_pipewire_process_tree(
        root: LinuxProcessIdentity,
        instance: u64,
        sink: RawSink,
        stop_rx: pw::channel::Receiver<Terminate>,
        ready_tx: mpsc::Sender<std::result::Result<(), String>>,
    ) {
        let (main_loop, _keep, reconcile_state) =
            match setup_pipewire_process_tree(root, instance, sink) {
                Ok(value) => value,
                Err(reason) => {
                    let _ = ready_tx.send(Err(reason));
                    return;
                }
            };
        // PipeWire graph callbacks cover node/client churn, but process
        // authority may change without a graph event (for example, root exit
        // while a stale node remains). Revalidate independently so stale
        // links are revoked within a documented bound instead of waiting for
        // unrelated graph activity.
        let revalidation_state = reconcile_state.clone();
        let revalidation_timer = main_loop.loop_().add_timer(move |_| {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                revalidation_state.reconcile(root);
            }));
        });
        let revalidation_period = Duration::from_millis(200);
        if let Err(error) = revalidation_timer
            .update_timer(Some(revalidation_period), Some(revalidation_period))
            .into_result()
        {
            let _ = ready_tx.send(Err(format!(
                "arm PipeWire process-tree authority revalidation: {error}"
            )));
            return;
        }
        let loop_for_stop = main_loop.clone();
        let _stop_receiver = stop_rx.attach(main_loop.loop_(), move |_| loop_for_stop.quit());
        if ready_tx.send(Ok(())).is_err() {
            return;
        }
        main_loop.run();
    }

    fn setup_pipewire_process_tree(
        root: LinuxProcessIdentity,
        instance: u64,
        sink: RawSink,
    ) -> std::result::Result<(pw::main_loop::MainLoopRc, ProcessTreeKeep, ReconcileState), String>
    {
        pw::init();
        let main_loop = pw::main_loop::MainLoopRc::new(None)
            .map_err(|error| format!("create PipeWire main loop: {error}"))?;
        let context = pw::context::ContextRc::new(&main_loop, None)
            .map_err(|error| format!("create PipeWire context: {error}"))?;
        let core = context
            .connect_rc(None)
            .map_err(|error| format!("connect PipeWire daemon: {error}"))?;
        let registry = core
            .get_registry_rc()
            .map_err(|error| format!("get PipeWire registry: {error}"))?;

        let node_name = format!("easynet-remoteapp-audio-{}-{instance}", root.pid);
        let props = properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_CLASS => "Stream/Input/Audio",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::NODE_NAME => node_name.as_str(),
        };
        let stream =
            pw::stream::StreamRc::new(core.clone(), "easynet-remoteapp-process-tree", props)
                .map_err(|error| format!("create PipeWire capture stream: {error}"))?;
        let stream_listener = add_capture_listener(
            &stream,
            CaptureUserData {
                format: spa::param::audio::AudioInfoRaw::new(),
                sink,
            },
        )?;
        let format_bytes = build_format_pod_bytes()?;
        let format = Pod::from_bytes(&format_bytes)
            .ok_or_else(|| "build PipeWire audio format pod".to_string())?;
        let mut params = [format];
        stream
            .connect(
                spa::utils::Direction::Input,
                None,
                StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
                &mut params,
            )
            .map_err(|error| format!("connect PipeWire capture stream: {error}"))?;

        let self_node_id = Rc::new(Cell::new(None));
        let clients = Rc::new(RefCell::new(HashMap::<u32, u32>::new()));
        let nodes = Rc::new(RefCell::new(HashMap::<u32, LinuxAudioNodeOwner>::new()));
        let ports = Rc::new(RefCell::new(HashMap::<u32, PortEntry>::new()));
        let links = Rc::new(RefCell::new(HashMap::<u32, Vec<pw::link::Link>>::new()));

        let reconcile_state = ReconcileState {
            core: core.clone(),
            stream: stream.clone(),
            self_node_id: Rc::clone(&self_node_id),
            clients: Rc::clone(&clients),
            nodes: Rc::clone(&nodes),
            ports: Rc::clone(&ports),
            links: Rc::clone(&links),
        };
        let state_for_global = reconcile_state.clone();
        let state_for_remove = reconcile_state.clone();

        let registry_listener = registry
            .add_listener_local()
            .global(move |global| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let Some(props) = global.props else {
                        return;
                    };
                    match global.type_ {
                        pw::types::ObjectType::Client => {
                            let Some(pid) = props
                                .get(*pw::keys::SEC_PID)
                                .and_then(|value| value.parse::<u32>().ok())
                            else {
                                return;
                            };
                            state_for_global.clients.borrow_mut().insert(global.id, pid);
                        }
                        pw::types::ObjectType::Node => {
                            if props.get(*pw::keys::MEDIA_CLASS).unwrap_or("")
                                != "Stream/Output/Audio"
                            {
                                return;
                            }
                            state_for_global.nodes.borrow_mut().insert(
                                global.id,
                                LinuxAudioNodeOwner {
                                    owning_client_id: props
                                        .get(*pw::keys::CLIENT_ID)
                                        .and_then(|value| value.parse().ok()),
                                    app_pid: props
                                        .get(*pw::keys::SEC_PID)
                                        .and_then(|value| value.parse().ok()),
                                },
                            );
                        }
                        pw::types::ObjectType::Port => {
                            let Some(node_id) = props
                                .get(*pw::keys::NODE_ID)
                                .and_then(|value| value.parse().ok())
                            else {
                                return;
                            };
                            let direction = props
                                .get(*pw::keys::PORT_DIRECTION)
                                .unwrap_or("")
                                .to_string();
                            if direction != "in" && direction != "out" {
                                return;
                            }
                            state_for_global.ports.borrow_mut().insert(
                                global.id,
                                PortEntry {
                                    node_id,
                                    direction,
                                    channel: props
                                        .get(*pw::keys::AUDIO_CHANNEL)
                                        .unwrap_or("")
                                        .to_string(),
                                },
                            );
                        }
                        _ => return,
                    }
                    state_for_global.reconcile(root);
                }));
            })
            .global_remove(move |id| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let removed_port = state_for_remove.ports.borrow_mut().remove(&id);
                    if let Some(port) = &removed_port {
                        if port.direction == "in"
                            && state_for_remove.self_node_id.get() == Some(port.node_id)
                        {
                            state_for_remove.links.borrow_mut().clear();
                        } else if port.direction == "out" {
                            state_for_remove.links.borrow_mut().remove(&port.node_id);
                        }
                    }
                    if state_for_remove.self_node_id.get() == Some(id) {
                        state_for_remove.self_node_id.set(None);
                        state_for_remove.links.borrow_mut().clear();
                    }
                    state_for_remove.nodes.borrow_mut().remove(&id);
                    state_for_remove.clients.borrow_mut().remove(&id);
                    state_for_remove.links.borrow_mut().remove(&id);
                    state_for_remove.reconcile(root);
                }));
            })
            .register();

        Ok((
            main_loop,
            ProcessTreeKeep {
                _stream: stream,
                _stream_listener: stream_listener,
                _registry: registry,
                _registry_listener: registry_listener,
                _links: links,
                _core: core,
            },
            reconcile_state,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_links(
        core: &pw::core::CoreRc,
        stream: &pw::stream::StreamRc,
        root: LinuxProcessIdentity,
        self_node_id: &Cell<Option<u32>>,
        clients: &RefCell<HashMap<u32, u32>>,
        nodes: &RefCell<HashMap<u32, LinuxAudioNodeOwner>>,
        ports: &RefCell<HashMap<u32, PortEntry>>,
        links: &RefCell<HashMap<u32, Vec<pw::link::Link>>>,
    ) {
        let stream_node_id = stream.node_id();
        if stream_node_id != 0 && stream_node_id != pw::constants::ID_ANY {
            self_node_id.set(Some(stream_node_id));
        }
        let Some(input_node_id) = self_node_id.get() else {
            return;
        };
        let allowed_pids = current_process_tree(root);
        let eligible_nodes = {
            let clients = clients.borrow();
            let nodes = nodes.borrow();
            eligible_audio_node_ids(
                &allowed_pids,
                &clients,
                nodes.iter().map(|(&node_id, &owner)| (node_id, owner)),
            )
        };

        links
            .borrow_mut()
            .retain(|node_id, _| eligible_nodes.contains(node_id));
        if eligible_nodes.is_empty() {
            return;
        }

        let input_ports = ports
            .borrow()
            .iter()
            .filter(|(_, port)| port.node_id == input_node_id && port.direction == "in")
            .map(|(&id, port)| (id, port.channel.clone()))
            .collect::<Vec<_>>();
        if input_ports.is_empty() {
            return;
        }

        for node_id in eligible_nodes {
            if links.borrow().contains_key(&node_id) {
                continue;
            }
            let output_ports = ports
                .borrow()
                .iter()
                .filter(|(_, port)| port.node_id == node_id && port.direction == "out")
                .map(|(&id, port)| (id, port.channel.clone()))
                .collect::<Vec<_>>();
            let pairs = pair_ports(&output_ports, &input_ports);
            if pairs.is_empty() {
                continue;
            }
            let expected = pairs.len();
            let mut created = Vec::with_capacity(expected);
            for (output_port, input_port) in pairs {
                let props = properties! {
                    *pw::keys::LINK_OUTPUT_NODE => node_id.to_string(),
                    *pw::keys::LINK_OUTPUT_PORT => output_port.to_string(),
                    *pw::keys::LINK_INPUT_NODE => input_node_id.to_string(),
                    *pw::keys::LINK_INPUT_PORT => input_port.to_string(),
                };
                match core.create_object::<pw::link::Link>("link-factory", &props) {
                    Ok(link) => created.push(link),
                    Err(_) => break,
                }
            }
            if created.len() == expected {
                links.borrow_mut().insert(node_id, created);
            }
        }
    }

    fn pair_ports(outputs: &[(u32, String)], inputs: &[(u32, String)]) -> Vec<(u32, u32)> {
        let mut pairs = Vec::new();
        let mut used_inputs = vec![false; inputs.len()];
        for (output_id, output_channel) in outputs {
            if output_channel.is_empty() {
                continue;
            }
            if let Some(index) = inputs
                .iter()
                .enumerate()
                .position(|(index, (_, channel))| !used_inputs[index] && channel == output_channel)
            {
                used_inputs[index] = true;
                pairs.push((*output_id, inputs[index].0));
            }
        }
        if outputs.len() == 1 {
            for (index, (input_id, _)) in inputs.iter().enumerate() {
                if !used_inputs[index] {
                    used_inputs[index] = true;
                    pairs.push((outputs[0].0, *input_id));
                }
            }
            return pairs;
        }
        let mut used_outputs = pairs
            .iter()
            .map(|(output, _)| *output)
            .collect::<HashSet<_>>();
        for (output_id, _) in outputs {
            if used_outputs.contains(output_id) {
                continue;
            }
            if let Some(index) = used_inputs.iter().position(|used| !used) {
                used_inputs[index] = true;
                used_outputs.insert(*output_id);
                pairs.push((*output_id, inputs[index].0));
            }
        }
        pairs
    }

    fn add_capture_listener(
        stream: &pw::stream::StreamRc,
        user_data: CaptureUserData,
    ) -> std::result::Result<pw::stream::StreamListener<CaptureUserData>, String> {
        PROCESS_SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            let missing = PROCESS_SCRATCH_CAPACITY.saturating_sub(scratch.capacity());
            scratch.reserve(missing);
        });
        stream
            .add_local_listener_with_user_data(user_data)
            .param_changed(|_, user_data, id, param| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let Some(param) = param else {
                        return;
                    };
                    if id != spa::param::ParamType::Format.as_raw() {
                        return;
                    }
                    let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                        return;
                    };
                    if media_type == MediaType::Audio && media_subtype == MediaSubtype::Raw {
                        let _ = user_data.format.parse(param);
                    }
                }));
            })
            .process(|stream, user_data| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let Some(mut buffer) = stream.dequeue_buffer() else {
                        return;
                    };
                    let Some(data) = buffer.datas_mut().first_mut() else {
                        return;
                    };
                    let chunk = data.chunk();
                    let size = chunk.size() as usize;
                    let offset = chunk.offset() as usize;
                    let Some(bytes) = data.data() else {
                        return;
                    };
                    let end = offset.saturating_add(size);
                    if size == 0 || end > bytes.len() {
                        return;
                    }
                    let valid = &bytes[offset..end];
                    let sample_count = valid.len() / std::mem::size_of::<f32>();
                    PROCESS_SCRATCH.with(|scratch| {
                        let mut scratch = scratch.borrow_mut();
                        let missing = sample_count.saturating_sub(scratch.capacity());
                        scratch.reserve(missing);
                        scratch.clear();
                        for bytes in valid.chunks_exact(4) {
                            scratch
                                .push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                        }
                        user_data.sink.push(&scratch, monotonic_now_ns());
                    });
                }));
            })
            .register()
            .map_err(|error| format!("register PipeWire stream listener: {error}"))
    }

    fn build_format_pod_bytes() -> std::result::Result<Vec<u8>, String> {
        let mut info = spa::param::audio::AudioInfoRaw::new();
        info.set_format(spa::param::audio::AudioFormat::F32LE);
        info.set_rate(NATIVE_RATE);
        info.set_channels(NATIVE_CHANNELS as u32);
        let object = pw::spa::pod::Object {
            type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties: info.into(),
        };
        pw::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(object),
        )
        .map(|serialized| serialized.0.into_inner())
        .map_err(|error| format!("serialize PipeWire audio format: {error}"))
    }
}

#[cfg(all(feature = "native-media", target_os = "linux"))]
pub(in crate::daemon::plugins::remote_desktop) use platform::LinuxProcessTreeAudioBackend;

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, parent_pid: u32, start_time_ticks: u64) -> LinuxProcessRow {
        LinuxProcessRow {
            identity: LinuxProcessIdentity {
                pid,
                start_time_ticks,
            },
            parent_pid,
        }
    }

    #[test]
    fn proc_stat_parser_handles_spaces_and_close_parentheses_in_process_name() {
        let mut tail = vec!["S".to_string(), "41".to_string()];
        tail.extend((5..=22).map(|field| {
            if field == 22 {
                "9001".to_string()
            } else {
                field.to_string()
            }
        }));
        let stat = format!("42 (audio worker ) helper) {}", tail.join(" "));
        let parsed = parse_proc_stat(42, &stat).expect("valid proc stat");
        assert_eq!(parsed.parent_pid, 41);
        assert_eq!(parsed.identity.start_time_ticks, 9001);
    }

    #[test]
    fn process_tree_includes_all_descendants_and_excludes_unrelated_processes() {
        let root = LinuxProcessIdentity {
            pid: 10,
            start_time_ticks: 100,
        };
        let selected = process_tree_from_rows(
            root,
            [
                row(10, 1, 100),
                row(11, 10, 110),
                row(12, 11, 120),
                row(13, 10, 130),
                row(90, 1, 900),
                row(91, 90, 910),
            ],
        );
        assert_eq!(selected, HashSet::from([10, 11, 12, 13]));
    }

    #[test]
    fn reused_root_pid_fails_closed_instead_of_selecting_new_process_tree() {
        let selected = process_tree_from_rows(
            LinuxProcessIdentity {
                pid: 10,
                start_time_ticks: 100,
            },
            [row(10, 1, 999), row(11, 10, 1_000)],
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn missing_root_fails_closed_even_when_orphan_rows_reference_its_pid() {
        let selected = process_tree_from_rows(
            LinuxProcessIdentity {
                pid: 10,
                start_time_ticks: 100,
            },
            [row(11, 10, 110), row(12, 11, 120)],
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn audio_node_selection_includes_every_node_in_the_process_tree() {
        let allowed_pids = HashSet::from([10, 11]);
        let clients = HashMap::from([(501, 10), (502, 11), (590, 90)]);
        let selected = eligible_audio_node_ids(
            &allowed_pids,
            &clients,
            [
                (
                    1_001,
                    LinuxAudioNodeOwner {
                        app_pid: Some(10),
                        owning_client_id: None,
                    },
                ),
                (
                    1_002,
                    LinuxAudioNodeOwner {
                        app_pid: Some(10),
                        owning_client_id: None,
                    },
                ),
                (
                    1_003,
                    LinuxAudioNodeOwner {
                        app_pid: None,
                        owning_client_id: Some(502),
                    },
                ),
                (
                    1_090,
                    LinuxAudioNodeOwner {
                        app_pid: None,
                        owning_client_id: Some(590),
                    },
                ),
            ],
        );
        assert_eq!(selected, HashSet::from([1_001, 1_002, 1_003]));
    }

    #[test]
    fn contradictory_node_and_client_pid_identity_fails_closed_in_both_directions() {
        for (node_pid, client_pid) in [(90, 10), (10, 90)] {
            let selected = eligible_audio_node_ids(
                &HashSet::from([10]),
                &HashMap::from([(501, client_pid)]),
                [(
                    1_001,
                    LinuxAudioNodeOwner {
                        app_pid: Some(node_pid),
                        owning_client_id: Some(501),
                    },
                )],
            );
            assert!(selected.is_empty());
        }
    }

    #[test]
    fn empty_authority_set_revokes_all_previously_eligible_nodes() {
        let selected = eligible_audio_node_ids(
            &HashSet::new(),
            &HashMap::from([(501, 10)]),
            [(
                1_001,
                LinuxAudioNodeOwner {
                    app_pid: None,
                    owning_client_id: Some(501),
                },
            )],
        );
        assert!(selected.is_empty());
    }
}
