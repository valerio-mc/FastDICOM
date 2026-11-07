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
                let (width, height, rgba) = downsample(&frame, edge);
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

// The private worker supplies only frames validated by dicom::decode.
fn downsample(frame: &dicom::Frame, edge: u32) -> (u32, u32, Vec<u8>) {
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
    (width, height, rgba)
}

#[cfg(test)]
#[path = "../tests/unit/thumbnails.rs"]
mod tests;
