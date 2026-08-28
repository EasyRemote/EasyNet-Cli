//! Generation-fenced shared-memory media slots for RemoteApp.
//!
//! This module owns the allocation and lease primitive only. `RVID`/`RAUD`
//! framing, conversation sequencing, notification descriptors and WebRTC
//! lifecycle remain with their existing owners.

use std::fmt;
#[cfg(unix)]
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::{align_of, size_of};
use std::ops::Range;
#[cfg(windows)]
use std::ops::{Deref, DerefMut};
#[cfg(windows)]
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(unix)]
use memmap2::{Mmap, MmapMut, MmapOptions};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING,
    PIPE_ACCESS_INBOUND,
};
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ,
    FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::media_session::{
    binary_media_header_len, encode_binary_media_header_compact, generation_nonce_bytes,
    validate_event_shape, EventBody, EventMetadata, MediaLane, MAX_PAYLOAD_BYTES,
};

const MAGIC: [u8; 8] = *b"RMSHLN02";
const VERSION: u32 = 2;
const CONTROL_REGION_BYTES: usize = 64 * 1024;
const SLOT_ALIGNMENT: usize = 64;
const MAX_SLOTS: usize = 8;
const MAX_FRAME_BYTES: usize = MAX_PAYLOAD_BYTES + 128;
pub const VIDEO_SHARED_LANE_FD_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_VIDEO_SHM_FD";
pub const AUDIO_SHARED_LANE_FD_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_AUDIO_SHM_FD";
pub const VIDEO_SHARED_LANE_NAME_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_VIDEO_SHM_NAME";
pub const AUDIO_SHARED_LANE_NAME_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_AUDIO_SHM_NAME";
pub const VIDEO_NOTIFICATION_PIPE_NAME_ENV: &str =
    "EASYNET_REMOTEAPP_MEDIA_VIDEO_NOTIFICATION_PIPE";
pub const AUDIO_NOTIFICATION_PIPE_NAME_ENV: &str =
    "EASYNET_REMOTEAPP_MEDIA_AUDIO_NOTIFICATION_PIPE";
const NOTIFICATION_MAGIC: [u8; 4] = *b"RSNT";
pub const SHARED_SLOT_NOTIFICATION_BYTES: usize = 56;
const NOTIFICATION_PUBLISHED: u8 = 1;
const NOTIFICATION_DROPPED: u8 = 2;
const NOTIFICATION_REPLACED: u8 = 1 << 0;

const STATE_FREE: u32 = 0;
const STATE_WRITING: u32 = 1;
const STATE_READY: u32 = 2;
const STATE_READING: u32 = 3;

#[repr(C, align(64))]
struct LaneHeader {
    magic: [u8; 8],
    version: u32,
    lane: u32,
    slot_count: u32,
    slot_capacity: u32,
    slot_stride: u32,
    reserved: u32,
    generation_nonce: [u8; 16],
    next_ticket: AtomicU64,
}

#[repr(C, align(64))]
struct SlotControl {
    state: AtomicU32,
    frame_len: AtomicU32,
    ticket: AtomicU64,
    sequence: AtomicU64,
}

#[derive(Debug)]
pub enum SharedMediaLaneError {
    Io(io::Error),
    Invalid(String),
    StaleTicket,
    SlotUnavailable,
}

impl fmt::Display for SharedMediaLaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "shared media lane I/O failed: {error}"),
            Self::Invalid(detail) => write!(formatter, "invalid shared media lane: {detail}"),
            Self::StaleTicket => write!(formatter, "shared media lane ticket is stale"),
            Self::SlotUnavailable => write!(formatter, "shared media lane slot is unavailable"),
        }
    }
}

impl std::error::Error for SharedMediaLaneError {}

impl From<io::Error> for SharedMediaLaneError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, SharedMediaLaneError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedMediaLaneLayout {
    pub lane: MediaLane,
    pub slot_count: u32,
    pub slot_capacity: u32,
    pub slot_stride: u32,
    pub generation_nonce: [u8; 16],
}

impl SharedMediaLaneLayout {
    pub fn new(
        lane: MediaLane,
        slot_count: u32,
        slot_capacity: u32,
        generation_nonce: [u8; 16],
    ) -> Result<Self> {
        if lane == MediaLane::Control {
            return Err(SharedMediaLaneError::Invalid(
                "control events cannot use the shared media hot lane".into(),
            ));
        }
        let slot_count_usize = slot_count as usize;
        let slot_capacity_usize = slot_capacity as usize;
        if slot_count_usize == 0 || slot_count_usize > MAX_SLOTS {
            return Err(SharedMediaLaneError::Invalid(format!(
                "slot count must be in 1..={MAX_SLOTS}"
            )));
        }
        if slot_capacity_usize == 0 || slot_capacity_usize > MAX_FRAME_BYTES {
            return Err(SharedMediaLaneError::Invalid(format!(
                "slot capacity must be in 1..={MAX_FRAME_BYTES}"
            )));
        }
        if generation_nonce.iter().all(|byte| *byte == 0) {
            return Err(SharedMediaLaneError::Invalid(
                "generation nonce cannot be all zero".into(),
            ));
        }
        let slot_stride = align_up(slot_capacity_usize, SLOT_ALIGNMENT)
            .ok_or_else(|| SharedMediaLaneError::Invalid("slot stride overflow".into()))?;
        let control_bytes = slot_controls_offset()
            .checked_add(slot_count_usize.saturating_mul(size_of::<SlotControl>()))
            .ok_or_else(|| SharedMediaLaneError::Invalid("control layout overflow".into()))?;
        if control_bytes > CONTROL_REGION_BYTES {
            return Err(SharedMediaLaneError::Invalid(
                "slot controls exceed the fixed control region".into(),
            ));
        }
        Ok(Self {
            lane,
            slot_count,
            slot_capacity,
            slot_stride: u32::try_from(slot_stride)
                .map_err(|_| SharedMediaLaneError::Invalid("slot stride exceeds u32".into()))?,
            generation_nonce,
        })
    }

    fn file_len(self) -> Result<u64> {
        let data_bytes = u64::from(self.slot_stride)
            .checked_mul(u64::from(self.slot_count))
            .ok_or_else(|| SharedMediaLaneError::Invalid("data layout overflow".into()))?;
        (CONTROL_REGION_BYTES as u64)
            .checked_add(data_bytes)
            .ok_or_else(|| SharedMediaLaneError::Invalid("mapping length overflow".into()))
    }
}

pub struct SharedMediaLaneFile {
    #[cfg(unix)]
    file: File,
    #[cfg(windows)]
    mapping: WindowsNamedMapping,
    layout: SharedMediaLaneLayout,
}

impl SharedMediaLaneFile {
    pub fn create(layout: SharedMediaLaneLayout) -> Result<Self> {
        #[cfg(unix)]
        let owner = {
            let file = tempfile::tempfile()?;
            file.set_len(layout.file_len()?)?;
            let mut control = unsafe {
                MmapOptions::new()
                    .len(CONTROL_REGION_BYTES)
                    .map_mut(&file)?
            };
            initialize_control(&mut control, layout);
            control.flush()?;
            drop(control);
            Self { file, layout }
        };
        #[cfg(windows)]
        let owner = {
            let name = windows_mapping_name(layout);
            let wide_name = windows_wide(&name)?;
            let mapping_len = layout.file_len()?;
            let handle = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    std::ptr::null(),
                    PAGE_READWRITE,
                    (mapping_len >> 32) as u32,
                    mapping_len as u32,
                    wide_name.as_ptr(),
                )
            };
            if handle.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return Err(SharedMediaLaneError::Invalid(
                    "generation-scoped Windows media mapping already exists".into(),
                ));
            }
            let mut control = match WindowsMappedView::map_handle(
                handle,
                FILE_MAP_READ | FILE_MAP_WRITE,
                0,
                CONTROL_REGION_BYTES,
                false,
            ) {
                Ok(control) => control,
                Err(error) => {
                    unsafe { CloseHandle(handle) };
                    return Err(error);
                }
            };
            control.fill(0);
            initialize_control(&mut control, layout);
            drop(control);
            Self {
                mapping: WindowsNamedMapping { handle, name },
                layout,
            }
        };
        Ok(owner)
    }

    #[cfg(unix)]
    pub fn try_clone_file(&self) -> Result<File> {
        Ok(self.file.try_clone()?)
    }

    #[cfg(windows)]
    pub fn bootstrap_name(&self) -> &str {
        &self.mapping.name
    }

    pub const fn layout(&self) -> SharedMediaLaneLayout {
        self.layout
    }
}

fn initialize_control(control: &mut [u8], layout: SharedMediaLaneLayout) {
    control.fill(0);
    let header = LaneHeader {
        magic: MAGIC,
        version: VERSION,
        lane: lane_code(layout.lane),
        slot_count: layout.slot_count,
        slot_capacity: layout.slot_capacity,
        slot_stride: layout.slot_stride,
        reserved: 0,
        generation_nonce: layout.generation_nonce,
        next_ticket: AtomicU64::new(1),
    };
    unsafe {
        control.as_mut_ptr().cast::<LaneHeader>().write(header);
        for index in 0..layout.slot_count as usize {
            slot_control_mut_ptr(control, index).write(SlotControl {
                state: AtomicU32::new(STATE_FREE),
                frame_len: AtomicU32::new(0),
                ticket: AtomicU64::new(0),
                sequence: AtomicU64::new(0),
            });
        }
    }
}

#[cfg(unix)]
impl std::os::fd::AsRawFd for SharedMediaLaneFile {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.file)
    }
}

#[cfg(windows)]
struct WindowsNamedMapping {
    handle: HANDLE,
    name: String,
}

#[cfg(windows)]
impl Drop for WindowsNamedMapping {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { CloseHandle(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(windows)]
struct WindowsMappedView {
    handle: HANDLE,
    address: NonNull<u8>,
    len: usize,
    owns_handle: bool,
}

#[cfg(windows)]
unsafe impl Send for WindowsMappedView {}
#[cfg(windows)]
unsafe impl Sync for WindowsMappedView {}

#[cfg(windows)]
impl WindowsMappedView {
    fn open(name: &str, access: u32, offset: u64, len: usize) -> Result<Self> {
        let wide_name = windows_wide(name)?;
        let handle = unsafe { OpenFileMappingW(access, 0, wide_name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        Self::map_handle(handle, access, offset, len, true)
    }

    fn map_handle(
        handle: HANDLE,
        access: u32,
        offset: u64,
        len: usize,
        owns_handle: bool,
    ) -> Result<Self> {
        let view =
            unsafe { MapViewOfFile(handle, access, (offset >> 32) as u32, offset as u32, len) };
        let Some(address) = NonNull::new(view.Value.cast::<u8>()) else {
            let error = io::Error::last_os_error();
            if owns_handle {
                unsafe { CloseHandle(handle) };
            }
            return Err(error.into());
        };
        Ok(Self {
            handle,
            address,
            len,
            owns_handle,
        })
    }
}

#[cfg(windows)]
impl Deref for WindowsMappedView {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        unsafe { std::slice::from_raw_parts(self.address.as_ptr(), self.len) }
    }
}

#[cfg(windows)]
impl DerefMut for WindowsMappedView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::slice::from_raw_parts_mut(self.address.as_ptr(), self.len) }
    }
}

#[cfg(windows)]
impl Drop for WindowsMappedView {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.address.as_ptr().cast(),
            });
            if self.owns_handle && !self.handle.is_null() {
                CloseHandle(self.handle);
            }
        }
        self.handle = std::ptr::null_mut();
    }
}

#[cfg(windows)]
fn windows_mapping_name(layout: SharedMediaLaneLayout) -> String {
    let mut nonce = String::with_capacity(32);
    for byte in layout.generation_nonce {
        use std::fmt::Write as _;
        write!(&mut nonce, "{byte:02x}").expect("writing into String cannot fail");
    }
    format!(
        "Local\\EasyNet.RemoteApp.Media.v{VERSION}.{}.{}.{}",
        std::process::id(),
        nonce,
        lane_code(layout.lane)
    )
}

#[cfg(windows)]
fn windows_wide(value: &str) -> Result<Vec<u16>> {
    if value.encode_utf16().any(|unit| unit == 0) {
        return Err(SharedMediaLaneError::Invalid(
            "Windows media bootstrap name contains NUL".into(),
        ));
    }
    Ok(value.encode_utf16().chain(std::iter::once(0)).collect())
}

#[cfg(windows)]
pub struct WindowsNotificationPipeServer {
    handle: HANDLE,
    name: String,
    connected: Option<std::fs::File>,
}

#[cfg(windows)]
unsafe impl Send for WindowsNotificationPipeServer {}

#[cfg(windows)]
impl WindowsNotificationPipeServer {
    pub fn create(lane: MediaLane, generation_nonce: [u8; 16]) -> Result<Self> {
        if lane == MediaLane::Control || generation_nonce.iter().all(|byte| *byte == 0) {
            return Err(SharedMediaLaneError::Invalid(
                "Windows notification pipe requires a media lane and generation nonce".into(),
            ));
        }
        let mut nonce = String::with_capacity(32);
        for byte in generation_nonce {
            use std::fmt::Write as _;
            write!(&mut nonce, "{byte:02x}").expect("writing into String cannot fail");
        }
        let name = format!(
            "\\\\.\\pipe\\EasyNet.RemoteApp.Media.v{VERSION}.{}.{}.{}",
            std::process::id(),
            nonce,
            lane_code(lane)
        );
        let wide_name = windows_wide(&name)?;
        let handle = unsafe {
            CreateNamedPipeW(
                wide_name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                SHARED_SLOT_NOTIFICATION_BYTES as u32 * 8,
                5_000,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            handle,
            name,
            connected: None,
        })
    }

    pub fn bootstrap_name(&self) -> &str {
        &self.name
    }

    fn ensure_connected(&mut self) -> Result<()> {
        if self.connected.is_some() {
            return Ok(());
        }
        let connected = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            return Err(io::Error::last_os_error().into());
        }
        use std::os::windows::io::FromRawHandle;
        let file = unsafe { std::fs::File::from_raw_handle(self.handle.cast()) };
        self.handle = INVALID_HANDLE_VALUE;
        self.connected = Some(file);
        Ok(())
    }
}

#[cfg(windows)]
impl Read for WindowsNotificationPipeServer {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.ensure_connected()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.connected
            .as_mut()
            .expect("connected Windows notification pipe must own its file")
            .read(buffer)
    }
}

#[cfg(windows)]
impl Drop for WindowsNotificationPipeServer {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
    }
}

#[cfg(windows)]
pub fn open_windows_notification_writer(name: &str) -> Result<std::fs::File> {
    let wide_name = windows_wide(name)?;
    if unsafe { WaitNamedPipeW(wide_name.as_ptr(), 5_000) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let handle = unsafe {
        CreateFileW(
            wide_name.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().into());
    }
    use std::os::windows::io::FromRawHandle;
    Ok(unsafe { std::fs::File::from_raw_handle(handle.cast()) })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedFrameIdentity {
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub media_gate: u32,
}

impl SharedFrameIdentity {
    fn validate(self) -> Result<Self> {
        if self.sequence == 0 || self.observed_at_ms == 0 || self.media_gate == 0 {
            return Err(SharedMediaLaneError::Invalid(
                "shared media frame identity must be positive".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSlotTicket {
    pub slot_index: u32,
    pub ticket: u64,
    pub identity: SharedFrameIdentity,
    pub replaced_ticket: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedPublishOutcome {
    Published(SharedSlotTicket),
    Dropped { identity: SharedFrameIdentity },
}

impl SharedPublishOutcome {
    pub fn identity(self) -> SharedFrameIdentity {
        match self {
            Self::Published(ticket) => ticket.identity,
            Self::Dropped { identity } => identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedSlotNotification {
    Published(SharedSlotTicket),
    Dropped { identity: SharedFrameIdentity },
}

impl From<SharedPublishOutcome> for SharedSlotNotification {
    fn from(outcome: SharedPublishOutcome) -> Self {
        match outcome {
            SharedPublishOutcome::Published(ticket) => Self::Published(ticket),
            SharedPublishOutcome::Dropped { identity } => Self::Dropped { identity },
        }
    }
}

impl SharedSlotNotification {
    pub fn write_to(self, writer: &mut impl Write, lane: MediaLane) -> Result<()> {
        if lane == MediaLane::Control {
            return Err(SharedMediaLaneError::Invalid(
                "control events cannot use shared slot notifications".into(),
            ));
        }
        let mut bytes = [0_u8; SHARED_SLOT_NOTIFICATION_BYTES];
        bytes[..4].copy_from_slice(&NOTIFICATION_MAGIC);
        bytes[4] = VERSION as u8;
        bytes[5] = lane_code(lane) as u8;
        let (kind, slot_index, ticket, identity, replaced_ticket) = match self {
            Self::Published(ticket) => (
                NOTIFICATION_PUBLISHED,
                ticket.slot_index,
                ticket.ticket,
                ticket.identity,
                ticket.replaced_ticket,
            ),
            Self::Dropped { identity } => (NOTIFICATION_DROPPED, u32::MAX, 0, identity, None),
        };
        identity.validate()?;
        bytes[6] = kind;
        bytes[7] = u8::from(replaced_ticket.is_some()) * NOTIFICATION_REPLACED;
        bytes[8..12].copy_from_slice(&slot_index.to_be_bytes());
        bytes[16..24].copy_from_slice(&ticket.to_be_bytes());
        bytes[24..32].copy_from_slice(&identity.sequence.to_be_bytes());
        bytes[32..40].copy_from_slice(&identity.observed_at_ms.to_be_bytes());
        bytes[40..44].copy_from_slice(&identity.media_gate.to_be_bytes());
        bytes[48..56].copy_from_slice(&replaced_ticket.unwrap_or(0).to_be_bytes());
        writer.write_all(&bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn read_from(reader: &mut impl Read, lane: MediaLane) -> Result<Option<Self>> {
        if lane == MediaLane::Control {
            return Err(SharedMediaLaneError::Invalid(
                "control events cannot use shared slot notifications".into(),
            ));
        }
        let mut bytes = [0_u8; SHARED_SLOT_NOTIFICATION_BYTES];
        let read = reader.read(&mut bytes[..1])?;
        if read == 0 {
            return Ok(None);
        }
        reader.read_exact(&mut bytes[1..])?;
        if bytes[..4] != NOTIFICATION_MAGIC
            || bytes[4] != VERSION as u8
            || bytes[5] != lane_code(lane) as u8
            || bytes[7] & !NOTIFICATION_REPLACED != 0
            || bytes[12..16] != [0; 4]
            || bytes[44..48] != [0; 4]
        {
            return Err(SharedMediaLaneError::Invalid(
                "shared slot notification envelope is invalid".into(),
            ));
        }
        let identity = SharedFrameIdentity {
            sequence: u64::from_be_bytes(bytes[24..32].try_into().unwrap()),
            observed_at_ms: u64::from_be_bytes(bytes[32..40].try_into().unwrap()),
            media_gate: u32::from_be_bytes(bytes[40..44].try_into().unwrap()),
        }
        .validate()?;
        let slot_index = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        let ticket = u64::from_be_bytes(bytes[16..24].try_into().unwrap());
        let replaced_ticket = u64::from_be_bytes(bytes[48..56].try_into().unwrap());
        match bytes[6] {
            NOTIFICATION_PUBLISHED
                if slot_index != u32::MAX
                    && ticket != 0
                    && (bytes[7] & NOTIFICATION_REPLACED != 0) == (replaced_ticket != 0) =>
            {
                Ok(Some(Self::Published(SharedSlotTicket {
                    slot_index,
                    ticket,
                    identity,
                    replaced_ticket: (replaced_ticket != 0).then_some(replaced_ticket),
                })))
            }
            NOTIFICATION_DROPPED
                if slot_index == u32::MAX
                    && ticket == 0
                    && replaced_ticket == 0
                    && bytes[7] == 0 =>
            {
                Ok(Some(Self::Dropped { identity }))
            }
            _ => Err(SharedMediaLaneError::Invalid(
                "shared slot notification body is invalid".into(),
            )),
        }
    }
}

pub struct SharedMediaLaneProducer {
    #[cfg(unix)]
    control: MmapMut,
    #[cfg(windows)]
    control: WindowsMappedView,
    #[cfg(unix)]
    data: MmapMut,
    #[cfg(windows)]
    data: WindowsMappedView,
    layout: SharedMediaLaneLayout,
    next_slot: usize,
    protected_video_recovery: Option<(usize, u64)>,
}

impl SharedMediaLaneProducer {
    #[cfg(unix)]
    pub fn open(file: &File, expected_lane: MediaLane, expected_nonce: [u8; 16]) -> Result<Self> {
        let control = unsafe { MmapOptions::new().len(CONTROL_REGION_BYTES).map_mut(file)? };
        let layout = validate_header(&control, expected_lane, expected_nonce)?;
        let data = unsafe {
            MmapOptions::new()
                .offset(CONTROL_REGION_BYTES as u64)
                .len(data_region_len(layout)?)
                .map_mut(file)?
        };
        Ok(Self {
            control,
            data,
            layout,
            next_slot: 0,
            protected_video_recovery: None,
        })
    }

    #[cfg(windows)]
    pub fn open_named(
        name: &str,
        expected_lane: MediaLane,
        expected_nonce: [u8; 16],
    ) -> Result<Self> {
        let control = WindowsMappedView::open(
            name,
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            CONTROL_REGION_BYTES,
        )?;
        let layout = validate_header(&control, expected_lane, expected_nonce)?;
        let data = WindowsMappedView::open(
            name,
            FILE_MAP_READ | FILE_MAP_WRITE,
            CONTROL_REGION_BYTES as u64,
            data_region_len(layout)?,
        )?;
        Ok(Self {
            control,
            data,
            layout,
            next_slot: 0,
            protected_video_recovery: None,
        })
    }

    pub const fn layout(&self) -> SharedMediaLaneLayout {
        self.layout
    }

    pub fn publish(
        &mut self,
        identity: SharedFrameIdentity,
        frame: &[u8],
    ) -> Result<SharedPublishOutcome> {
        let identity = identity.validate()?;
        if frame.is_empty() || frame.len() > self.layout.slot_capacity as usize {
            return Err(SharedMediaLaneError::Invalid(format!(
                "frame length {} exceeds slot capacity {}",
                frame.len(),
                self.layout.slot_capacity
            )));
        }
        self.publish_frame(identity, frame.len(), false, |slot| {
            slot.copy_from_slice(frame);
            Ok(())
        })
    }

    /// Encode one canonical `RVID`/`RAUD` frame directly into a shared slot.
    /// The codec payload is copied exactly once and no contiguous staging
    /// allocation is created.
    pub fn publish_event(
        &mut self,
        metadata: &EventMetadata,
        payload: &[u8],
    ) -> Result<SharedPublishOutcome> {
        let nonce = generation_nonce_bytes(&metadata.fence)
            .map_err(|error| SharedMediaLaneError::Invalid(error.to_string()))?;
        if nonce != self.layout.generation_nonce {
            return Err(SharedMediaLaneError::Invalid(
                "media event generation differs from shared lane".into(),
            ));
        }
        self.publish_media_event(
            metadata.sequence,
            metadata.observed_at_ms,
            &metadata.body,
            payload,
        )
    }

    /// Publish one media event without constructing or cloning an owned
    /// generation envelope on the hot path. Lane open already validated the
    /// immutable generation nonce.
    pub fn publish_media_event(
        &mut self,
        sequence: u64,
        observed_at_ms: u64,
        body: &EventBody,
        payload: &[u8],
    ) -> Result<SharedPublishOutcome> {
        validate_event_shape(self.layout.lane, sequence, observed_at_ms, body, payload)
            .map_err(|error| SharedMediaLaneError::Invalid(error.to_string()))?;
        let media_gate = match body {
            EventBody::VideoH264 { media_gate, .. } | EventBody::AudioOpus { media_gate, .. } => {
                *media_gate
            }
            _ => {
                return Err(SharedMediaLaneError::Invalid(
                    "shared media lane requires video or audio metadata".into(),
                ))
            }
        };
        let identity = SharedFrameIdentity {
            sequence,
            observed_at_ms,
            media_gate,
        }
        .validate()?;
        let header_len = binary_media_header_len(self.layout.lane)
            .map_err(|error| SharedMediaLaneError::Invalid(error.to_string()))?;
        let frame_len = header_len
            .checked_add(payload.len())
            .ok_or_else(|| SharedMediaLaneError::Invalid("shared frame length overflow".into()))?;
        if frame_len > self.layout.slot_capacity as usize {
            return Err(SharedMediaLaneError::Invalid(format!(
                "frame length {frame_len} exceeds slot capacity {}",
                self.layout.slot_capacity
            )));
        }
        let lane = self.layout.lane;
        let generation_nonce = self.layout.generation_nonce;
        let protects_video_recovery = matches!(
            body,
            EventBody::VideoH264 {
                keyframe: true,
                sps_pps_present: true,
                ..
            }
        );
        self.publish_frame(identity, frame_len, protects_video_recovery, |slot| {
            let (header, slot_payload) = slot.split_at_mut(header_len);
            encode_binary_media_header_compact(
                header,
                lane,
                generation_nonce,
                sequence,
                observed_at_ms,
                body,
                payload.len(),
            )
            .map_err(|error| SharedMediaLaneError::Invalid(error.to_string()))?;
            slot_payload.copy_from_slice(payload);
            Ok(())
        })
    }

    fn publish_frame(
        &mut self,
        identity: SharedFrameIdentity,
        frame_len: usize,
        protects_video_recovery: bool,
        fill: impl FnOnce(&mut [u8]) -> Result<()>,
    ) -> Result<SharedPublishOutcome> {
        let Some((slot_index, replaced_ticket)) =
            self.acquire_writable_slot(protects_video_recovery)?
        else {
            return Ok(SharedPublishOutcome::Dropped { identity });
        };
        let range = slot_range(self.layout, slot_index, frame_len)?;
        if let Err(error) = fill(&mut self.data[range]) {
            slot_control(&self.control, slot_index)
                .state
                .store(STATE_FREE, Ordering::Release);
            return Err(error);
        }
        let control = slot_control(&self.control, slot_index);
        let ticket = header(&self.control)
            .next_ticket
            .fetch_add(1, Ordering::Relaxed);
        if ticket == 0 || ticket == u64::MAX {
            control.state.store(STATE_FREE, Ordering::Release);
            return Err(SharedMediaLaneError::Invalid(
                "shared media lane ticket space exhausted".into(),
            ));
        }
        control.frame_len.store(frame_len as u32, Ordering::Relaxed);
        control.sequence.store(identity.sequence, Ordering::Relaxed);
        control.ticket.store(ticket, Ordering::Relaxed);
        control.state.store(STATE_READY, Ordering::Release);
        if protects_video_recovery {
            self.protected_video_recovery = Some((slot_index, ticket));
        }
        self.next_slot = (slot_index + 1) % self.layout.slot_count as usize;
        Ok(SharedPublishOutcome::Published(SharedSlotTicket {
            slot_index: slot_index as u32,
            ticket,
            identity,
            replaced_ticket,
        }))
    }

    fn acquire_writable_slot(
        &mut self,
        replacing_with_video_recovery: bool,
    ) -> Result<Option<(usize, Option<u64>)>> {
        self.refresh_video_recovery_protection();
        let slot_count = self.layout.slot_count as usize;
        for offset in 0..slot_count {
            let index = (self.next_slot + offset) % slot_count;
            let slot = slot_control(&self.control, index);
            if slot
                .state
                .compare_exchange(
                    STATE_FREE,
                    STATE_WRITING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Ok(Some((index, None)));
            }
        }

        // A lane has at most MAX_SLOTS entries, so a bounded linear scan is
        // cheaper and more predictable than allocating and sorting a Vec on
        // every saturated publish. Retry after a lost CAS because the consumer
        // may have claimed the selected slot between the scan and transition.
        for _ in 0..slot_count {
            let mut oldest_ready: Option<(u64, usize)> = None;
            for index in 0..slot_count {
                let slot = slot_control(&self.control, index);
                if slot.state.load(Ordering::Acquire) != STATE_READY {
                    continue;
                }
                let ticket = slot.ticket.load(Ordering::Relaxed);
                if !replacing_with_video_recovery
                    && self.protected_video_recovery == Some((index, ticket))
                {
                    continue;
                }
                let candidate = (ticket, index);
                if oldest_ready.is_none_or(|oldest| candidate < oldest) {
                    oldest_ready = Some(candidate);
                }
            }
            let Some((old_ticket, index)) = oldest_ready else {
                return Ok(None);
            };
            let slot = slot_control(&self.control, index);
            if slot
                .state
                .compare_exchange(
                    STATE_READY,
                    STATE_WRITING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Ok(Some((index, Some(old_ticket))));
            }
        }
        Ok(None)
    }

    fn refresh_video_recovery_protection(&mut self) {
        let Some((index, ticket)) = self.protected_video_recovery else {
            return;
        };
        let slot = slot_control(&self.control, index);
        let state = slot.state.load(Ordering::Acquire);
        if !matches!(state, STATE_READY | STATE_READING)
            || slot.ticket.load(Ordering::Relaxed) != ticket
        {
            self.protected_video_recovery = None;
        }
    }
}

pub struct SharedMediaLaneConsumer {
    #[cfg(unix)]
    control: Arc<MmapMut>,
    #[cfg(windows)]
    control: Arc<WindowsMappedView>,
    #[cfg(unix)]
    data: Arc<Mmap>,
    #[cfg(windows)]
    data: Arc<WindowsMappedView>,
    layout: SharedMediaLaneLayout,
}

impl SharedMediaLaneConsumer {
    #[cfg(unix)]
    pub fn open(file: &File, expected_lane: MediaLane, expected_nonce: [u8; 16]) -> Result<Self> {
        let control = unsafe { MmapOptions::new().len(CONTROL_REGION_BYTES).map_mut(file)? };
        let layout = validate_header(&control, expected_lane, expected_nonce)?;
        let data = unsafe {
            MmapOptions::new()
                .offset(CONTROL_REGION_BYTES as u64)
                .len(data_region_len(layout)?)
                .map(file)?
        };
        Ok(Self {
            control: Arc::new(control),
            data: Arc::new(data),
            layout,
        })
    }

    #[cfg(windows)]
    pub fn open_named(
        name: &str,
        expected_lane: MediaLane,
        expected_nonce: [u8; 16],
    ) -> Result<Self> {
        let control = WindowsMappedView::open(
            name,
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            CONTROL_REGION_BYTES,
        )?;
        let layout = validate_header(&control, expected_lane, expected_nonce)?;
        let data = WindowsMappedView::open(
            name,
            FILE_MAP_READ,
            CONTROL_REGION_BYTES as u64,
            data_region_len(layout)?,
        )?;
        Ok(Self {
            control: Arc::new(control),
            data: Arc::new(data),
            layout,
        })
    }

    pub const fn layout(&self) -> SharedMediaLaneLayout {
        self.layout
    }

    pub fn claim(&self, ticket: SharedSlotTicket) -> Result<SharedMediaPayload> {
        let slot_index = ticket.slot_index as usize;
        if slot_index >= self.layout.slot_count as usize {
            return Err(SharedMediaLaneError::Invalid(
                "ticket slot index exceeds lane layout".into(),
            ));
        }
        let slot = slot_control(&self.control, slot_index);
        if slot
            .state
            .compare_exchange(
                STATE_READY,
                STATE_READING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Err(SharedMediaLaneError::SlotUnavailable);
        }
        let observed_ticket = slot.ticket.load(Ordering::Relaxed);
        let observed_sequence = slot.sequence.load(Ordering::Relaxed);
        if observed_ticket != ticket.ticket || observed_sequence != ticket.identity.sequence {
            slot.state.store(STATE_READY, Ordering::Release);
            return Err(SharedMediaLaneError::StaleTicket);
        }
        let frame_len = slot.frame_len.load(Ordering::Relaxed) as usize;
        if frame_len == 0 || frame_len > self.layout.slot_capacity as usize {
            slot.state.store(STATE_FREE, Ordering::Release);
            return Err(SharedMediaLaneError::Invalid(
                "claimed slot frame length violates the lane layout".into(),
            ));
        }
        let range = slot_range(self.layout, slot_index, frame_len)?;
        Ok(SharedMediaPayload {
            control: Arc::clone(&self.control),
            data: Arc::clone(&self.data),
            slot_index,
            range,
            ticket,
        })
    }
}

pub struct SharedMediaPayload {
    #[cfg(unix)]
    control: Arc<MmapMut>,
    #[cfg(windows)]
    control: Arc<WindowsMappedView>,
    #[cfg(unix)]
    data: Arc<Mmap>,
    #[cfg(windows)]
    data: Arc<WindowsMappedView>,
    slot_index: usize,
    range: Range<usize>,
    ticket: SharedSlotTicket,
}

impl SharedMediaPayload {
    pub const fn ticket(&self) -> SharedSlotTicket {
        self.ticket
    }
}

impl AsRef<[u8]> for SharedMediaPayload {
    fn as_ref(&self) -> &[u8] {
        &self.data[self.range.clone()]
    }
}

impl Drop for SharedMediaPayload {
    fn drop(&mut self) {
        let slot = slot_control(&self.control, self.slot_index);
        let _ = slot.state.compare_exchange(
            STATE_READING,
            STATE_FREE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

fn validate_header(
    control: &[u8],
    expected_lane: MediaLane,
    expected_nonce: [u8; 16],
) -> Result<SharedMediaLaneLayout> {
    if control.len() != CONTROL_REGION_BYTES
        || control.as_ptr().align_offset(align_of::<LaneHeader>()) != 0
    {
        return Err(SharedMediaLaneError::Invalid(
            "control mapping size or alignment is invalid".into(),
        ));
    }
    let header = header(control);
    if header.magic != MAGIC
        || header.version != VERSION
        || header.lane != lane_code(expected_lane)
        || header.reserved != 0
        || header.generation_nonce != expected_nonce
    {
        return Err(SharedMediaLaneError::Invalid(
            "header identity or generation fence mismatch".into(),
        ));
    }
    let layout = SharedMediaLaneLayout::new(
        expected_lane,
        header.slot_count,
        header.slot_capacity,
        expected_nonce,
    )?;
    if layout.slot_stride != header.slot_stride {
        return Err(SharedMediaLaneError::Invalid(
            "header slot stride is not canonical".into(),
        ));
    }
    Ok(layout)
}

fn header(control: &[u8]) -> &LaneHeader {
    unsafe { &*control.as_ptr().cast::<LaneHeader>() }
}

fn slot_control(control: &[u8], index: usize) -> &SlotControl {
    unsafe { &*slot_control_ptr(control, index) }
}

unsafe fn slot_control_mut_ptr(control: &mut [u8], index: usize) -> *mut SlotControl {
    unsafe {
        control
            .as_mut_ptr()
            .add(slot_controls_offset() + index * size_of::<SlotControl>())
            .cast::<SlotControl>()
    }
}

unsafe fn slot_control_ptr(control: &[u8], index: usize) -> *const SlotControl {
    unsafe {
        control
            .as_ptr()
            .add(slot_controls_offset() + index * size_of::<SlotControl>())
            .cast::<SlotControl>()
    }
}

fn slot_controls_offset() -> usize {
    align_up(size_of::<LaneHeader>(), align_of::<SlotControl>())
        .expect("fixed lane header alignment cannot overflow")
}

fn slot_range(
    layout: SharedMediaLaneLayout,
    slot_index: usize,
    frame_len: usize,
) -> Result<Range<usize>> {
    let start = slot_index
        .checked_mul(layout.slot_stride as usize)
        .ok_or_else(|| SharedMediaLaneError::Invalid("slot offset overflow".into()))?;
    let end = start
        .checked_add(frame_len)
        .ok_or_else(|| SharedMediaLaneError::Invalid("slot range overflow".into()))?;
    if end > data_region_len(layout)? {
        return Err(SharedMediaLaneError::Invalid(
            "slot range exceeds data mapping".into(),
        ));
    }
    Ok(start..end)
}

fn data_region_len(layout: SharedMediaLaneLayout) -> Result<usize> {
    (layout.slot_stride as usize)
        .checked_mul(layout.slot_count as usize)
        .ok_or_else(|| SharedMediaLaneError::Invalid("data mapping length overflow".into()))
}

fn lane_code(lane: MediaLane) -> u32 {
    match lane {
        MediaLane::Control => 0,
        MediaLane::Video => 1,
        MediaLane::Audio => 2,
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_session::{
        binary_media_frame_capacity, decode_binary_media_event_frame, GenerationFence, PROTOCOL,
        SCHEMA_VERSION,
    };
    use bytes::Bytes;

    const NONCE: [u8; 16] = [0x5a; 16];

    fn identity(sequence: u64) -> SharedFrameIdentity {
        SharedFrameIdentity {
            sequence,
            observed_at_ms: 1_000 + sequence,
            media_gate: 1,
        }
    }

    fn endpoints(slots: u32, capacity: u32) -> (SharedMediaLaneProducer, SharedMediaLaneConsumer) {
        let layout = SharedMediaLaneLayout::new(MediaLane::Video, slots, capacity, NONCE).unwrap();
        let lane = SharedMediaLaneFile::create(layout).unwrap();
        let producer =
            SharedMediaLaneProducer::open(&lane.try_clone_file().unwrap(), MediaLane::Video, NONCE)
                .unwrap();
        let consumer =
            SharedMediaLaneConsumer::open(&lane.try_clone_file().unwrap(), MediaLane::Video, NONCE)
                .unwrap();
        (producer, consumer)
    }

    fn video_body(keyframe: bool, sps_pps_present: bool) -> EventBody {
        EventBody::VideoH264 {
            media_gate: 1,
            pts_90khz: 72_000,
            duration_90khz: 3_000,
            keyframe,
            sps_pps_present,
            discontinuity: false,
            codec_generation: 1,
            width: 640,
            height: 360,
            encode_submitted_at_ms: 1_020,
            encoded_at_ms: 1_023,
        }
    }

    #[test]
    fn publish_claim_and_drop_release_one_borrowed_slot() {
        let (mut producer, consumer) = endpoints(1, 128);
        let published = producer.publish(identity(7), b"RVID-payload").unwrap();
        let SharedPublishOutcome::Published(ticket) = published else {
            panic!("first frame must publish");
        };
        let payload = consumer.claim(ticket).unwrap();
        assert_eq!(payload.as_ref(), b"RVID-payload");
        assert_eq!(payload.ticket(), ticket);
        assert_eq!(
            producer.publish(identity(8), b"held-slot").unwrap(),
            SharedPublishOutcome::Dropped {
                identity: identity(8)
            }
        );
        drop(payload);
        assert!(matches!(
            producer.publish(identity(9), b"reused-slot").unwrap(),
            SharedPublishOutcome::Published(_)
        ));
    }

    #[test]
    fn bytes_owner_borrows_the_mapping_and_releases_on_last_drop() {
        let (mut producer, consumer) = endpoints(1, 128);
        let SharedPublishOutcome::Published(ticket) = producer
            .publish(identity(20), b"borrowed-mapped-payload")
            .unwrap()
        else {
            panic!("frame must publish");
        };
        let payload = consumer.claim(ticket).unwrap();
        let mapped_ptr = payload.as_ref().as_ptr();
        let bytes = Bytes::from_owner(payload);
        assert_eq!(bytes.as_ptr(), mapped_ptr);
        assert_eq!(&bytes[..], b"borrowed-mapped-payload");
        assert!(matches!(
            producer.publish(identity(21), b"still-held").unwrap(),
            SharedPublishOutcome::Dropped { .. }
        ));
        let clone = bytes.clone();
        drop(bytes);
        assert!(matches!(
            producer.publish(identity(22), b"clone-held").unwrap(),
            SharedPublishOutcome::Dropped { .. }
        ));
        drop(clone);
        assert!(matches!(
            producer.publish(identity(23), b"released").unwrap(),
            SharedPublishOutcome::Published(_)
        ));
    }

    #[test]
    fn fixed_notification_claim_and_webrtc_bytes_keep_one_mapped_payload() {
        let fence = GenerationFence {
            process_generation: 7,
            build_id: "33".repeat(32),
            session_nonce: "5a".repeat(16),
            transport_epoch: 3,
            media_source_epoch: 5,
            contract_digest: "44".repeat(32),
        };
        let metadata = EventMetadata {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.into(),
            fence: fence.clone(),
            sequence: 24,
            observed_at_ms: 1_024,
            body: EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 72_000,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 1_020,
                encoded_at_ms: 1_023,
            },
        };
        let codec_payload = b"\0\0\0\x01\x67\x42\0\x1f\x80\0\0\0\x01\x68\0\0\0\x01\x65\0";
        let capacity = binary_media_frame_capacity(MediaLane::Video, codec_payload.len()).unwrap();
        let layout =
            SharedMediaLaneLayout::new(MediaLane::Video, 1, capacity as u32, NONCE).unwrap();
        let lane = SharedMediaLaneFile::create(layout).unwrap();
        let mut producer =
            SharedMediaLaneProducer::open(&lane.try_clone_file().unwrap(), MediaLane::Video, NONCE)
                .unwrap();
        let consumer =
            SharedMediaLaneConsumer::open(&lane.try_clone_file().unwrap(), MediaLane::Video, NONCE)
                .unwrap();

        let outcome = producer.publish_event(&metadata, codec_payload).unwrap();
        let mut notification_bytes = Vec::new();
        SharedSlotNotification::from(outcome)
            .write_to(&mut notification_bytes, MediaLane::Video)
            .unwrap();
        assert_eq!(notification_bytes.len(), SHARED_SLOT_NOTIFICATION_BYTES);
        assert!(!notification_bytes
            .windows(codec_payload.len())
            .any(|window| window == codec_payload));

        let notification =
            SharedSlotNotification::read_from(&mut &notification_bytes[..], MediaLane::Video)
                .unwrap()
                .unwrap();
        let SharedSlotNotification::Published(ticket) = notification else {
            panic!("media publish must produce a slot ticket");
        };
        let lease = consumer.claim(ticket).unwrap();
        let mapped_frame_ptr = lease.as_ref().as_ptr();
        let frame = Bytes::from_owner(lease);
        assert_eq!(frame.as_ptr(), mapped_frame_ptr);
        let (decoded, payload_view) =
            decode_binary_media_event_frame(&frame, MediaLane::Video, &fence).unwrap();
        assert_eq!(decoded, metadata);
        assert_eq!(payload_view, codec_payload);
        let payload_offset = payload_view.as_ptr() as usize - frame.as_ptr() as usize;
        let webrtc_bytes = frame.slice(payload_offset..payload_offset + payload_view.len());
        assert_eq!(webrtc_bytes.as_ptr(), payload_view.as_ptr());
        assert_eq!(&webrtc_bytes[..], codec_payload);
        drop(frame);
        assert!(matches!(
            producer.publish_event(&metadata, codec_payload).unwrap(),
            SharedPublishOutcome::Dropped { .. }
        ));
        drop(webrtc_bytes);
        assert!(matches!(
            producer.publish_event(&metadata, codec_payload).unwrap(),
            SharedPublishOutcome::Published(_)
        ));
    }

    #[test]
    fn ready_slot_replacement_invalidates_only_the_old_ticket() {
        let (mut producer, consumer) = endpoints(1, 128);
        let SharedPublishOutcome::Published(first) =
            producer.publish(identity(10), b"first").unwrap()
        else {
            panic!("first frame must publish");
        };
        let SharedPublishOutcome::Published(second) =
            producer.publish(identity(11), b"second").unwrap()
        else {
            panic!("replacement frame must publish");
        };
        assert_eq!(second.replaced_ticket, Some(first.ticket));
        assert!(matches!(
            consumer.claim(first),
            Err(SharedMediaLaneError::StaleTicket)
        ));
        let payload = consumer.claim(second).unwrap();
        assert_eq!(payload.as_ref(), b"second");
    }

    #[test]
    fn video_recovery_survives_ready_slot_saturation_until_claimed() {
        let recovery_payload = b"\0\0\0\x01\x67\x42\0\x1f\x80\0\0\0\x01\x68\0\0\0\x01\x65\0";
        let delta_payload = b"\0\0\0\x01\x41\x9a\x22";
        let capacity =
            binary_media_frame_capacity(MediaLane::Video, recovery_payload.len()).unwrap() as u32;
        let (mut producer, consumer) = endpoints(2, capacity);

        let SharedPublishOutcome::Published(recovery) = producer
            .publish_media_event(1, 2_001, &video_body(true, true), recovery_payload)
            .unwrap()
        else {
            panic!("recovery frame must publish");
        };
        for sequence in 2..20 {
            assert!(matches!(
                producer
                    .publish_media_event(
                        sequence,
                        2_000 + sequence,
                        &video_body(false, false),
                        delta_payload,
                    )
                    .unwrap(),
                SharedPublishOutcome::Published(_)
            ));
        }

        let recovery_lease = consumer
            .claim(recovery)
            .expect("delta replacement must preserve the recovery ticket");
        drop(recovery_lease);
        assert!(matches!(
            producer
                .publish_media_event(20, 2_020, &video_body(false, false), delta_payload)
                .unwrap(),
            SharedPublishOutcome::Published(_)
        ));
    }

    #[test]
    fn newer_video_recovery_can_supersede_an_unclaimed_recovery() {
        let recovery_payload = b"\0\0\0\x01\x67\x42\0\x1f\x80\0\0\0\x01\x68\0\0\0\x01\x65\0";
        let capacity =
            binary_media_frame_capacity(MediaLane::Video, recovery_payload.len()).unwrap() as u32;
        let (mut producer, consumer) = endpoints(1, capacity);
        let SharedPublishOutcome::Published(first) = producer
            .publish_media_event(1, 2_001, &video_body(true, true), recovery_payload)
            .unwrap()
        else {
            panic!("first recovery frame must publish");
        };
        let SharedPublishOutcome::Published(second) = producer
            .publish_media_event(2, 2_002, &video_body(true, true), recovery_payload)
            .unwrap()
        else {
            panic!("newer recovery frame must replace the older recovery");
        };

        assert_eq!(second.replaced_ticket, Some(first.ticket));
        assert!(matches!(
            consumer.claim(first),
            Err(SharedMediaLaneError::StaleTicket)
        ));
        consumer
            .claim(second)
            .expect("newest recovery must remain claimable");
    }

    #[test]
    fn mapping_open_rejects_the_wrong_generation_or_lane() {
        let layout = SharedMediaLaneLayout::new(MediaLane::Audio, 4, 2048, NONCE).unwrap();
        let lane = SharedMediaLaneFile::create(layout).unwrap();
        assert!(matches!(
            SharedMediaLaneConsumer::open(
                &lane.try_clone_file().unwrap(),
                MediaLane::Audio,
                [0x33; 16]
            ),
            Err(SharedMediaLaneError::Invalid(_))
        ));
        assert!(matches!(
            SharedMediaLaneProducer::open(&lane.try_clone_file().unwrap(), MediaLane::Video, NONCE),
            Err(SharedMediaLaneError::Invalid(_))
        ));
    }

    #[test]
    fn layout_is_strictly_bounded() {
        assert!(SharedMediaLaneLayout::new(MediaLane::Control, 1, 1, NONCE).is_err());
        assert!(SharedMediaLaneLayout::new(MediaLane::Video, 0, 1, NONCE).is_err());
        assert!(SharedMediaLaneLayout::new(MediaLane::Video, 9, 1, NONCE).is_err());
        assert!(SharedMediaLaneLayout::new(
            MediaLane::Video,
            1,
            (MAX_FRAME_BYTES + 1) as u32,
            NONCE
        )
        .is_err());
        assert!(SharedMediaLaneLayout::new(MediaLane::Video, 1, 128, [0; 16]).is_err());
    }

    #[test]
    fn fixed_notification_round_trip_preserves_publish_and_drop_identity() {
        let published = SharedSlotNotification::Published(SharedSlotTicket {
            slot_index: 2,
            ticket: 77,
            identity: identity(12),
            replaced_ticket: Some(71),
        });
        let mut bytes = Vec::new();
        published.write_to(&mut bytes, MediaLane::Video).unwrap();
        assert_eq!(bytes.len(), SHARED_SLOT_NOTIFICATION_BYTES);
        assert_eq!(
            SharedSlotNotification::read_from(&mut bytes.as_slice(), MediaLane::Video)
                .unwrap()
                .unwrap(),
            published
        );

        let dropped = SharedSlotNotification::Dropped {
            identity: identity(13),
        };
        let mut bytes = Vec::new();
        dropped.write_to(&mut bytes, MediaLane::Video).unwrap();
        assert_eq!(
            SharedSlotNotification::read_from(&mut bytes.as_slice(), MediaLane::Video)
                .unwrap()
                .unwrap(),
            dropped
        );
    }
}
