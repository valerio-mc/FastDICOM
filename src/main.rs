mod dicom;
mod glass;
mod gpu;
mod icons;
mod thumbnails;
mod ui;

use rune::{App, AppSpec, FileDialogOptions, Key, KeyEvent, Latest, Tx};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
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
    tool: Tool,
    drag_origin: (f32, f32, f32, f32, f32),
    wheel_remainder: f32,
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
            tool: Tool::Window,
            drag_origin: (400.0, 40.0, 1.0, 0.0, 0.0),
            wheel_remainder: 0.0,
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
            || self.panes[other].demo
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
        if self.demo {
            160
        } else {
            self.selected.map_or(0, |i| self.series[i].files.len())
        }
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
        self.frame = None;
        self.series.clear();
        self.selected = None;
        self.demo = false;
        self.error = None;
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
        let path = self
            .selected
            .and_then(|i| self.series[i].files.get(self.index))
            .cloned();
        if !self.demo && path.is_none() {
            return;
        }
        self.error = None;
        self.initialize_window |= default_window;
        let default_window = self.initialize_window;
        let demo = self.demo;
        let index = self.index;
        let window_revision = self.window_revision;
        let cache = self.cache.clone();
        self.render.request((), move |_| {
            let frame = if demo {
                Arc::new(synthetic_frame(index))
            } else {
                let path = path.expect("real requests carry a file path");
                cached_frame(path, &cache)?
            };
            Ok(Rendered {
                frame,
                index,
                default_window,
                window_revision,
            })
        });
    }

    fn is_ct(&self) -> bool {
        !self.demo
            && self
                .selected
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
        self.series_windows.clear();
        self.series_drag = None;
        self.series_rects.clear();
        self.hovered_drop = None;
        self.hovered_series = None;
        self.hovered_patient_info = false;
        self.scan.cancel();
        self.clear_pane(0);
        self.clear_pane(1);
        self.active = 0;
        if let Some(thumbnails) = &mut self.thumbnails {
            thumbnails.clear();
        }
        self.series.clear();
        self.load_demo();
    }

    fn load_demo(&mut self) {
        self.clear_pane(self.active);
        self.demo = true;
        self.index = 80;
        self.reset_geometry();
        self.invert = false;
        self.request_image(true);
    }
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
        let auto = initialize && !self.demo;
        if initialize {
            (self.window, self.level) = if auto {
                dicom::robust_window(&result.frame.contrast_samples)
            } else {
                (result.frame.window, result.frame.level)
            };
            self.initialize_window = false;
        }
        self.frame = Some(result.frame);
        self.displayed_index = result.index;
        auto
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

fn synthetic_frame(index: usize) -> dicom::Frame {
    let size = 512;
    let z = (index as f32 - 80.0) / 100.0;
    let mut values = vec![-1000.0; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 - 256.0) / 200.0;
            let v = (y as f32 - 256.0) / 200.0;
            let r = u * u / 0.85 + v * v;
            let noise = ((x * 17 + y * 31 + index * 7) % 23) as f32 - 11.0;
            let mut value = -1000.0;
            if r < 0.91 - z * z * 0.15 {
                value = 25.0 + noise;
            }
            if r > 0.82 - z * z * 0.15 && r < 0.91 - z * z * 0.15 {
                value = -90.0 + noise;
            }
            for side in [-1.0, 1.0] {
                let lung = (u - side * 0.37).powi(2) / 0.08 + (v + 0.12).powi(2) / 0.27;
                if lung < 1.0 && r < 0.8 {
                    value = -780.0 + noise * 4.0;
                }
                let bone = (u - side * 0.68).powi(2) + (v - 0.08).powi(2);
                if bone < 0.006 {
                    value = 850.0 + noise;
                }
            }
            if u * u / 0.035 + (v - 0.51).powi(2) / 0.025 < 1.0 {
                value = 650.0 + noise;
            }
            if (u - 0.08).powi(2) / 0.09 + (v - 0.10).powi(2) / 0.12 < 1.0 {
                value = 70.0 + noise;
            }
            values[y * size + x] = value;
        }
    }
    dicom::Frame {
        width: size as u32,
        height: size as u32,
        contrast_samples: values.iter().step_by(32).copied().collect(),
        values,
        window: 1500.0,
        level: -500.0,
        supplied_window: true,
        inverted: false,
        pixel_aspect: 1.0,
    }
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
        scenes: &[
            ("demo", |s| {
                s.demo = true;
                s.index = 80;
                s.displayed_index = 80;
                let frame = synthetic_frame(80);
                s.window = frame.window;
                s.level = frame.level;
                s.frame = Some(Arc::new(frame));
            }),
            ("split-demo", |s| {
                s.split = true;
                for (i, pane) in s.panes.iter_mut().enumerate() {
                    let frame = synthetic_frame(50 + i * 50);
                    pane.demo = true;
                    pane.index = 50 + i * 50;
                    pane.displayed_index = pane.index;
                    pane.window = frame.window;
                    pane.level = frame.level;
                    pane.frame = Some(Arc::new(frame));
                }
            }),
        ],
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
mod tests {
    use super::*;

    fn contrast_study(supplied: bool) -> State {
        let mut s = State::default();
        s.series.push(dicom::Series {
            id: "study/series".into(),
            description: "".into(),
            modality: "MR".into(),
            study_description: "".into(),
            patient_name: "".into(),
            patient_id: "".into(),
            patient_issuer: "".into(),
            files: (0..10)
                .map(|i| PathBuf::from(format!("slice-{i}")))
                .collect(),
        });
        for i in 0..10 {
            let values = vec![i as f32 * 100.0; 100];
            s.cache.lock().unwrap().insert(
                s.series[0].files[i].clone(),
                Arc::new(dicom::Frame {
                    width: 10,
                    height: 10,
                    pixel_aspect: 1.0,
                    contrast_samples: values.clone(),
                    values,
                    window: if supplied { 777.0 } else { 1.0 },
                    level: if supplied { 222.0 } else { 0.5 },
                    supplied_window: supplied,
                    inverted: false,
                }),
            );
        }
        s.selected = Some(0);
        s.request_image(true);
        s
    }

    fn contrast_runtime(s: State) -> rune::Runtime<State> {
        let mut rt = rune::Runtime::new(s, ui::view, (1000.0, 680.0), rune::Env::new(true, 1.0));
        rt.poll = Some(poll);
        rt
    }

    fn settle(rt: &mut rune::Runtime<State>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            rt.frame(0.0);
            if rt
                .state
                .panes
                .iter()
                .all(|p| !p.render.is_pending() && !p.contrast.is_pending())
            {
                rt.frame(0.0);
                if rt
                    .state
                    .panes
                    .iter()
                    .all(|p| !p.render.is_pending() && !p.contrast.is_pending())
                {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background contrast did not settle"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn auto_is_default_despite_supplied_window_and_series_contrast_is_cached_across_navigation() {
        let mut rt = contrast_runtime(contrast_study(true));
        settle(&mut rt);
        let window = (rt.state.window, rt.state.level);
        assert_eq!(window, (901.0, 450.5));
        assert_eq!(rt.state.series_windows.len(), 1);
        let revision = rt.state.window_revision;
        rt.update(|s, _| s.step(7));
        settle(&mut rt);
        assert_eq!((rt.state.window, rt.state.level), window);
        assert_eq!(rt.state.window_revision, revision);
        // A completed series result needs no frame decode on repeated Auto.
        *rt.state.cache.lock().unwrap() = FrameCache::default();
        rt.update(|s, _| s.request_auto(s.active, true));
        assert!(!rt.state.contrast.is_pending());
        assert_eq!((rt.state.window, rt.state.level), window);
    }

    #[test]
    fn absent_window_initializes_once_then_late_initial_slice_cannot_overwrite_series_or_manual_window()
     {
        let mut rt = contrast_runtime(contrast_study(false));
        settle(&mut rt);
        assert_eq!((rt.state.window, rt.state.level), (901.0, 450.5));
        let frame = rt.state.frame.clone().unwrap();
        // A request made while the first image was pending may still carry default_window=true.
        rt.update(|s, _| {
            s.accept_image(Rendered {
                frame: frame.clone(),
                index: 1,
                default_window: true,
                window_revision: 0,
            });
        });
        assert_eq!((rt.state.window, rt.state.level), (901.0, 450.5));
        rt.update(|s, _| {
            s.window = 321.0;
            s.level = 123.0;
            s.apply_window();
            s.step(1);
        });
        settle(&mut rt);
        assert_eq!((rt.state.window, rt.state.level), (321.0, 123.0));
    }

    #[test]
    fn pending_contrast_cannot_override_manual_window_or_ct_preset_and_cancels_on_replacement() {
        let mut rt = contrast_runtime(contrast_study(true));
        settle(&mut rt);
        let (send, receive) = std::sync::mpsc::channel();
        let (started, entered) = std::sync::mpsc::channel();
        rt.update(|s, _| {
            s.contrast.request((), move |_| {
                started.send(()).unwrap();
                receive.recv().unwrap();
                Ok(AutoWindow {
                    series_id: "study/series".into(),
                    window_revision: 0,
                    window: 9999.0,
                    level: 999.0,
                })
            });
        });
        entered
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        rt.update(|s, _| {
            s.window = 250.0;
            s.apply_window();
        });
        send.send(()).unwrap();
        settle(&mut rt);
        assert_eq!(rt.state.window, 250.0);
        let stale = AutoWindow {
            series_id: "study/series".into(),
            window_revision: 0,
            window: 9999.0,
            level: 999.0,
        };
        assert!(!rt.state.panes[0].accept_contrast(&stale, Some("study/series")));
        rt.update(|s, _| {
            s.series[0].modality = "CT".into();
            s.request_auto(0, true);
            s.ct_preset(CtPreset::Lung);
        });
        settle(&mut rt);
        assert_eq!((rt.state.window, rt.state.level), (1500.0, -600.0));
        assert!(!rt.state.contrast.is_pending());
        rt.update(|s, _| {
            s.request_auto(0, true);
            s.select(0);
        });
        settle(&mut rt);
        assert_eq!((rt.state.window, rt.state.level), (901.0, 450.5));
        assert!(!rt.state.contrast.is_pending());
    }

    #[test]
    fn automatic_requests_are_pane_local_and_move_with_retained_active_pane() {
        let mut rt = contrast_runtime(contrast_study(true));
        settle(&mut rt);
        rt.update(|s, _| {
            s.split = true;
            s.load_series(1, 0);
        });
        settle(&mut rt);
        rt.update(|s, _| {
            s.active = 0;
            s.window = 333.0;
            s.apply_window();
            s.request_auto(1, true);
            s.active = 1;
            s.toggle_split();
        });
        settle(&mut rt);
        assert!(!rt.state.split);
        assert_eq!((rt.state.window, rt.state.level), (901.0, 450.5));
        assert!(rt.state.panes[1].frame.is_none());
        assert!(!rt.state.panes[1].contrast.is_pending());
        rt.update(|s, _| {
            s.split = true;
            s.load_series(1, 0);
        });
        settle(&mut rt);
        assert_eq!(
            (rt.state.panes[0].window, rt.state.panes[0].level),
            (901.0, 450.5)
        );
        assert_eq!(
            (rt.state.panes[1].window, rt.state.panes[1].level),
            (901.0, 450.5)
        );
        rt.update(|s, _| {
            s.request_auto(1, true);
        });
        assert_eq!(
            (rt.state.panes[1].window, rt.state.panes[1].level),
            (901.0, 450.5)
        );
    }

    #[test]
    fn independent_window_revisions_and_latest_requests_survive_active_retention() {
        let mut s = State {
            split: true,
            ..State::default()
        };
        for p in &mut s.panes {
            p.demo = true;
            p.initialize_window = true;
        }
        s.window = 222.0;
        s.apply_window();
        s.accept_image(Rendered {
            frame: Arc::new(synthetic_frame(1)),
            index: 1,
            default_window: true,
            window_revision: 0,
        });
        assert_eq!(s.window, 222.0);
        s.active = 1;
        s.request_image(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while s.render.is_pending() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        s.toggle_split(); // Move the active Latest with its pane, not its result into the discarded pane.
        let result = s
            .render
            .take()
            .expect("retained request completed")
            .unwrap();
        s.accept_image(result);
        assert_eq!(s.window, 1500.0);
        assert_eq!(s.window_revision, 0);
        assert!(!s.initialize_window);
        assert!(s.panes[1].frame.is_none());
        assert!(!s.panes[1].render.is_pending());
    }

    #[test]
    fn clearing_one_latest_cannot_cancel_or_deliver_into_the_other_pane() {
        let mut s = State {
            split: true,
            ..State::default()
        };
        for pane in 0..2 {
            s.active = pane;
            s.demo = true;
            s.index = pane + 5;
            s.request_image(true);
        }
        s.clear_pane(0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while s.panes[1].render.is_pending() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let result = s.panes[1]
            .render
            .take()
            .expect("second request completed")
            .unwrap();
        assert_eq!(result.index, 6);
        assert!(s.panes[0].render.take().is_none());
        assert!(s.panes[0].frame.is_none());
        s.active = 1;
        s.accept_image(result);
        assert_eq!(s.displayed_index, 6);
        assert_eq!(s.window, 1500.0);
    }

    #[test]
    fn slice_navigation_clamps_and_preserves_requested_index() {
        let mut s = State {
            panes: [
                Pane {
                    demo: true,
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        s.step(-100);
        assert_eq!(s.index, 0);
        s.step(1000);
        assert_eq!(s.index, 159);
    }

    #[test]
    fn cache_evicts_oldest_frames_to_budget() {
        let mut cache = FrameCache::default();
        let frame = Arc::new(dicom::Frame {
            width: 4096,
            height: 4096,
            values: vec![0.0; 4096 * 4096],
            window: 1.0,
            level: 0.0,
            supplied_window: true,
            contrast_samples: vec![],
            inverted: false,
            pixel_aspect: 1.0,
        });
        for i in 0..3 {
            cache.insert(PathBuf::from(i.to_string()), frame.clone());
        }
        assert_eq!(cache.frames.len(), 2);
        assert!(!cache.frames.contains_key(&PathBuf::from("0")));
        assert_eq!(cache.bytes, 128 * 1024 * 1024);
    }

    #[test]
    fn window_controls_do_not_schedule_decode_work() {
        let mut s = State {
            panes: [
                Pane {
                    window: 700.0,
                    level: 120.0,
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        s.apply_window();
        assert!(!s.render.is_pending());
        assert_eq!(s.window_revision, 1);
    }

    #[test]
    fn first_window_initialization_survives_navigation_but_not_a_user_edit() {
        let mut s = State {
            panes: [
                Pane {
                    demo: true,
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        s.request_image(true);
        s.step(1);
        assert!(s.initialize_window);
        s.render.cancel();
        let default_window = s.initialize_window;
        let window_revision = s.window_revision;
        s.accept_image(Rendered {
            frame: Arc::new(synthetic_frame(1)),
            index: 1,
            default_window,
            window_revision,
        });
        assert_eq!(s.window, 1500.0);
        assert_eq!(s.level, -500.0);
        assert!(!s.initialize_window);

        s.initialize_window = true;
        let old_revision = s.window_revision;
        s.window = 250.0;
        s.apply_window();
        s.accept_image(Rendered {
            frame: Arc::new(synthetic_frame(2)),
            index: 2,
            default_window: true,
            window_revision: old_revision,
        });
        assert_eq!(s.window, 250.0);
    }

    #[test]
    fn toolbar_invert_and_fit_change_presentation() {
        let state = State {
            panes: [
                Pane {
                    demo: true,
                    frame: Some(Arc::new(synthetic_frame(80))),
                    zoom: 2.0,
                    pan: (20.0, 30.0),
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        let mut runtime = rune::Runtime::new(
            state,
            ui::view,
            (1440.0, 940.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        let (x, y) = runtime.find("invert").expect("invert button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.invert);
        assert!(!runtime.state.render.is_pending());
        let (x, y) = runtime.find("fit").expect("fit button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert_eq!(runtime.state.zoom, 1.0);
        assert_eq!(runtime.state.pan, (0.0, 0.0));
    }

    #[test]
    fn inversion_during_first_load_preserves_window_initialization() {
        let mut state = State {
            panes: [
                Pane {
                    demo: true,
                    initialize_window: true,
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        state.toggle_invert();
        assert!(state.invert);
        assert!(state.initialize_window);
        let revision = state.window_revision;
        state.accept_image(Rendered {
            frame: Arc::new(synthetic_frame(0)),
            index: 0,
            default_window: true,
            window_revision: revision,
        });
        assert_eq!(state.window, 1500.0);
        assert!(state.invert);
    }

    #[test]
    fn pending_slice_keeps_the_displayed_image_visible() {
        let state = State {
            panes: [
                Pane {
                    demo: true,
                    index: 1,
                    displayed_index: 0,
                    frame: Some(Arc::new(synthetic_frame(0))),
                    ..Pane::default()
                },
                Pane::default(),
            ],
            ..State::default()
        };
        let mut runtime = rune::Runtime::new(
            state,
            ui::view,
            (1000.0, 680.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("image").is_some());
        assert!(runtime.find("slice-pending").is_some());
        assert!(runtime.find("loading-progress").is_some());
        assert!(runtime.find("image-loading").is_none());
        assert_eq!(runtime.state.displayed_index, 0);
    }
}
