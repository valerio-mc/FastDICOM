//! Progressive, local series previews. One shared-pool job decodes the batch;
//! only small RGBA buffers cross the bounded inbox to the UI-owned cache.

use crate::dicom;
use rune::{Latest, Pixels, Waker};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

const RGBA_BUDGET: usize = 16 * 1024 * 1024;
const MAX_EDGE: u32 = 96;
const INBOX_CAPACITY: usize = 2;

struct Preview {
    id: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

pub struct Thumbnails {
    batch: Latest<()>,
    wake: Waker,
    cancelled: Arc<AtomicBool>,
    inbox: Option<mpsc::Receiver<Preview>>,
    images: HashMap<String, Pixels>,
}

impl Thumbnails {
    pub fn new(waker: Waker) -> Self {
        Self {
            batch: Latest::new(waker.clone()),
            wake: waker,
            cancelled: Arc::new(AtomicBool::new(false)),
            inbox: None,
            images: HashMap::new(),
        }
    }

    pub fn start(&mut self, series: &[dicom::Series]) {
        self.clear();
        let jobs: Vec<_> = series
            .iter()
            .filter_map(|s| middle_file(&s.files).map(|path| (s.id.clone(), path.to_owned())))
            .collect();
        if jobs.is_empty() {
            return;
        }
        let edge = max_edge(jobs.len());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancelled = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(INBOX_CAPACITY);
        self.inbox = Some(receiver);
        let wake = self.wake.clone();
        self.batch.request(jobs, move |jobs| {
            for (id, path) in jobs {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                // A failed preview is absent, not an error overlay or a reason to
                // stop decoding the other series. Never publish path/PHI errors.
                let Ok(frame) = dicom::decode(&path) else {
                    continue;
                };
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let Some((width, height, rgba)) = downsample(&frame, edge) else {
                    continue;
                };
                drop(frame);
                if cancelled.load(Ordering::Relaxed)
                    || sender
                        .send(Preview {
                            id,
                            width,
                            height,
                            rgba,
                        })
                        .is_err()
                {
                    break;
                }
                wake();
            }
        });
    }

    pub fn clear(&mut self) {
        // Disconnect first: a worker blocked on the bounded send must be released.
        // Each generation owns a distinct inbox, so stale sends cannot be accepted.
        self.inbox = None;
        self.cancelled.store(true, Ordering::Relaxed);
        self.batch.cancel();
        self.images.clear();
    }

    pub fn poll(&mut self) -> bool {
        self.batch.take();
        let Some(inbox) = &self.inbox else {
            return false;
        };
        let mut changed = false;
        // Bound UI work even if a large batch is producing continuously.
        for _ in 0..8 {
            let Ok(preview) = inbox.try_recv() else {
                return changed;
            };
            let pixels = Pixels::new();
            pixels.set(preview.width, preview.height, preview.rgba);
            self.images.insert(preview.id, pixels);
            changed = true;
        }
        (self.wake)();
        changed
    }

    pub fn get(&self, series_id: &str) -> Option<&Pixels> {
        self.images.get(series_id)
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.clear();
    }
}

fn middle_file(files: &[PathBuf]) -> Option<&Path> {
    files.get(files.len() / 2).map(PathBuf::as_path)
}

fn max_edge(count: usize) -> u32 {
    // Square thumbnails are the worst case. At the scanner's 100k-file ceiling
    // even the minimum 1px thumbnail remains well within this RGBA budget.
    ((RGBA_BUDGET / count.max(1) / 4) as f64)
        .sqrt()
        .floor()
        .clamp(1.0, f64::from(MAX_EDGE)) as u32
}

fn downsample(frame: &dicom::Frame, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
    let count = (frame.width as usize).checked_mul(frame.height as usize)?;
    if count == 0
        || count != frame.values.len()
        || !frame.window.is_finite()
        || frame.window < 1.0
        || !frame.level.is_finite()
        || !frame.pixel_aspect.is_finite()
        || frame.pixel_aspect <= 0.0
    {
        return None;
    }
    let physical_width = f64::from(frame.width);
    let physical_height = f64::from(frame.height) * f64::from(frame.pixel_aspect);
    let edge = f64::from(edge.clamp(1, MAX_EDGE));
    let scale = edge / physical_width.max(physical_height);
    let width = (physical_width * scale).round().clamp(1.0, edge) as u32;
    let height = (physical_height * scale).round().clamp(1.0, edge) as u32;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    let center = f64::from(frame.level) - 0.5;
    let window = f64::from(frame.window) - 1.0;
    for y in 0..height {
        let sy =
            ((u64::from(y) * 2 + 1) * u64::from(frame.height) / (u64::from(height) * 2)) as usize;
        for x in 0..width {
            let sx =
                ((u64::from(x) * 2 + 1) * u64::from(frame.width) / (u64::from(width) * 2)) as usize;
            let value = f64::from(frame.values[sy * frame.width as usize + sx]);
            if !value.is_finite() {
                return None;
            }
            let gray = if value <= center - window / 2.0 {
                0
            } else if value > center + window / 2.0 {
                255
            } else {
                (((value - center) / window + 0.5) * 255.0).round() as u8
            };
            let gray = if frame.inverted { 255 - gray } else { gray };
            rgba.extend_from_slice(&[gray, gray, gray, 255]);
        }
    }
    Some((width, height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> dicom::Frame {
        dicom::Frame {
            width: 4,
            height: 1,
            pixel_aspect: 1.0,
            values: vec![-1.0, 0.0, 1.0, 2.0],
            window: 4.0,
            level: 1.0,
            supplied_window: true,
            contrast_samples: vec![],
            inverted: false,
        }
    }

    #[test]
    fn middle_slice_uses_sorted_series_order() {
        assert!(middle_file(&[]).is_none());
        for count in [1, 2, 3, 4, 99, 100] {
            let paths: Vec<_> = (0..count).map(|i| PathBuf::from(i.to_string())).collect();
            assert_eq!(middle_file(&paths), Some(paths[count / 2].as_path()));
        }
    }

    #[test]
    fn default_window_and_monochrome1_match_dicom_linear_reference() {
        let mut f = frame();
        for window in [1.0, 4.0] {
            f.window = window;
            for inverted in [false, true] {
                f.inverted = inverted;
                let (w, h, rgba) = downsample(&f, 4).unwrap();
                assert_eq!((w, h), (4, 1));
                assert_eq!(rgba, dicom::window_rgba(&f, f.window, f.level, false));
            }
        }
    }

    #[test]
    fn physical_aspect_and_budget_bound_output_not_source_rgba() {
        let mut f = frame();
        f.pixel_aspect = 8.0;
        let (w, h, rgba) = downsample(&f, 96).unwrap();
        assert_eq!((w, h), (48, 96));
        assert_eq!(rgba.len(), 48 * 96 * 4);
        for count in [1, 100, 1_000, 100_000] {
            let edge = max_edge(count) as usize;
            assert!(edge <= 96);
            assert!(count * edge * edge * 4 <= RGBA_BUDGET);
        }
        let (_, _, rgba) = downsample(&f, 1).unwrap();
        assert_eq!(rgba.len(), 4);
    }

    #[test]
    fn inbox_is_progressive_and_clear_disconnects_stale_generation() {
        let mut thumbs = Thumbnails::new(Arc::new(|| {}));
        let (old_sender, receiver) = mpsc::sync_channel(INBOX_CAPACITY);
        thumbs.inbox = Some(receiver);
        let preview = |id: &str| Preview {
            id: id.into(),
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        old_sender.send(preview("first")).unwrap();
        assert!(thumbs.poll());
        assert_eq!(thumbs.get("first").unwrap().size(), (1, 1));
        assert!(!thumbs.poll());
        old_sender.send(preview("second")).unwrap();
        assert!(thumbs.poll());
        assert!(thumbs.get("second").is_some());
        let cancelled = thumbs.cancelled.clone();
        old_sender.send(preview("stale-queued")).unwrap();
        thumbs.clear();
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(old_sender.send(preview("stale-running")).is_err());
        let (sender, receiver) = mpsc::sync_channel(INBOX_CAPACITY);
        thumbs.inbox = Some(receiver);
        sender.send(preview("current")).unwrap();
        assert!(thumbs.poll());
        assert!(thumbs.get("current").is_some());
        assert!(thumbs.get("stale-queued").is_none());
        assert!(thumbs.get("first").is_none());
    }

    #[test]
    fn cancellation_releases_a_sender_when_the_inbox_is_full() {
        let mut thumbs = Thumbnails::new(Arc::new(|| {}));
        let (sender, receiver) = mpsc::sync_channel(INBOX_CAPACITY);
        thumbs.inbox = Some(receiver);
        let preview = || Preview {
            id: "preview".into(),
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        for _ in 0..INBOX_CAPACITY {
            sender.send(preview()).unwrap();
        }
        let (finished, completion) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            finished.send(sender.send(preview()).is_err()).unwrap();
        });
        thumbs.clear();
        assert!(
            completion
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        );
        worker.join().unwrap();
        assert!(!thumbs.poll());
    }
}
