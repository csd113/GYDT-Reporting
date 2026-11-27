use std::{
    io::{self, Read, Write},
    os::windows::process::CommandExt as _,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, ColorImage, Context, TextureHandle, Ui};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use uuid::Uuid;

use super::model::{PhotoCopy, Result};

const HELPER: &str = "GYDT-camera.exe";
const TICK: Duration = Duration::from_millis(80);
const START_LIMIT: Duration = Duration::from_secs(6);
const FRESH_LIMIT: Duration = Duration::from_millis(850);

#[derive(Clone)]
struct Camera {
    id: String,
    name: String,
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

fn helper() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.with_file_name(HELPER))
}

impl Broker {
    fn start(id: Option<&str>) -> io::Result<Self> {
        let mut command = Command::new(helper()?);
        command
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        if let Some(id) = id {
            command.arg("preview").arg(id);
        } else {
            command.arg("list");
        }
        let mut child = command.spawn()?;
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let mut output = child.stdout.take().ok_or_else(invalid)?;
        let mut input = child.stdin.take().ok_or_else(invalid)?;
        let stream = id.is_some();
        let copy = Arc::clone(&mailbox);
        let now = Instant::now();
        if let Err(err) = thread::Builder::new()
            .name("webcam-pipe".into())
            .spawn(move || {
                if stream {
                    while input.write_all(b"f").is_ok() {
                        let Ok(current) = frame(&mut output) else {
                            break;
                        };
                        if let Ok(mut state) = copy.lock() {
                            state.latest = Some(current);
                        }
                        thread::sleep(TICK);
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

/// One modal, one sample ID and one broker generation. A different camera can
/// be opened only after the old process has exited. Each process has its own
/// mailbox, so an old thread cannot publish into a new session.
enum Request {
    List,
    Stream(String),
}

pub(super) struct Preview {
    pub(super) sample: Uuid,
    devices: Vec<Camera>,
    selected: Option<String>,
    broker: Option<Broker>,
    next: Option<Request>,
    stopping: bool,
    latest: Option<Frame>,
    captured: Option<Frame>,
    accept: Option<Instant>,
    texture: Option<TextureHandle>,
    message: &'static str,
}

pub(super) enum Action {
    Keep,
    Cancel,
    Remember(String),
    Use(PhotoCopy, String),
}

impl Preview {
    pub(super) fn new(sample: Uuid, selected: Option<String>) -> Self {
        Self {
            sample,
            selected,
            devices: Vec::new(),
            broker: None,
            next: Some(Request::List),
            stopping: false,
            latest: None,
            captured: None,
            accept: None,
            texture: None,
            message: "Looking for color cameras…",
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

    fn queue(&mut self, next: Option<Request>) {
        self.reset_frame();
        if let Some(broker) = &mut self.broker {
            broker.stop();
            self.stopping = true;
        }
        self.next = next;
    }

    fn fail(&mut self, message: &'static str) {
        self.queue(None);
        self.devices.clear();
        self.next = Some(Request::List);
        self.message = message;
    }

    fn choose(&mut self, choice: Option<String>) {
        if choice != self.selected {
            self.selected = choice;
            self.queue(None);
            self.message = "Choose Start preview to open this camera.";
        }
    }

    fn selected_device(&self) -> Option<&Camera> {
        self.devices
            .iter()
            .find(|device| Some(&device.id) == self.selected.as_ref())
    }

    fn can_start(&self) -> bool {
        self.broker.is_none() && !self.stopping && self.selected_device().is_some()
    }

    fn tick(&mut self, ctx: &Context) {
        ctx.request_repaint_after(TICK);
        if self.stopping {
            if !self.broker.as_mut().is_some_and(Broker::exited) {
                return;
            }
            self.broker = None;
            self.stopping = false;
        }
        if let Some(request) = self.next.take() {
            let id = match &request {
                Request::List => None,
                Request::Stream(id) => Some(id.as_str()),
            };
            match Broker::start(id) {
                Ok(broker) => {
                    if id.is_some() {
                        self.message = "Starting video preview…";
                    }
                    self.broker = Some(broker);
                }
                Err(_) => {
                    self.message = "Camera support is unavailable. You can still attach a JPG.";
                }
            }
        }
        let Some(broker) = &mut self.broker else {
            return;
        };
        let Ok(mut mailbox) = broker.mailbox.lock() else {
            return;
        };
        let list = mailbox.devices.take();
        let new_frame = mailbox.latest.take();
        let ended = mailbox.ended;
        drop(mailbox);
        if let Some(list) = list {
            self.devices = list;
            if self.message == "Looking for color cameras…" {
                self.message = if self.devices.is_empty() {
                    "No color camera found."
                } else {
                    "Choose a camera, then start preview."
                };
            }
            self.broker = None;
            return;
        }
        if ended || broker.exited() {
            if self.message == "Looking for color cameras…" || self.devices.is_empty() {
                self.message = "Could not find cameras. Refresh or attach a JPG.";
                self.broker = None;
            } else {
                self.fail("Video stopped or Windows refused access. Reconnect explicitly or attach a JPG.");
            }
            return;
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
        if self.latest.as_ref().map_or_else(
            || broker.started.elapsed() > START_LIMIT,
            |f| f.at.elapsed() > FRESH_LIMIT,
        ) {
            if self.devices.is_empty() {
                self.queue(None);
                self.message = "Camera discovery timed out. Refresh or attach a JPG.";
            } else {
                self.fail("No current video frame. Reconnect explicitly or attach a JPG.");
            }
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui) -> Action {
        self.tick(ui.ctx());
        if let Some(after) = self.accept
            && self.latest.as_ref().is_some_and(|f| f.at > after)
            && let (Some(captured), Some(selected)) = (&self.captured, &self.selected)
        {
            return if let Ok(photo) = captured.photo() {
                Action::Use(photo, selected.clone())
            } else {
                self.reset_frame();
                self.message = "Could not prepare the captured JPG. Try again.";
                Action::Keep
            };
        }
        self.camera_selector(ui);
        self.preview_image(ui);
        self.buttons(ui)
    }

    fn camera_selector(&mut self, ui: &mut Ui) {
        let mut choice = self.selected.clone();
        ui.horizontal(|ui| {
            ui.label("Camera");
            let label = self
                .selected_device()
                .map_or("Choose camera", |device| device.name.as_str());
            egui::ComboBox::from_id_salt("webcam camera")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (i, device) in self.devices.iter().enumerate() {
                        let duplicate = self
                            .devices
                            .iter()
                            .filter(|item| item.name == device.name)
                            .count()
                            > 1;
                        let label = if duplicate {
                            format!("{} ({})", device.name, i + 1)
                        } else {
                            device.name.clone()
                        };
                        ui.selectable_value(&mut choice, Some(device.id.clone()), label);
                    }
                });
            if ui.button("Refresh").clicked() {
                self.devices.clear();
                self.queue(Some(Request::List));
                self.message = "Looking for color cameras…";
            }
        });
        self.choose(choice);
    }

    fn preview_image(&self, ui: &mut Ui) {
        if !self.message.is_empty() {
            ui.label(self.message);
        }
        if let Some(texture) = &self.texture {
            let mut size = texture.size_vec2();
            size *= (ui.available_width() / size.x).min(156.0 / size.y);
            ui.image((texture.id(), size));
        }
    }

    fn buttons(&mut self, ui: &mut Ui) -> Action {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Cancel").clicked() {
                return Action::Cancel;
            }
            if ui
                .add_enabled(self.can_start(), egui::Button::new("Start preview"))
                .clicked()
                && let Some(id) = self.selected.clone()
            {
                self.queue(Some(Request::Stream(id.clone())));
                return Action::Remember(id);
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
    use super::{devices, frame};

    #[test]
    fn source_loss_clears_snapshot_and_requires_explicit_reconnect() -> super::Result<()> {
        use super::{Preview, Request};
        use std::time::Instant;
        use uuid::Uuid;

        let sample = Uuid::from_u128(1);
        let mut state = Preview::new(sample, Some("old-id".into()));
        assert!(state.matches_target(Some(sample)));
        assert!(!state.matches_target(Some(Uuid::from_u128(2))));
        assert!(!state.matches_target(None));
        state.latest = Some(frame(
            &mut [1, 0, 0, 0, 1, 0, 0, 0, 7, 8, 9, 255].as_slice(),
        )?);
        state.captured.clone_from(&state.latest);
        state.accept = Some(Instant::now());
        state.fail("device lost");
        assert!(state.latest.is_none() && state.captured.is_none() && state.accept.is_none());
        assert!(matches!(state.next, Some(Request::List)));
        assert_eq!(state.selected.as_deref(), Some("old-id"));
        Ok(())
    }

    #[test]
    fn unplug_reorder_and_restore_never_select_or_start_another_camera() -> super::Result<()> {
        use super::{Camera, Preview, Request};
        use uuid::Uuid;
        let builtin = Camera {
            id: "builtin-id".into(),
            name: "Camera".into(),
        };
        let usb = Camera {
            id: "usb-id".into(),
            name: "Camera".into(),
        };
        let mut state = Preview::new(Uuid::from_u128(1), Some(usb.id.clone()));
        state.devices = vec![builtin.clone(), usb.clone()];
        assert_eq!(state.selected_device().ok_or("USB unavailable")?.id, usb.id);
        assert!(state.can_start());

        state.fail("USB disconnected");
        assert!(matches!(state.next.take(), Some(Request::List)));
        state.devices = vec![builtin.clone()];
        assert!(!state.can_start());
        assert_eq!(state.selected.as_deref(), Some(usb.id.as_str()));

        state.devices = vec![usb.clone(), builtin.clone()];
        assert!(state.can_start() && state.next.is_none());
        assert_eq!(
            state.selected_device().ok_or("USB not restored")?.id,
            usb.id
        );

        state.queue(Some(Request::Stream(usb.id.clone())));
        state.choose(Some(builtin.id.clone()));
        assert!(state.next.is_none());
        assert_eq!(
            state.selected_device().ok_or("built-in unavailable")?.id,
            builtin.id
        );
        assert!(state.can_start());
        state.choose(Some(usb.id.clone()));
        assert_eq!(state.selected_device().ok_or("USB unavailable")?.id, usb.id);
        assert!(state.can_start() && state.next.is_none());
        Ok(())
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
        Ok(())
    }
}
