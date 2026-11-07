use super::*;

#[test]
fn demo_resolves_source_samples_from_the_working_directory() {
    assert_eq!(
        demo_samples_path(
            Path::new("/build/target/debug/fastdicom"),
            Path::new("/checkout")
        ),
        PathBuf::from("/checkout/samples")
    );
}

#[test]
fn demo_resolves_bundle_resources_independently_of_working_directory() {
    let executable = Path::new("/Applications/FastDICOM.app/Contents/MacOS/FastDICOM");
    for cwd in ["/", "/checkout"] {
        assert_eq!(
            demo_samples_path(executable, Path::new(cwd)),
            PathBuf::from("/Applications/FastDICOM.app/Contents/Resources/samples")
        );
    }
    // A MacOS directory outside an app bundle is not a packaged installation.
    assert_eq!(
        demo_samples_path(Path::new("/build/MacOS/fastdicom"), Path::new("/checkout")),
        PathBuf::from("/checkout/samples")
    );
}

#[test]
fn missing_demo_pack_reports_recovery_without_scheduling_a_fake_image() {
    let mut state = State::default();
    let missing =
        std::env::temp_dir().join(format!("fastdicom-missing-demo-{}", std::process::id()));
    assert!(!missing.exists());
    state.open_demo_path(missing.clone());
    let error = state.error.as_deref().unwrap();
    assert!(error.contains(&missing.display().to_string()));
    assert!(error.contains("scripts/bundle.sh"));
    assert!(error.contains("Open folder"));
    assert!(!state.scan.is_pending());
    assert!(state.series.is_empty());
    assert!(
        state
            .panes
            .iter()
            .all(|pane| pane.frame.is_none() && !pane.render.is_pending())
    );
}

#[test]
fn reset_geometry_clears_horizontal_flip_only_in_the_active_pane() {
    let mut state = State::default();
    assert!(state.panes.iter().all(|pane| !pane.horizontal_flip));
    for pane in &mut state.panes {
        pane.horizontal_flip = true;
        pane.rotation = std::f32::consts::FRAC_PI_2;
        pane.zoom = 2.0;
        pane.pan = (10.0, 20.0);
    }
    state.active = 1;
    state.reset_geometry();
    assert!(!state.panes[1].horizontal_flip);
    assert_eq!(state.panes[1].rotation, 0.0);
    assert_eq!(state.panes[1].zoom, 1.0);
    assert_eq!(state.panes[1].pan, (0.0, 0.0));
    assert!(state.panes[0].horizontal_flip);
    assert_eq!(state.panes[0].rotation, std::f32::consts::FRAC_PI_2);
}

fn contrast_study(supplied: bool) -> State {
    let mut s = State::default();
    s.series.push(dicom::Series {
        id: "study/series".into(),
        study_uid: "study".into(),
        study_date: None,
        series_number: None,
        description: "".into(),
        modality: "MR".into(),
        study_description: "".into(),
        patient_name: "".into(),
        patient_birth_date: None,
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
        ..fixtures::cached_study(160)
    };
    for p in &mut s.panes {
        p.selected = Some(0);
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
    assert_eq!(
        s.window,
        dicom::robust_window(&s.frame.as_ref().unwrap().contrast_samples).0
    );
    assert_eq!(s.window_revision, 0);
    assert!(!s.initialize_window);
    assert!(s.panes[1].frame.is_none());
    assert!(!s.panes[1].render.is_pending());
}

#[test]
fn clearing_one_latest_cannot_cancel_or_deliver_into_the_other_pane() {
    let mut s = State {
        split: true,
        ..fixtures::cached_study(160)
    };
    for pane in 0..2 {
        s.active = pane;
        s.selected = Some(0);
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
    assert_eq!(
        s.window,
        dicom::robust_window(&s.frame.as_ref().unwrap().contrast_samples).0
    );
}

#[test]
fn slice_navigation_clamps_and_preserves_requested_index() {
    let mut s = fixtures::cached_study(160);
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
    let mut s = State::default();
    s.apply_window();
    assert!(!s.render.is_pending());
    assert_eq!(s.window_revision, 1);
}

#[test]
fn first_window_initialization_survives_navigation_but_not_a_user_edit() {
    let mut s = fixtures::cached_study(160);
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
    assert_eq!(
        (s.window, s.level),
        dicom::robust_window(&s.frame.as_ref().unwrap().contrast_samples)
    );
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
    let mut state = fixtures::cached_study(160);
    state.frame = Some(Arc::new(synthetic_frame(80)));
    state.zoom = 2.0;
    state.pan = (20.0, 30.0);
    let mut runtime =
        rune::Runtime::new(state, ui::view, (1440.0, 940.0), rune::Env::new(true, 1.0));
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
    let mut state = fixtures::cached_study(160);
    state.initialize_window = true;
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
    assert_eq!(
        state.window,
        dicom::robust_window(&state.frame.as_ref().unwrap().contrast_samples).0
    );
    assert!(state.invert);
}

#[test]
fn pending_slice_keeps_the_displayed_image_visible() {
    let mut state = fixtures::cached_study(160);
    state.index = 1;
    state.frame = Some(Arc::new(synthetic_frame(0)));
    let mut runtime =
        rune::Runtime::new(state, ui::view, (1000.0, 680.0), rune::Env::new(true, 1.0));
    runtime.frame(0.0);
    assert!(runtime.find("image").is_some());
    assert!(!runtime.elements().iter().any(|element| {
        element
            .text
            .as_deref()
            .is_some_and(|text| text.starts_with("Loading slice"))
    }));
    assert!(runtime.find("loading-progress").is_some());
    assert!(runtime.find("image-loading").is_none());
    assert_eq!(runtime.state.displayed_index, 0);
}
