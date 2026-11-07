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
