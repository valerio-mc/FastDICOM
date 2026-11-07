mod dicom;
mod glass;
mod gpu;
mod icons;
mod thumbnails;
mod ui;

use rune::{App, AppSpec, FileDialogOptions, Key, KeyEvent, Latest, Tx};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Window,
    Pan,
    Zoom,
}

struct Rendered {
    frame: Arc<dicom::Frame>,
    index: usize,
    default_window: bool,
    window_revision: u64,
}

struct AutoWindow {
    series_id: String,
    window_revision: u64,
    window: f32,
    level: f32,
}

#[derive(Clone, Copy)]
enum CtPreset {
    SoftTissue,
    Lung,
    Bone,
}

impl CtPreset {
    fn window(self) -> (f32, f32) {
        match self {
            Self::SoftTissue => (400.0, 40.0),
            Self::Lung => (1500.0, -600.0),
            Self::Bone => (2000.0, 400.0),
        }
    }
}

#[derive(Default)]
struct FrameCache {
    frames: HashMap<PathBuf, Arc<dicom::Frame>>,
    order: VecDeque<PathBuf>,
    bytes: usize,
}

impl FrameCache {
    fn get(&mut self, path: &PathBuf) -> Option<Arc<dicom::Frame>> {
        let result = self.frames.get(path)?.clone();
        self.order.retain(|p| p != path);
        self.order.push_back(path.clone());
        Some(result)
    }

    fn insert(&mut self, path: PathBuf, frame: Arc<dicom::Frame>) {
        const BUDGET: usize = 128 * 1024 * 1024;
        let bytes = (frame.values.len() + frame.contrast_samples.len()) * 4;
        if bytes > BUDGET || self.frames.contains_key(&path) {
            return;
        }
        while self.bytes + bytes > BUDGET {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(frame) = self.frames.remove(&old) {
                self.bytes -= (frame.values.len() + frame.contrast_samples.len()) * 4;
            }
        }
        self.bytes += bytes;
        self.order.push_back(path.clone());
        self.frames.insert(path, frame);
    }
}

struct Pane {
    selected: Option<usize>,
    index: usize,
    displayed_index: usize,
    frame: Option<Arc<dicom::Frame>>,
    window: f32,
    level: f32,
    window_revision: u64,
    initialize_window: bool,
    invert: bool,
    zoom: f32,
    pan: (f32, f32),
    rotation: f32,
    horizontal_flip: bool,
    tool: Tool,
    drag_origin: (f32, f32, f32, f32, f32),
    wheel_remainder: f32,
    // Test-only switch used by the compositor's independent glass fixture.
    #[cfg(test)]
    demo: bool,
    error: Option<String>,
    render: Latest<Result<Rendered, String>>,
    contrast: Latest<Result<AutoWindow, String>>,
    contrast_stop: Arc<AtomicBool>,
    contrast_error: Option<String>,
}

struct State {
    panes: [Pane; 2],
    active: usize,
    split: bool,
    split_ratio: f32,
    divider_origin: f32,
    viewport_width: f32,
    hovered_drop: Option<usize>,
    pane_rects: [[f32; 4]; 2],
    series_rects: HashMap<String, [f32; 4]>,
    series_drag: Option<SeriesDrag>,
    suppressed_series_press: Option<String>,
    series: Vec<dicom::Series>,
    hovered_series: Option<String>,
    hovered_patient_info: bool,
    privacy: bool,
    sidebar_visible: bool,
    logo: Option<rune::Image>,
    logo_ready: bool,
    assets: Option<rune::ImageCache>,
    thumbnails: Option<thumbnails::Thumbnails>,
    skipped_files: usize,
    scan: Latest<Result<dicom::ScanResult, String>>,
    cache: Arc<Mutex<FrameCache>>,
    series_windows: HashMap<String, (f32, f32)>,
}

struct SeriesDrag {
    id: String,
    start: (f32, f32),
    moved: bool,
}

fn contains(rect: [f32; 4], point: (f32, f32)) -> bool {
    rect[2] > 0.0
        && rect[3] > 0.0
        && point.0 >= rect[0]
        && point.1 >= rect[1]
        && point.0 < rect[0] + rect[2]
        && point.1 < rect[1] + rect[3]
}

impl Default for Pane {
    fn default() -> Self {
        Self {
            selected: None,
            index: 0,
            displayed_index: 0,
            frame: None,
            window: 400.0,
            level: 40.0,
            window_revision: 0,
            initialize_window: false,
            invert: false,
            zoom: 1.0,
            pan: (0.0, 0.0),
            rotation: 0.0,
            horizontal_flip: false,
            tool: Tool::Window,
            drag_origin: (400.0, 40.0, 1.0, 0.0, 0.0),
            wheel_remainder: 0.0,
            #[cfg(test)]
            demo: false,
            error: None,
            render: Latest::default(),
            contrast: Latest::default(),
            contrast_stop: Arc::new(AtomicBool::new(false)),
            contrast_error: None,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            panes: [Pane::default(), Pane::default()],
            active: 0,
            split: false,
            split_ratio: 0.5,
            divider_origin: 0.5,
            viewport_width: 1.0,
            hovered_drop: None,
            pane_rects: [[0.0; 4]; 2],
            series_rects: HashMap::new(),
            series_drag: None,
            suppressed_series_press: None,
            series: vec![],
            hovered_series: None,
            hovered_patient_info: false,
            privacy: false,
            sidebar_visible: true,
            logo: None,
            logo_ready: false,
            assets: None,
            thumbnails: None,
            skipped_files: 0,
            scan: Latest::default(),
            cache: Arc::new(Mutex::new(FrameCache::default())),
            series_windows: HashMap::new(),
        }
    }
}

// Shared controls deliberately address the active pane; pane-targeted input selects its owner
// before invoking these same controls. No study/assets/cache state is duplicated per pane.
impl std::ops::Deref for State {
    type Target = Pane;
    fn deref(&self) -> &Pane {
        &self.panes[self.active]
    }
}
impl std::ops::DerefMut for State {
    fn deref_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }
}

impl State {
    fn resize_split(&mut self, ratio: f32) {
        let minimum = (300.0 / (self.viewport_width - 6.0).max(1.0)).clamp(0.3, 0.5);
        self.split_ratio = ratio.clamp(minimum, 1.0 - minimum);
    }

    fn clear_pane(&mut self, index: usize) {
        self.panes[index].render.cancel();
        self.panes[index].cancel_contrast();
        let render = std::mem::take(&mut self.panes[index].render);
        let contrast = std::mem::take(&mut self.panes[index].contrast);
        self.panes[index] = Pane {
            render,
            contrast,
            ..Pane::default()
        };
    }

    fn toggle_split(&mut self) {
        if self.split {
            if self.active == 1 {
                self.panes.swap(0, 1);
            }
            self.active = 0;
            self.clear_pane(1);
        }
        self.split = !self.split;
        self.hovered_drop = None;
    }

    fn load_series(&mut self, pane: usize, index: usize) {
        let Some(series) = self.series.get(index) else {
            return;
        };
        let other = 1 - pane;
        // Unknown identities are safe only within the same study (encoded in series id).
        if self.panes[other]
            .selected
            .is_some_and(|i| !self.series[i].same_patient(series))
        {
            self.clear_pane(other);
        }
        self.active = pane;
        self.clear_pane(pane);
        self.selected = Some(index);
        self.request_image(true);
    }

    fn drag_series(&mut self, id: &str, drag: &rune::Drag) {
        if drag.phase == rune::Phase::Down {
            self.suppressed_series_press = None;
            self.series_drag = self.series_rects.get(id).map(|rect| SeriesDrag {
                id: id.to_owned(),
                start: (rect[0] + drag.start_x, rect[1] + drag.start_y),
                moved: false,
            });
            return;
        }
        let Some(gesture) = &mut self.series_drag else {
            return;
        };
        if gesture.id != id {
            return;
        }
        gesture.moved |= drag.dx.hypot(drag.dy) > 6.0;
        let point = (gesture.start.0 + drag.dx, gesture.start.1 + drag.dy);
        self.hovered_drop = if gesture.moved && !drag.cancelled() {
            self.pane_rects
                .iter()
                .take(if self.split { 2 } else { 1 })
                .position(|rect| contains(*rect, point))
        } else {
            None
        };
        if drag.phase == rune::Phase::Up {
            let gesture = self.series_drag.take().expect("gesture checked above");
            let target = self.hovered_drop.take();
            if gesture.moved && !drag.cancelled() {
                // Rune also sends on_press when a dragged pointer returns to its source.
                // Suppress only that queued release, preserving keyboard activation.
                if self
                    .series_rects
                    .get(id)
                    .is_some_and(|rect| contains(*rect, point))
                {
                    self.suppressed_series_press = Some(id.to_owned());
                }
                if let Some(pane) = target
                    && let Some(index) = self.series.iter().position(|s| s.id == id)
                {
                    self.load_series(pane, index);
                }
            }
        }
    }
    fn toggle_privacy(&mut self) {
        self.privacy = !self.privacy;
        self.hovered_patient_info = false;
    }

    fn count(&self) -> usize {
        self.selected.map_or(0, |i| self.series[i].files.len())
    }

    fn open(&mut self, path: PathBuf) {
        self.series_windows.clear();
        self.series_drag = None;
        self.series_rects.clear();
        self.hovered_drop = None;
        self.hovered_series = None;
        self.hovered_patient_info = false;
        self.clear_pane(0);
        self.clear_pane(1);
        self.active = 0;
        if let Some(thumbnails) = &mut self.thumbnails {
            thumbnails.clear();
        }
        self.series.clear();
        self.skipped_files = 0;
        self.cache = Arc::new(Mutex::new(FrameCache::default()));
        self.scan.request(path, |p| dicom::scan_folder(&p));
    }

    fn select(&mut self, i: usize) {
        self.load_series(self.active, i);
    }

    fn reset_geometry(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.rotation = 0.0;
        self.horizontal_flip = false;
    }

    fn step(&mut self, delta: i32) {
        let count = self.count();
        if count == 0 {
            return;
        }
        let next = (self.index as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        if next == self.index {
            return;
        }
        self.index = next;
        self.request_image(false);
    }

    fn request_image(&mut self, default_window: bool) {
        let Some(path) = self
            .selected
            .and_then(|i| self.series[i].files.get(self.index))
            .cloned()
        else {
            return;
        };
        self.error = None;
        self.initialize_window |= default_window;
        let default_window = self.initialize_window;
        let index = self.index;
        let window_revision = self.window_revision;
        let cache = self.cache.clone();
        self.render.request((), move |_| {
            let frame = cached_frame(path, &cache)?;
            Ok(Rendered {
                frame,
                index,
                default_window,
                window_revision,
            })
        });
    }

    fn is_ct(&self) -> bool {
        self.selected
            .and_then(|i| self.series.get(i))
            .is_some_and(|s| s.modality == "CT")
    }

    fn ct_preset(&mut self, preset: CtPreset) {
        if !self.is_ct() {
            return;
        }
        self.apply_window();
        (self.window, self.level) = preset.window();
    }

    fn request_auto(&mut self, pane: usize, explicit: bool) {
        let p = &mut self.panes[pane];
        let Some(frame) = p.frame.as_ref().cloned() else {
            return;
        };
        if explicit {
            p.apply_window();
            (p.window, p.level) = dicom::robust_window(&frame.contrast_samples);
        }
        let Some(series) = p.selected.and_then(|i| self.series.get(i)) else {
            return;
        };
        if let Some(window) = self.series_windows.get(&series.id) {
            (p.window, p.level) = *window;
            return;
        }
        p.cancel_contrast();
        p.contrast_stop = Arc::new(AtomicBool::new(false));
        p.contrast_error = None;
        let stop = p.contrast_stop.clone();
        let paths = dicom::representative_indices(series.files.len())
            .into_iter()
            .map(|i| series.files[i].clone())
            .collect::<Vec<_>>();
        let cache = self.cache.clone();
        let series_id = series.id.clone();
        let window_revision = p.window_revision;
        p.contrast.request((), move |_| {
            let mut samples = Vec::with_capacity(5 * 8192);
            for path in paths {
                if stop.load(Ordering::Relaxed) {
                    return Err("Auto contrast cancelled".into());
                }
                let frame = cached_frame(path, &cache)?;
                samples.extend_from_slice(&frame.contrast_samples);
            }
            let (window, level) = dicom::robust_window(&samples);
            Ok(AutoWindow {
                series_id,
                window_revision,
                window,
                level,
            })
        });
    }

    fn toggle_invert(&mut self) {
        self.invert = !self.invert;
    }

    fn demo(&mut self) {
        match std::env::current_exe()
            .and_then(|exe| std::env::current_dir().map(|cwd| demo_samples_path(&exe, &cwd)))
        {
            Ok(path) => self.open_demo_path(path),
            Err(error) => {
                self.error = Some(format!(
                    "Could not locate Demo samples: {error}. Use Open folder to choose a DICOM folder."
                ));
            }
        }
    }

    fn open_demo_path(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.error = Some(format!(
                "Demo samples are missing at {}. Rebuild the app bundle with scripts/bundle.sh, run from the source folder, or use Open folder to choose a DICOM folder.",
                path.display()
            ));
            return;
        }
        self.open(path);
    }
}

fn demo_samples_path(executable: &Path, working_directory: &Path) -> PathBuf {
    if let Some(macos) = executable.parent()
        && macos.file_name().is_some_and(|name| name == "MacOS")
        && let Some(contents) = macos.parent()
        && contents.file_name().is_some_and(|name| name == "Contents")
        && contents
            .parent()
            .is_some_and(|app| app.extension().is_some_and(|ext| ext == "app"))
    {
        // A broken bundle must report its missing resources, not borrow a source checkout.
        return contents.join("Resources/samples");
    }
    working_directory.join("samples")
}

impl Pane {
    fn apply_window(&mut self) {
        // Presentation edits touch only uniforms, and invalidate pending automatic adjustment.
        self.window_revision = self.window_revision.wrapping_add(1);
        self.initialize_window = false;
        self.cancel_contrast();
        self.contrast_error = None;
    }

    fn cancel_contrast(&mut self) {
        self.contrast_stop.store(true, Ordering::Relaxed);
        self.contrast.cancel();
    }

    fn accept_contrast(&mut self, result: &AutoWindow, series_id: Option<&str>) -> bool {
        if result.window_revision != self.window_revision
            || Some(result.series_id.as_str()) != series_id
        {
            return false;
        }
        (self.window, self.level) = (result.window, result.level);
        true
    }

    fn accept_image(&mut self, result: Rendered) -> bool {
        let initialize = self.initialize_window
            && result.default_window
            && result.window_revision == self.window_revision;
        if initialize {
            (self.window, self.level) = dicom::robust_window(&result.frame.contrast_samples);
            self.initialize_window = false;
        }
        self.frame = Some(result.frame);
        self.displayed_index = result.index;
        initialize
    }
}

fn cached_frame(
    path: PathBuf,
    cache: &Arc<Mutex<FrameCache>>,
) -> Result<Arc<dicom::Frame>, String> {
    if let Some(frame) = cache
        .lock()
        .map_err(|_| "Image cache unavailable")?
        .get(&path)
    {
        return Ok(frame);
    }
    let frame = Arc::new(dicom::decode(&path)?);
    cache
        .lock()
        .map_err(|_| "Image cache unavailable")?
        .insert(path, frame.clone());
    Ok(frame)
}

fn open_dialog(_: &mut State, tx: &mut Tx) {
    tx.open_folder(
        FileDialogOptions {
            title: "Open DICOM folder / DICOMDIR media".into(),
            ..Default::default()
        },
        |s: &mut State, result, _| match result {
            Ok(Some(path)) => s.open(path),
            Ok(None) => {}
            Err(error) => s.error = Some(format!("Could not open folder: {error}")),
        },
    );
}

fn setup(s: &mut State, wake: rune::Waker) {
    let assets = rune::ImageCache::new(wake.clone());
    let logo = assets.load(include_bytes!("../assets/titlebar-icon.png").as_slice());
    // Snapshot fixtures must include the static icon even when the virtual run is one frame.
    // Native windows load asynchronously and are woken by the cache.
    if rune::tooling::headless_run().is_some() {
        logo.wait().expect("embedded app logo decodes");
    }
    s.logo = Some(logo);
    s.assets = Some(assets);
    s.thumbnails = Some(thumbnails::Thumbnails::new(wake.clone()));
    s.scan.set_waker(wake.clone());
    for pane in &mut s.panes {
        pane.render.set_waker(wake.clone());
        pane.contrast.set_waker(wake.clone());
    }
    if let Some(path) = rune::extra_args().first() {
        s.open(PathBuf::from(path));
    }
}

fn poll(s: &mut State, _: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !s.logo_ready && s.logo.as_ref().is_some_and(|logo| logo.is_ready()) {
        s.logo_ready = true;
        changed = true;
    }
    if let Some(result) = s.scan.take() {
        changed = true;
        match result {
            Ok(result) => {
                s.skipped_files = result.skipped;
                s.series = result.series;
                if s.series.is_empty() {
                    s.error = Some("No supported image series found in this folder.".into());
                } else {
                    s.select(0);
                    if let Some(thumbnails) = &mut s.thumbnails {
                        thumbnails.start(&s.series);
                    }
                }
            }
            Err(error) => s.error = Some(error),
        }
    }
    let mut auto = Vec::with_capacity(2);
    for (index, pane) in s.panes.iter_mut().enumerate() {
        if let Some(result) = pane.render.take() {
            changed = true;
            match result {
                Ok(result) => {
                    if pane.accept_image(result) {
                        auto.push(index);
                    }
                }
                Err(error) => {
                    pane.frame = None;
                    pane.error = Some(error);
                }
            }
        }
        if let Some(result) = pane.contrast.take() {
            changed = true;
            match result {
                Ok(result) => {
                    let id = pane
                        .selected
                        .and_then(|i| s.series.get(i))
                        .map(|s| s.id.as_str());
                    if pane.accept_contrast(&result, id) {
                        s.series_windows
                            .insert(result.series_id, (result.window, result.level));
                    }
                }
                Err(error) => pane.contrast_error = Some(error),
            }
        }
    }
    for pane in auto {
        s.request_auto(pane, false);
    }
    if let Some(thumbnails) = &mut s.thumbnails {
        changed |= thumbnails.poll();
    }
    if changed {
        tx.instant();
    }
    changed
}

fn keys(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    match &event.key {
        Key::Command('b') => {
            if !tx.key_repeat() {
                s.sidebar_visible = !s.sidebar_visible;
                tx.instant();
            }
            true
        }
        Key::Command('p') if event.mods.shift => {
            if !tx.key_repeat() {
                s.toggle_privacy();
                tx.instant();
            }
            true
        }
        Key::Command('o') => {
            open_dialog(s, tx);
            true
        }
        _ => false,
    }
}

fn main() {
    rune::set_app_id("app.fastdicom.viewer");
    App::new(AppSpec {
        min_size: Some((1000.0, 680.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(keys),
        ..AppSpec::new("FastDICOM", (1280.0, 840.0), State::default, ui::view)
    })
    .compositor(gpu::ImagingCompositor::default())
    .window_options(rune::WindowOptions {
        transparent: false,
        ..Default::default()
    })
    .chrome(rune::Chrome {
        full_size_content: true,
        traffic_lights: Some((16.0, 13.0)),
        drag_regions: true,
    })
    .run();
}

#[cfg(test)]
#[path = "../tests/unit/fixtures.rs"]
mod fixtures;
#[cfg(test)]
use fixtures::synthetic_frame;

#[cfg(test)]
#[path = "../tests/unit/app.rs"]
mod tests;
