use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_env = "msvc")]
use std::{os::windows::process::CommandExt as _, process::Stdio};

use eframe::egui::{self, Color32, ColorImage, Context, TextureHandle, Ui};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use uuid::Uuid;

use super::{
    model::{PhotoCopy, Result},
    store::Settings,
};

const TICK: Duration = Duration::from_millis(80);
const START_LIMIT: Duration = Duration::from_secs(6);
const FRESH_LIMIT: Duration = Duration::from_millis(850);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Camera {
    pub(super) id: String,
    pub(super) name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unavailable {
    NoCamera,
    ChooseCamera,
    Disconnected,
}

type OpenReply = std::result::Result<Camera, Unavailable>;

#[derive(Clone, Copy)]
enum Request<'a> {
    List,
    Preview(Option<&'a str>),
}

#[derive(Clone)]
struct Frame {
    width: usize,
    height: usize,
    bgra: Arc<[u8]>,
    at: Instant,
}

impl Frame {
    fn image(&self) -> ColorImage {
        ColorImage::new(
            [self.width, self.height],
            self.bgra
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&[b, g, r, _]| Color32::from_rgb(r, g, b))
                .collect(),
        )
    }

    fn photo(&self) -> Result<PhotoCopy> {
        let mut jpg = Vec::new();
        let rgb: Vec<_> = self
            .bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[b, g, r, _]| [r, g, b])
            .collect();
        JpegEncoder::new_with_quality(&mut jpg, 90).encode(
            &rgb,
            u32::try_from(self.width)?,
            u32::try_from(self.height)?,
            ExtendedColorType::Rgb8,
        )?;
        let clean = gydt_results_compiler::Photo::from_jpeg_bytes("jpg", jpg)?;
        Ok(PhotoCopy::new(
            "Webcam photo.jpg",
            "jpg",
            &clean.into_jpeg_bytes(),
        ))
    }
}

#[derive(Default)]
struct Mailbox {
    devices: Option<Vec<Camera>>,
    opened: Option<OpenReply>,
    latest: Option<Frame>,
    ended: bool,
}

struct Broker {
    child: Child,
    mailbox: Arc<Mutex<Mailbox>>,
    started: Instant,
}

fn word(pipe: &mut impl Read) -> io::Result<usize> {
    let mut v = [0; 4];
    pipe.read_exact(&mut v)?;
    Ok(u32::from_le_bytes(v) as usize)
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid camera response")
}

fn string(pipe: &mut impl Read) -> io::Result<String> {
    let size = word(pipe)?;
    if !(1..=4096).contains(&size) {
        return Err(invalid());
    }
    let mut data = vec![0; size];
    pipe.read_exact(&mut data)?;
    let text = String::from_utf8(data).map_err(|_| invalid())?;
    if text.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(text)
}

fn devices(pipe: &mut impl Read) -> io::Result<Vec<Camera>> {
    let count = word(pipe)?;
    if count > 32 {
        return Err(invalid());
    }
    let mut values: Vec<Camera> = Vec::with_capacity(count);
    for _ in 0..count {
        let id = string(pipe)?;
        let name = string(pipe)?;
        if values.iter().any(|item| item.id == id) {
            return Err(invalid());
        }
        values.push(Camera { id, name });
    }
    Ok(values)
}

fn open_reply(pipe: &mut impl Read) -> io::Result<OpenReply> {
    Ok(match word(pipe)? {
        0 => Ok(Camera {
            id: string(pipe)?,
            name: string(pipe)?,
        }),
        1 => Err(Unavailable::NoCamera),
        2 => Err(Unavailable::ChooseCamera),
        3 => Err(Unavailable::Disconnected),
        _ => return Err(invalid()),
    })
}

fn frame(pipe: &mut impl Read) -> io::Result<Frame> {
    let width = word(pipe)?;
    let height = word(pipe)?;
    if !(1..=640).contains(&width) || !(1..=480).contains(&height) {
        return Err(invalid());
    }
    let mut data = vec![0; width * height * 4];
    pipe.read_exact(&mut data)?;
    Ok(Frame {
        width,
        height,
        bgra: data.into(),
        at: Instant::now(),
    })
}

#[cfg(target_env = "msvc")]
fn camera_child(settings: &Path, request: Request<'_>) -> io::Result<Child> {
    if matches!(request, Request::Preview(Some(""))) {
        return Err(invalid());
    }
    let helper = super::camera_helper::prepare(settings)?;
    let system = winsafe::GetSystemDirectory().map_err(io::Error::other)?;
    let mut command = std::process::Command::new(&helper.path);
    command
        .current_dir(system)
        .arg(std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match request {
        Request::List => {
            command.arg("list");
        }
        Request::Preview(id) => {
            command.arg("preview");
            if let Some(id) = id {
                command.arg(id);
            }
        }
    }
    let child = command.spawn();
    drop(helper);
    child
}

#[cfg(not(target_env = "msvc"))]
fn camera_child(_settings: &Path, _request: Request<'_>) -> io::Result<Child> {
    Err(io::ErrorKind::Unsupported.into())
}

impl Broker {
    fn start(settings: &Path, request: Request<'_>) -> io::Result<Self> {
        let stream = matches!(request, Request::Preview(_));
        let mut child = camera_child(settings, request)?;
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let mut output = child.stdout.take().ok_or_else(invalid)?;
        let mut input = child.stdin.take().ok_or_else(invalid)?;
        let copy = Arc::clone(&mailbox);
        let now = Instant::now();
        if let Err(err) = thread::Builder::new()
            .name("webcam-pipe".into())
            .spawn(move || {
                if stream {
                    let success = if let Ok(reply) = open_reply(&mut output)
                        && let Ok(mut state) = copy.lock()
                    {
                        let started = reply.is_ok();
                        state.opened = Some(reply);
                        started
                    } else {
                        false
                    };
                    if success {
                        while input.write_all(b"f").is_ok() {
                            let Ok(current) = frame(&mut output) else {
                                break;
                            };
                            if let Ok(mut state) = copy.lock() {
                                state.latest = Some(current);
                            }
                            thread::sleep(TICK);
                        }
                    }
                } else if let Ok(list) = devices(&mut output)
                    && let Ok(mut state) = copy.lock()
                {
                    state.devices = Some(list);
                }
                if let Ok(mut state) = copy.lock() {
                    state.latest = None;
                    state.ended = true;
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        Ok(Self {
            child,
            mailbox,
            started: now,
        })
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
    }
    fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        if !self.exited() {
            self.stop();
        }
    }
}

/// Settings lists device names, but never starts a preview.
pub(super) struct SettingsCameras {
    root: PathBuf,
    broker: Option<Broker>,
    listed: Option<Vec<Camera>>,
    failed: bool,
}

impl SettingsCameras {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            broker: None,
            listed: None,
            failed: false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.broker = None;
        self.listed = None;
        self.failed = false;
    }

    fn refresh(&mut self) {
        self.clear();
        match Broker::start(&self.root, Request::List) {
            Ok(broker) => self.broker = Some(broker),
            Err(_) => self.failed = true,
        }
    }

    fn receive(&mut self, ctx: &Context, settings: &mut Settings) {
        let Some(broker) = &mut self.broker else {
            return;
        };
        ctx.request_repaint_after(TICK);
        let Some((devices, ended)) = broker
            .mailbox
            .lock()
            .ok()
            .map(|mut state| (state.devices.take(), state.ended))
        else {
            return;
        };
        if let Some(devices) = devices {
            if let [camera] = devices.as_slice()
                && settings.webcam_camera.is_none()
            {
                set_selection(settings, camera);
            }
            self.listed = Some(devices);
            self.broker = None;
        } else if ended || broker.started.elapsed() > START_LIMIT {
            self.broker = None;
            self.failed = true;
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, settings: &mut Settings) {
        self.receive(ui.ctx(), settings);
        let devices = self.listed.as_deref().unwrap_or_default();
        let mut selection = None;
        let mut refresh = false;
        let saved = settings.webcam_camera.as_deref();
        let show_disconnected = self.listed.is_some()
            && saved.is_some_and(|id| devices.iter().all(|camera| camera.id != id));
        let name = devices
            .iter()
            .find(|camera| Some(camera.id.as_str()) == saved)
            .map(|camera| camera.name.as_str())
            .or(settings.webcam_camera_name.as_deref())
            .unwrap_or(if saved.is_some() {
                "Previously selected camera"
            } else {
                "Choose camera"
            });
        let label = if show_disconnected {
            format!("{name} (disconnected)")
        } else {
            name.to_owned()
        };

        ui.horizontal_wrapped(|ui| {
            let field = ui.label("Camera");
            let combo = egui::ComboBox::from_id_salt("settings camera")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if devices.is_empty() {
                        ui.label(if self.failed {
                            "Camera discovery failed. Use Refresh to retry."
                        } else if self.listed.is_some() {
                            "No color camera found."
                        } else {
                            "Looking for color cameras…"
                        });
                    }
                    for (i, camera) in devices.iter().enumerate() {
                        let name = if devices
                            .iter()
                            .filter(|other| other.name == camera.name)
                            .count()
                            > 1
                        {
                            format!("{} ({})", camera.name, i + 1)
                        } else {
                            camera.name.clone()
                        };
                        if ui
                            .selectable_label(saved == Some(camera.id.as_str()), name)
                            .clicked()
                        {
                            selection = Some(camera.clone());
                        }
                    }
                });
            let _response = combo.response.labelled_by(field.id);
            let clicked_refresh = ui
                .add_enabled(self.broker.is_none(), egui::Button::new("Refresh"))
                .clicked();
            refresh = clicked_refresh
                || (combo.inner.is_some()
                    && self.listed.is_none()
                    && self.broker.is_none()
                    && !self.failed);
        });
        if let Some(selection) = selection {
            set_selection(settings, &selection);
        }
        if refresh {
            self.refresh();
            ui.ctx().request_repaint_after(TICK);
        }
        if self.failed {
            ui.label("Can’t find cameras. You can still attach a JPG.");
        } else if show_disconnected {
            ui.label("Reconnect this camera or choose another, then save your settings.");
        }
    }
}

pub(super) fn set_selection(settings: &mut Settings, camera: &Camera) {
    settings.webcam_camera = Some(camera.id.clone());
    settings.webcam_camera_name = Some(camera.name.clone());
}

/// One explicitly opened preview bound to a single sample and camera. Reconnect
/// requests the same camera. Only Settings can switch a saved camera identity.
pub(super) struct Preview {
    pub(super) sample: Uuid,
    root: PathBuf,
    selected: Option<String>,
    active: Option<Camera>,
    broker: Option<Broker>,
    start_requested: bool,
    latest: Option<Frame>,
    captured: Option<Frame>,
    accept: Option<Instant>,
    texture: Option<TextureHandle>,
    message: &'static str,
}

pub(super) enum Action {
    Keep,
    Cancel,
    Browse,
    Settings,
    Remember(Camera),
    Use(PhotoCopy),
}

impl Preview {
    pub(super) fn new(sample: Uuid, selected: Option<String>, root: PathBuf) -> Self {
        Self {
            sample,
            selected,
            root,
            active: None,
            broker: None,
            start_requested: true,
            latest: None,
            captured: None,
            accept: None,
            texture: None,
            message: "Starting live preview…",
        }
    }

    pub(super) fn matches_target(&self, current: Option<Uuid>) -> bool {
        current == Some(self.sample)
    }

    fn reset_frame(&mut self) {
        self.texture = None;
        self.latest = None;
        self.captured = None;
        self.accept = None;
    }

    fn fail(&mut self, message: &'static str) {
        self.reset_frame();
        self.active = None;
        self.start_requested = false;
        self.broker = None;
        self.message = message;
    }

    fn opened(&mut self, reply: OpenReply) -> Option<Camera> {
        match reply {
            Ok(camera) if self.selected.as_ref().is_none_or(|id| id == &camera.id) => {
                self.selected = Some(camera.id.clone());
                self.active = Some(camera.clone());
                Some(camera)
            }
            Ok(_) | Err(Unavailable::Disconnected) => {
                self.fail("The selected camera is disconnected. Reconnect it and try again, or choose another camera in Settings.");
                None
            }
            Err(Unavailable::NoCamera) => {
                self.fail("No color camera found. Connect one or attach a JPG.");
                None
            }
            Err(Unavailable::ChooseCamera) => {
                self.fail("Choose a camera in Settings to start live preview.");
                None
            }
        }
    }

    fn tick(&mut self, ctx: &Context) -> Option<Camera> {
        if std::mem::take(&mut self.start_requested) {
            match Broker::start(&self.root, Request::Preview(self.selected.as_deref())) {
                Ok(broker) => {
                    self.message = "Starting live preview…";
                    self.broker = Some(broker);
                }
                Err(_) => self.fail("Can’t open the bundled camera helper in local app data. You can still attach a JPG."),
            }
        }
        let broker = self.broker.as_mut()?;
        ctx.request_repaint_after(TICK);
        let (opened, new_frame, ended) = {
            let Ok(mut mailbox) = broker.mailbox.lock() else {
                return None;
            };
            (mailbox.opened.take(), mailbox.latest.take(), mailbox.ended)
        };
        let timed_out = self.latest.as_ref().map_or_else(
            || broker.started.elapsed() > START_LIMIT,
            |frame| frame.at.elapsed() > FRESH_LIMIT,
        );
        let remembered = opened.and_then(|reply| self.opened(reply));
        self.broker.as_ref()?;
        if ended || (new_frame.is_none() && timed_out) {
            self.fail("Video stopped or Windows refused access. Reconnect the camera and try again, check Settings, or attach a JPG.");
            return None;
        }
        if let Some(frame) = new_frame {
            self.message = "";
            if self.captured.is_none() {
                let image = frame.image();
                if let Some(texture) = &mut self.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture(
                        "Webcam preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.latest = Some(frame);
        }
        remembered
    }

    pub(super) fn show(&mut self, ui: &mut Ui) -> Action {
        let remembered = self.tick(ui.ctx());
        if let Some(after) = self.accept
            && self.latest.as_ref().is_some_and(|frame| frame.at > after)
            && let Some(captured) = &self.captured
        {
            return if let Ok(photo) = captured.photo() {
                Action::Use(photo)
            } else {
                self.reset_frame();
                self.message = "Could not prepare the captured JPG. Try again.";
                Action::Keep
            };
        }
        if !self.message.is_empty() {
            ui.label(self.message);
        }
        if let Some(camera) = &self.active {
            ui.weak(&camera.name);
        }
        if let Some(texture) = &self.texture {
            let mut size = texture.size_vec2();
            size *= (ui.available_width() / size.x).min(156.0 / size.y);
            ui.image((texture.id(), size));
        }
        match self.buttons(ui) {
            Action::Keep => remembered.map_or(Action::Keep, Action::Remember),
            action => action,
        }
    }

    fn buttons(&mut self, ui: &mut Ui) -> Action {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Cancel").clicked() {
                return Action::Cancel;
            }
            if ui.button("Browse JPG…").clicked() {
                return Action::Browse;
            }
            if self.broker.is_none() && !self.start_requested {
                if ui.button("Retry camera").clicked() {
                    self.start_requested = true;
                    ui.ctx().request_repaint();
                }
                if ui.button("Camera settings").clicked() {
                    return Action::Settings;
                }
                return Action::Keep;
            }
            let fresh = self
                .latest
                .as_ref()
                .is_some_and(|f| f.at.elapsed() <= FRESH_LIMIT);
            if self.captured.is_some() {
                if ui
                    .add_enabled(self.accept.is_none(), egui::Button::new("Retake"))
                    .clicked()
                {
                    self.captured = None;
                    self.texture = None;
                }
                if ui
                    .add_enabled(
                        fresh && self.accept.is_none(),
                        egui::Button::new("Use photo"),
                    )
                    .clicked()
                {
                    // Confirm a new frame from this still-live source after the click.
                    self.accept = Some(Instant::now());
                }
            } else if ui
                .add_enabled(fresh, egui::Button::new("Capture"))
                .clicked()
            {
                self.captured.clone_from(&self.latest);
            }
            Action::Keep
        })
        .inner
    }
}

#[cfg(test)]
mod tests {
    use super::{Camera, Preview, Unavailable, devices, frame, open_reply};
    use std::{path::PathBuf, time::Instant};
    use uuid::Uuid;

    #[test]
    fn source_loss_clears_snapshot_and_keeps_selected_identity() -> super::Result<()> {
        let sample = Uuid::from_u128(1);
        let camera = Camera {
            id: "built-in".into(),
            name: "Color camera".into(),
        };
        let mut preview = Preview::new(sample, None, PathBuf::new());
        assert!(preview.matches_target(Some(sample)));
        assert!(!preview.matches_target(Some(Uuid::from_u128(2))));
        assert!(!preview.matches_target(None));
        assert_eq!(preview.opened(Ok(camera.clone())), Some(camera.clone()));
        preview.latest = Some(frame(
            &mut [1, 0, 0, 0, 1, 0, 0, 0, 7, 8, 9, 255].as_slice(),
        )?);
        preview.captured.clone_from(&preview.latest);
        preview.accept = Some(Instant::now());
        preview.fail("device lost");
        assert!(preview.latest.is_none() && preview.captured.is_none() && preview.accept.is_none());
        assert!(!preview.start_requested);
        assert_eq!(preview.selected.as_deref(), Some(camera.id.as_str()));
        Ok(())
    }

    #[test]
    fn multiple_devices_missing_device_and_explicit_reconnect_never_switch() {
        let sample = Uuid::from_u128(1);
        let mut preview = Preview::new(sample, None, PathBuf::new());
        assert!(preview.opened(Err(Unavailable::ChooseCamera)).is_none());
        assert!(preview.selected.is_none() && !preview.start_requested);

        let builtin = Camera {
            id: "built-in".into(),
            name: "Color camera".into(),
        };
        let usb = Camera {
            id: "usb".into(),
            name: "Color camera".into(),
        };
        let mut usb_preview = Preview::new(sample, Some(usb.id.clone()), PathBuf::new());
        assert!(usb_preview.opened(Err(Unavailable::Disconnected)).is_none());
        assert!(usb_preview.opened(Ok(builtin)).is_none());
        assert_eq!(usb_preview.selected.as_deref(), Some(usb.id.as_str()));
        assert!(!usb_preview.start_requested && usb_preview.active.is_none());
        usb_preview.start_requested = true;
        assert_eq!(usb_preview.opened(Ok(usb.clone())), Some(usb));
    }

    #[test]
    fn packet_bounds_and_camera_identity() -> super::Result<()> {
        assert!(frame(&mut [0xff; 8].as_slice()).is_err());
        assert!(frame(&mut [1, 0, 0, 0, 1, 0, 0, 0, 7].as_slice()).is_err());
        let f = frame(&mut [1, 0, 0, 0, 1, 0, 0, 0, 7, 8, 9, 255].as_slice())?;
        assert!(f.photo().is_ok());
        let duplicate = [
            2, 0, 0, 0, 1, 0, 0, 0, b'a', 1, 0, 0, 0, b'b', 1, 0, 0, 0, b'a', 1, 0, 0, 0, b'c',
        ];
        assert!(devices(&mut duplicate.as_slice()).is_err());
        assert_eq!(
            open_reply(&mut [3, 0, 0, 0].as_slice())?,
            Err(Unavailable::Disconnected)
        );
        assert!(open_reply(&mut [0xff; 4].as_slice()).is_err());
        Ok(())
    }
}
