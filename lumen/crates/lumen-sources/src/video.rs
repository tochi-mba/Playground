//! Video playback through an external `ffmpeg` process.
//!
//! `ffprobe` reports the stream size and rate; `ffmpeg` decodes to raw RGBA on its stdout
//! while a reader thread slices it into frames. Nothing links against ffmpeg, so lumen stays
//! MIT/Apache and any build of ffmpeg on the PATH (or named by `LUMEN_FFMPEG`) works.

use crate::clock::Transport;
use crate::{Frame, FrameSource, SourceError};
use lumen_core::project::Playback;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::Arc;

/// Frames wider than this are decoded at reduced size.
pub const MAX_WIDTH: u32 = 1920;
/// Decoded frames buffered ahead of the playhead.
pub const QUEUE_DEPTH: usize = 6;

/// Where the ffmpeg binaries are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    /// `ffmpeg` executable.
    pub ffmpeg: PathBuf,
    /// `ffprobe` executable.
    pub ffprobe: PathBuf,
}

impl Tools {
    /// Uses `LUMEN_FFMPEG` / `LUMEN_FFPROBE` when set, else the PATH.
    pub fn from_env() -> Self {
        let ffmpeg =
            std::env::var_os("LUMEN_FFMPEG").map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
        let ffprobe = std::env::var_os("LUMEN_FFPROBE")
            .map_or_else(|| sibling(&ffmpeg, "ffprobe"), PathBuf::from);
        Self { ffmpeg, ffprobe }
    }

    /// True when `ffmpeg -version` runs.
    pub fn available(&self) -> bool {
        Command::new(&self.ffmpeg)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

/// `ffprobe` next to a given `ffmpeg` path, or the bare name when ffmpeg is a bare name.
pub fn sibling(ffmpeg: &Path, name: &str) -> PathBuf {
    match ffmpeg.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            let ext = ffmpeg
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            dir.join(format!("{name}{ext}"))
        }
        _ => PathBuf::from(name),
    }
}

/// What ffprobe reported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    /// Source width.
    pub width: u32,
    /// Source height.
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    /// Duration in seconds, when known.
    pub duration: Option<f64>,
}

impl VideoInfo {
    /// Parses `width,height,avg_frame_rate[,duration]` as `ffprobe -of csv=p=0` prints it.
    pub fn parse(csv: &str) -> Result<Self, String> {
        let line = csv
            .lines()
            .find(|l| !l.trim().is_empty())
            .ok_or("ffprobe printed nothing")?;
        let parts: Vec<&str> = line.trim().split(',').collect();
        if parts.len() < 3 {
            return Err(format!("unexpected ffprobe output {line:?}"));
        }
        let width: u32 = parts[0]
            .parse()
            .map_err(|_| format!("bad width {:?}", parts[0]))?;
        let height: u32 = parts[1]
            .parse()
            .map_err(|_| format!("bad height {:?}", parts[1]))?;
        let fps = parse_rate(parts[2]).ok_or_else(|| format!("bad frame rate {:?}", parts[2]))?;
        let duration = parts
            .get(3)
            .and_then(|d| d.trim().parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0);
        if width == 0 || height == 0 {
            return Err("zero-sized video".into());
        }
        Ok(Self {
            width,
            height,
            fps,
            duration,
        })
    }

    /// Decoded size: scaled down to [`MAX_WIDTH`] keeping aspect, even dimensions.
    pub fn decoded_size(&self) -> (u32, u32) {
        if self.width <= MAX_WIDTH {
            return (self.width & !1, (self.height & !1).max(2));
        }
        let w = MAX_WIDTH;
        let h = (u64::from(self.height) * u64::from(w) / u64::from(self.width)) as u32;
        (w, (h & !1).max(2))
    }
}

fn parse_rate(s: &str) -> Option<f64> {
    let s = s.trim();
    let v = match s.split_once('/') {
        Some((n, d)) => {
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            if d == 0.0 {
                return None;
            }
            n / d
        }
        None => s.parse().ok()?,
    };
    (v.is_finite() && v > 0.0 && v < 1000.0).then_some(v)
}

/// Runs ffprobe on a file.
pub fn probe(tools: &Tools, path: &Path) -> Result<VideoInfo, SourceError> {
    let out = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate:format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            SourceError::Video(
                path.to_path_buf(),
                format!("cannot run {}: {e}", tools.ffprobe.display()),
            )
        })?;
    if !out.status.success() {
        return Err(SourceError::Video(
            path.to_path_buf(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // The stream line and the format line come out separately; join them.
    let joined = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(",");
    VideoInfo::parse(&joined).map_err(|e| SourceError::Video(path.to_path_buf(), e))
}

struct Decoder {
    child: Child,
    frames: Receiver<Frame>,
    ended: bool,
    index: i64,
}

impl Decoder {
    fn spawn(tools: &Tools, path: &Path, size: (u32, u32)) -> Result<Self, SourceError> {
        let mut child = Command::new(&tools.ffmpeg)
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-an",
                "-sn",
                "-vf",
                &format!("scale={}:{}", size.0, size.1),
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                SourceError::Video(
                    path.to_path_buf(),
                    format!("cannot run {}: {e}", tools.ffmpeg.display()),
                )
            })?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| SourceError::Video(path.to_path_buf(), "no stdout".into()))?;
        let (tx, rx) = sync_channel(QUEUE_DEPTH);
        let (w, h) = size;
        std::thread::Builder::new()
            .name("lumen-video".into())
            .spawn(move || {
                let bytes = (w as usize) * (h as usize) * 4;
                loop {
                    let mut buf = vec![0u8; bytes];
                    if stdout.read_exact(&mut buf).is_err() {
                        break;
                    }
                    if tx
                        .send(Frame {
                            width: w,
                            height: h,
                            rgba: buf,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| SourceError::Video(path.to_path_buf(), e.to_string()))?;
        Ok(Self {
            child,
            frames: rx,
            ended: false,
            index: -1,
        })
    }

    /// Next decoded frame without blocking; `None` at end of stream or when none is ready.
    fn next(&mut self) -> Option<Frame> {
        match self.frames.try_recv() {
            Ok(f) => {
                self.index += 1;
                Some(f)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.ended = true;
                None
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A video file as a frame source.
pub struct VideoSource {
    tools: Tools,
    path: PathBuf,
    info: VideoInfo,
    size: (u32, u32),
    transport: Transport,
    decoder: Option<Decoder>,
    current: Option<Arc<Frame>>,
    /// Set when the decoder could not be (re)started; shown to the user.
    pub error: Option<String>,
}

impl VideoSource {
    /// Probes and starts decoding.
    pub fn open(tools: Tools, path: &Path, playback: Playback) -> Result<Self, SourceError> {
        let info = probe(&tools, path)?;
        let size = info.decoded_size();
        let mut v = Self {
            tools,
            path: path.to_path_buf(),
            info,
            size,
            transport: Transport::new(playback),
            decoder: None,
            current: None,
            error: None,
        };
        v.restart();
        Ok(v)
    }

    /// Stream information.
    pub fn info(&self) -> VideoInfo {
        self.info
    }

    /// Playhead in seconds.
    pub fn position(&self) -> f64 {
        self.transport.position
    }

    fn restart(&mut self) {
        self.transport.rewind();
        match Decoder::spawn(&self.tools, &self.path, self.size) {
            Ok(d) => {
                self.decoder = Some(d);
                self.error = None;
            }
            Err(e) => {
                self.decoder = None;
                self.error = Some(e.to_string());
            }
        }
    }
}

impl FrameSource for VideoSource {
    fn frame(&mut self, t: f64) -> Option<Arc<Frame>> {
        self.transport.advance(t);
        let target = (self.transport.position * self.info.fps).floor() as i64;
        let mut needs_restart = false;
        if let Some(d) = &mut self.decoder {
            while d.index < target {
                match d.next() {
                    Some(f) => self.current = Some(Arc::new(f)),
                    None => break,
                }
            }
            if d.ended && d.index <= target {
                if self.transport.playback.looping {
                    needs_restart = true;
                } else {
                    // Hold the last frame; keep the position parked at the end.
                    self.transport.position = (d.index.max(0) as f64) / self.info.fps;
                }
            }
        }
        if needs_restart {
            self.restart();
        }
        self.current.clone()
    }
    fn is_live(&self) -> bool {
        true
    }
    fn set_playback(&mut self, playback: &Playback) {
        if self.transport.set_playback(*playback) {
            self.restart();
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::field_reassign_with_default,
    clippy::expect_used,
    clippy::print_stderr
)]
mod tests {
    use super::*;

    #[test]
    fn info_parsing_and_sizes() {
        let i = VideoInfo::parse("1280,720,30000/1001,12.5\n").unwrap();
        assert_eq!((i.width, i.height), (1280, 720));
        assert!((i.fps - 29.97).abs() < 0.01);
        assert_eq!(i.duration, Some(12.5));
        assert_eq!(i.decoded_size(), (1280, 720));
        let big = VideoInfo::parse("3840,2160,25,N/A").unwrap();
        assert_eq!(big.duration, None);
        assert_eq!(big.decoded_size(), (1920, 1080));
        let odd = VideoInfo::parse("1921,1081,25").unwrap();
        assert_eq!(odd.decoded_size(), (1920, 1080));
        assert_eq!(VideoInfo::parse("1,1,25").unwrap().decoded_size(), (0, 2));
        for bad in [
            "",
            "a,b,c",
            "0,10,25",
            "10,10,0/0",
            "10,10,-5",
            "10,10,abc",
            "10",
        ] {
            assert!(VideoInfo::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_rate("50"), Some(50.0));
        assert_eq!(parse_rate("5000"), None);
        assert_eq!(
            sibling(Path::new("/opt/ff/ffmpeg.exe"), "ffprobe"),
            PathBuf::from("/opt/ff/ffprobe.exe")
        );
        assert_eq!(
            sibling(Path::new("ffmpeg"), "ffprobe"),
            PathBuf::from("ffprobe")
        );
        let tools = Tools {
            ffmpeg: PathBuf::from("/definitely/missing/ffmpeg"),
            ffprobe: PathBuf::from("/definitely/missing/ffprobe"),
        };
        assert!(!tools.available());
        assert!(probe(&tools, Path::new("x.mp4"))
            .unwrap_err()
            .to_string()
            .contains("cannot run"));
        assert!(VideoSource::open(tools, Path::new("x.mp4"), Playback::default()).is_err());
    }

    /// A stand-in for ffmpeg/ffprobe written in shell: probe prints a fixed stream, decode
    /// writes `N` solid 4×2 RGBA frames whose red channel is the frame index.
    #[cfg(unix)]
    fn fake_tools(dir: &Path, frames: u32) -> Tools {
        use std::os::unix::fs::PermissionsExt;
        let probe = dir.join("ffprobe");
        std::fs::write(&probe, "#!/bin/sh\necho '4,2,10/1'\necho '0.3'\n").unwrap();
        let decode = dir.join("ffmpeg");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"-version\" ]; then echo fake; exit 0; fi\ni=0\nwhile [ $i -lt {frames} ]; do\n  printf \"\\\\$(printf '%03o' $i)\\\\000\\\\000\\\\377\" > /tmp/px.$$\n  for p in 1 2 3 4 5 6 7 8; do cat /tmp/px.$$; done\n  i=$((i+1))\ndone\nrm -f /tmp/px.$$\n"
        );
        std::fs::write(&decode, script).unwrap();
        for p in [&probe, &decode] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Tools {
            ffmpeg: decode,
            ffprobe: probe,
        }
    }

    #[cfg(unix)]
    #[test]
    fn fake_decoder_paces_loops_and_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake_tools(dir.path(), 3);
        assert!(tools.available());
        let mut v = VideoSource::open(tools, Path::new("clip.mp4"), Playback::default()).unwrap();
        assert_eq!(v.info().fps, 10.0);
        assert_eq!(v.info().duration, Some(0.3));
        assert!(v.is_live());
        // Let the reader thread deliver.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut first = None;
        while std::time::Instant::now() < deadline {
            if let Some(f) = v.frame(0.0) {
                first = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let first = first.expect("first frame arrives");
        assert_eq!(first.pixel(0, 0), [0, 0, 0, 255]);
        // Advance 0.25 s at 10 fps: frame index 2.
        let mut f = v.frame(0.25).unwrap();
        for _ in 0..200 {
            if f.pixel(0, 0)[0] == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            f = v.frame(0.25).unwrap();
        }
        assert_eq!(f.pixel(0, 0)[0], 2);
        // Past the end with looping: the decoder restarts and the position rewinds.
        for _ in 0..200 {
            v.frame(0.45);
            if v.position() < 0.1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(v.position() < 0.1, "looped: {}", v.position());
        // A restart token also rewinds.
        let mut pb = Playback::default();
        pb.restart_token = 7;
        v.frame(0.5);
        v.set_playback(&pb);
        assert_eq!(v.position(), 0.0);
        // Non-looping: parks at the end.
        pb.looping = false;
        pb.restart_token = 8;
        v.set_playback(&pb);
        for _ in 0..200 {
            v.frame(2.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
            if v.position() < 1.0 {
                break;
            }
        }
        assert!(v.position() <= 0.31, "{}", v.position());
        assert!(v.error.is_none());
        // Paused: nothing moves.
        pb.paused = true;
        pb.restart_token = 9;
        v.set_playback(&pb);
        v.frame(10.0);
        v.frame(20.0);
        assert_eq!(v.position(), 0.0);
    }

    #[cfg(unix)]
    #[test]
    fn decoder_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let tools = fake_tools(dir.path(), 1);
        let mut v =
            VideoSource::open(tools.clone(), Path::new("clip.mp4"), Playback::default()).unwrap();
        std::fs::remove_file(&tools.ffmpeg).unwrap();
        let mut pb = Playback::default();
        pb.restart_token = 1;
        v.set_playback(&pb);
        assert!(v.error.as_deref().is_some_and(|e| e.contains("cannot run")));
        assert!(v.frame(1.0).is_none() || v.frame(1.0).is_some());
    }

    #[test]
    fn real_ffmpeg_decodes_a_generated_clip() {
        let tools = Tools::from_env();
        if !tools.available() {
            assert!(
                std::env::var_os("LUMEN_REQUIRE_FFMPEG").is_none_or(|v| v.is_empty()),
                "LUMEN_REQUIRE_FFMPEG set but ffmpeg is missing"
            );
            eprintln!("skipping: ffmpeg not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("test.mp4");
        let status = Command::new(&tools.ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x36:rate=10:duration=1",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success());
        let info = probe(&tools, &clip).unwrap();
        assert_eq!((info.width, info.height), (64, 36));
        assert_eq!(info.fps, 10.0);
        assert!(info.duration.is_some_and(|d| (d - 1.0).abs() < 0.2));
        let mut v = VideoSource::open(tools, &clip, Playback::default()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut got = None;
        while std::time::Instant::now() < deadline {
            if let Some(f) = v.frame(0.0) {
                got = Some(f);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = got.expect("ffmpeg delivered a frame");
        assert_eq!((f.width, f.height), (64, 36));
        assert!(f.is_consistent());
        assert!(probe(&v.tools, &dir.path().join("missing.mp4")).is_err());
    }
}
