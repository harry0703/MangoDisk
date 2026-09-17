//! A short-lived, unelevated companion owns Explorer layout mutations. Closing
//! the private stdin pipe (including abrupt GUI-process death) restores its lease.
//! The native window thread only exchanges bounded snapshots; it never waits on
//! another process or Explorer's UI thread.
use super::{layout::Bounds, position::Edge};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::Manager;

const SWITCH: &str = "--taskbar-layout-helper";
const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub shell: usize,
    pub parent: usize,
    pub host: Bounds,
    pub width: i32,
    pub height: i32,
    pub gap: i32,
    pub edge: Edge,
}
impl Request {
    pub fn new(
        shell: usize,
        parent: usize,
        host: Bounds,
        size: (i32, i32),
        gap: i32,
        edge: Edge,
    ) -> Self {
        Self {
            version: VERSION,
            shell,
            parent,
            host,
            width: size.0,
            height: size.1,
            gap,
            edge,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Spawn,
    Channel,
    Protocol,
    Host,
    Geometry,
    Space,
    Position,
    Xaml,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub stage: Stage,
    pub code: u32,
}
impl Failure {
    pub fn new(stage: Stage, code: u32) -> Self {
        Self { stage, code }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u32,
    result: Result<Bounds, Failure>,
}
type Snapshot = Option<(Request, Result<Bounds, Failure>, Instant)>;

pub struct Client {
    requests: mpsc::SyncSender<Request>,
    latest: Arc<Mutex<Snapshot>>,
}
impl Client {
    pub fn start(app: &tauri::AppHandle) -> Result<Self, Failure> {
        let executable = std::env::current_exe()
            .map_err(|e| Failure::new(Stage::Spawn, e.raw_os_error().unwrap_or(0) as u32))?;
        let log = app
            .path()
            .app_log_dir()
            .map_err(|_| Failure::new(Stage::Spawn, 0))?
            .join("MangoDisk.log");
        let mut child = Command::new(executable)
            .arg(SWITCH)
            .arg(log)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW, including debug builds.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Failure::new(Stage::Spawn, e.raw_os_error().unwrap_or(0) as u32))?;
        let mut input = child.stdin.take().expect("piped stdin");
        let mut output = BufReader::new(child.stdout.take().expect("piped stdout"));
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let latest: Arc<Mutex<Snapshot>> = Arc::new(Mutex::new(None));
        let snapshot = latest.clone();
        let pid = child.id();
        log::info!("resident_taskbar_layout_helper_started pid={pid}");
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let started = Instant::now();
                let exchange = (|| -> Result<Bounds, Failure> {
                    let packet = serde_json::to_vec(&request)
                        .map_err(|_| Failure::new(Stage::Protocol, 0))?;
                    input
                        .write_all(&packet)
                        .and_then(|_| input.write_all(b"\n"))
                        .and_then(|_| input.flush())
                        .map_err(|e| {
                            Failure::new(Stage::Channel, e.raw_os_error().unwrap_or(0) as u32)
                        })?;
                    let reply: Reply =
                        read_packet(&mut output).map_err(|_| Failure::new(Stage::Channel, 0))?;
                    if reply.version != VERSION {
                        return Err(Failure::new(Stage::Protocol, 0));
                    }
                    reply.result
                })();
                *snapshot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((request, exchange, started));
                if matches!(
                    exchange,
                    Err(Failure {
                        stage: Stage::Channel | Stage::Protocol,
                        ..
                    })
                ) {
                    break;
                }
            }
            // Never kill the companion: EOF is its restoration signal. Waiting
            // happens on this worker, not on a Tauri/native message-loop thread.
            drop(input);
            let result = child.wait();
            log::info!("resident_taskbar_layout_helper_stopped pid={pid} status={result:?}");
        });
        Ok(Self { requests, latest })
    }
    /// A live lease supplies authoritative placement even when UIA is slow.
    /// Never reuse it after a shell change, a failed exchange, or a stalled helper.
    pub fn has_recent_layout(&self, shell: usize, parent: usize) -> bool {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|(key, result, sampled)| {
                key.shell == shell
                    && key.parent == parent
                    && result.is_ok()
                    && sampled.elapsed() < Duration::from_secs(3)
            })
    }

    pub fn request(&self, request: Request) -> Option<Result<Bounds, Failure>> {
        if matches!(
            self.requests.try_send(request),
            Err(mpsc::TrySendError::Disconnected(_))
        ) {
            return Some(Err(Failure::new(Stage::Channel, 0)));
        }
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .filter(|(key, _, sampled)| {
                *key == request && sampled.elapsed() < Duration::from_secs(3)
            })
            .map(|(_, result, _)| result)
    }
}

fn read_packet<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> std::io::Result<T> {
    let mut line = String::new();
    // Stop malformed/oversized input before an unbounded allocation. No command,
    // file operation or arbitrary HWND is accepted by the layout implementation.
    reader.take(4096).read_line(&mut line)?;
    if !line.ends_with('\n') {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    serde_json::from_str(&line).map_err(|_| std::io::ErrorKind::InvalidData.into())
}

pub fn run_helper_mode(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<i32> {
    let mut args = args.into_iter().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(SWITCH)) {
        return None;
    }
    let Some(log_file) = args.next().map(PathBuf::from) else {
        return Some(2);
    };
    if args.next().is_some() || log_file.file_name() != Some(std::ffi::OsStr::new("MangoDisk.log"))
    {
        return Some(2);
    }
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let Some(_ownership) = super::reservation_windows::Ownership::acquire() else {
        return Some(3);
    };
    let modern = matches!(
        super::position::read_environment(),
        super::position::Environment::Windows11Centered
            | super::position::Environment::Windows11LeftAligned
    );
    let mut lease = super::reservation_windows::Lease::new(log_file.clone());
    let xaml = modern.then(|| super::reservation_xaml::Lease::new(&log_file));
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Ok(request) = read_packet::<Request>(&mut input) {
        if request.version != VERSION {
            break;
        }
        let reply = Reply {
            version: VERSION,
            result: match &xaml {
                Some(Ok(xaml)) => xaml.apply(request),
                Some(Err(error)) => Err(*error),
                None => unsafe { lease.apply(request) },
            },
        };
        if serde_json::to_writer(&mut output, &reply).is_err()
            || output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .is_err()
        {
            break;
        }
    }
    // Runs on EOF, invalid protocol and broken reply pipes alike, including when
    // the GUI dies between a successful MoveWindow and receiving its reply.
    drop(lease);
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_reuse_rejects_stale_failed_and_replaced_shells() {
        let (requests, _receiver) = mpsc::sync_channel(1);
        let key = Request::new(1, 2, Bounds::default(), (100, 40), 4, Edge::Left);
        let latest = Arc::new(Mutex::new(Some((
            key,
            Ok(Bounds::default()),
            Instant::now(),
        ))));
        let client = Client {
            requests,
            latest: latest.clone(),
        };
        assert!(client.has_recent_layout(1, 2));
        assert!(!client.has_recent_layout(3, 2));
        assert!(!client.has_recent_layout(1, 3));
        *latest.lock().unwrap() = Some((
            key,
            Ok(Bounds::default()),
            Instant::now() - Duration::from_secs(4),
        ));
        assert!(!client.has_recent_layout(1, 2));
        assert!(client.request(key).is_none());
        *latest.lock().unwrap() = Some((key, Err(Failure::new(Stage::Xaml, 1)), Instant::now()));
        assert!(!client.has_recent_layout(1, 2));
    }

    #[test]
    fn private_protocol_rejects_truncation_and_oversized_packets() {
        for bytes in [b"{}".to_vec(), vec![b' '; 5000]] {
            assert!(read_packet::<Request>(&mut std::io::Cursor::new(bytes)).is_err());
        }
    }
}
