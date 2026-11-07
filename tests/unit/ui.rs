use super::*;
use std::{path::PathBuf, sync::Arc};

fn runtime(state: State, dimensions: (f32, f32)) -> rune::Runtime<State> {
    rune::Runtime::new(state, view, dimensions, rune::Env::new(true, 1.0))
}

fn click(rt: &mut rune::Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn imported_study() -> State {
    State {
        series: vec![crate::dicom::Series {
            id: "study/series".into(),
            description: "PRIVATE-SERIES".into(),
            modality: "PRIVATE-MODALITY".into(),
            study_description: "PRIVATE-STUDY".into(),
            patient_name: "PRIVATE-PATIENT".into(),
            patient_id: "PRIVATE-ID".into(),
            patient_issuer: "".into(),
            files: vec![PathBuf::from("PRIVATE-FILE.dcm")],
        }],
        panes: [
            crate::Pane {
                selected: Some(0),
                frame: Some(Arc::new(crate::synthetic_frame(0))),
                ..crate::Pane::default()
            },
            crate::Pane::default(),
        ],
        ..State::default()
    }
}

fn exposed_text(rt: &rune::Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label, e.help])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn startup_prompt_is_centered_without_empty_sidebar_or_image_tools() {
    for (width, height) in [(1000.0, 680.0), (1600.0, 1000.0)] {
        let mut rt = runtime(State::default(), (width, height));
        rt.frame(0.0);
        assert!(rt.find("sidebar").is_none());
        assert!(rt.find("tool-dock").is_none());
        assert!(!exposed_text(&rt).contains("No series"));
        assert_eq!(
            rt.presented("viewport").unwrap().0,
            [0.0, 40.0, width, height - 40.0]
        );
        let heading = rt.presented("empty-heading").unwrap().0;
        let actions = rt.presented("empty-actions").unwrap().0;
        for rect in [heading, actions] {
            assert!((rect[0] + rect[2] * 0.5 - width * 0.5).abs() < 0.1);
        }
        let group_center = (heading[1] + actions[1] + actions[3]) * 0.5;
        assert!((group_center - (40.0 + (height - 40.0) * 0.5)).abs() < 0.1);
        click(&mut rt, "try-demo");
        assert!(rt.find("sidebar").is_some());
        assert!(rt.find("tool-dock").is_none());
        // The demo requests its image asynchronously; simulate its completed frame.
        rt.update(|s, _| s.frame = Some(Arc::new(crate::synthetic_frame(80))));
        rt.frame(0.0);
        assert!(rt.find("tool-dock").is_some());
        assert!(rt.find("empty-state").is_none());
    }
}

#[test]
fn privacy_toggle_removes_imported_text_and_accessibility_labels_then_restores_them() {
    let mut rt = runtime(imported_study(), (1000.0, 680.0));
    rt.frame(0.0);
    for token in [
        "PRIVATE-PATIENT",
        "PRIVATE-STUDY",
        "PRIVATE-SERIES",
        "PRIVATE-MODALITY",
    ] {
        assert!(exposed_text(&rt).contains(token));
    }
    let frame = rt.state.frame.clone().unwrap();
    for hidden in [true, false] {
        click(&mut rt, "privacy-toggle");
        assert_eq!(rt.state.privacy, hidden);
        assert_eq!(exposed_text(&rt).contains("PRIVATE-"), !hidden);
        assert!(Arc::ptr_eq(rt.state.frame.as_ref().unwrap(), &frame));
    }
}

#[test]
fn privacy_hides_error_paths_without_destroying_the_original_message() {
    let mut state = imported_study();
    state.privacy = true;
    state.frame = None;
    state.error = Some("Cannot read /patients/PRIVATE-PATIENT/PRIVATE-FILE.dcm".into());
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    assert!(!exposed_text(&rt).contains("PRIVATE-"));
    assert!(rt.state.error.as_ref().unwrap().contains("PRIVATE-PATIENT"));
}

#[test]
fn bottom_dock_leaves_metrics_and_canvas_gestures_accessible_at_minimum_size() {
    let state = State {
        panes: [
            crate::Pane {
                demo: true,
                frame: Some(Arc::new(crate::synthetic_frame(0))),
                tool: Tool::Pan,
                ..crate::Pane::default()
            },
            crate::Pane::default(),
        ],
        ..State::default()
    };
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    let (viewport, _) = rt.presented("viewport").unwrap();
    let (dock, _) = rt.presented("tool-dock").unwrap();
    let (metrics, _) = rt.presented("image-metrics").unwrap();
    let (index, _) = rt.presented("image-index").unwrap();
    assert_eq!(viewport[1], 40.0);
    assert_eq!(viewport[0], 240.0);
    assert!(rt.find("titlebar").is_some());
    assert!(rt.find("wordmark").is_none());
    assert!(metrics[0] + metrics[2] < dock[0]);
    assert!(dock[0] + dock[2] < index[0]);
    assert!((dock[0] + dock[2] * 0.5 - (viewport[0] + viewport[2] * 0.5)).abs() < 0.1);
    // The full-width positioning row must not swallow drags beside the dock.
    let x = viewport[0] + 10.0;
    let y = dock[1] + dock[3] * 0.5;
    rt.pointer_down(x, y);
    rt.pointer_move(x + 30.0, y - 5.0);
    rt.pointer_up(x + 30.0, y - 5.0);
    rt.frame(0.0);
    assert_eq!(rt.state.pan, (30.0, -5.0));
}

#[test]
fn header_sidebar_toggle_reflows_canvas_without_changing_the_study() {
    let mut state = imported_study();
    state.privacy = true;
    let frame = state.frame.clone().unwrap();
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.on_key = Some(crate::keys);
    rt.frame(0.0);
    let (toggle_x, toggle_y) = rt.find("toggle-sidebar").unwrap();
    let toggle = rt.presented("toggle-sidebar").unwrap().0;
    let open = rt.presented("open-folder-header").unwrap().0;
    let identity = rt.presented("app-identity").unwrap().0;
    let privacy = rt.presented("privacy-toggle").unwrap().0;
    assert!(toggle[0] >= 96.0);
    assert!(toggle[0] + toggle[2] <= open[0]);
    assert!(open[0] + open[2] <= identity[0]);
    assert!(identity[0] + identity[2] < privacy[0]);
    assert!(privacy[1] >= 0.0 && privacy[1] + privacy[3] <= 40.0);
    rt.pointer_down(toggle_x, toggle_y);
    rt.pointer_up(toggle_x, toggle_y);
    rt.frame(0.0);
    assert!(!rt.state.sidebar_visible);
    assert!(rt.find("sidebar").is_none());
    assert_eq!(
        rt.presented("viewport").unwrap().0,
        [0.0, 40.0, 1000.0, 640.0]
    );
    let dock = rt.presented("tool-dock").unwrap().0;
    assert!((dock[0] + dock[2] * 0.5 - 500.0).abs() < 0.1);
    rt.repeat_key(Key::Command('b').into());
    rt.frame(0.0);
    assert!(!rt.state.sidebar_visible);
    rt.key(Key::Command('b').into());
    rt.frame(0.0);
    assert!(rt.state.sidebar_visible);
    assert_eq!(rt.presented("viewport").unwrap().0[0], 240.0);
    assert!(rt.state.privacy);
    assert_eq!(rt.state.selected, Some(0));
    assert_eq!(rt.state.series.len(), 1);
    assert!(Arc::ptr_eq(rt.state.frame.as_ref().unwrap(), &frame));
    assert!(!exposed_text(&rt).contains("Privacy on"));
    assert!(!exposed_text(&rt).contains("Privacy off"));
}

#[test]
fn tool_selection_moves_without_shifting_button_hit_targets() {
    let mut rt = runtime(imported_study(), (1000.0, 680.0));
    rt.frame(0.0);
    let start = rt.presented("tool-selection").unwrap().0[0];
    let target = rt.find("tool-zoom").unwrap();
    rt.pointer_down(target.0, target.1);
    rt.pointer_up(target.0, target.1);
    rt.frame(0.016);
    assert!(rt.state.tool == Tool::Zoom);
    let during = rt.presented("tool-selection").unwrap().0[0];
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let end = rt.presented("tool-selection").unwrap().0[0];
    assert!(start <= during && during < end);
    assert!((end - start - 74.0).abs() < 0.1);
    assert_eq!(rt.find("tool-zoom").unwrap(), target);
}

#[test]
fn workspace_is_opaque_without_a_library_slab_or_footer() {
    let state = State {
        skipped_files: 2,
        ..imported_study()
    };
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    let elements = rt.elements();
    let root = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("fastdicom"))
        .unwrap();
    assert_eq!(root.rect, [0.0, 0.0, 1000.0, 680.0]);
    assert_eq!(root.fill.a, 1.0);
    let library = elements
        .iter()
        .find(|e| e.key.as_deref() == Some("library-surface"))
        .unwrap();
    assert_eq!(library.fill.a, 0.0);
    assert!(rt.find("statusbar").is_none());
    assert_eq!(rt.presented("viewport").unwrap().0[3], 640.0);
}

#[test]
fn series_hover_changes_glass_state_without_moving_the_row() {
    let mut rt = runtime(imported_study(), (1000.0, 680.0));
    rt.frame(0.0);
    let bounds = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("series-study/series"))
        .unwrap()
        .rect;
    rt.pointer_move(bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0);
    rt.frame(0.0);
    assert_eq!(rt.state.hovered_series.as_deref(), Some("study/series"));
    let hovered = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("series-study/series"))
        .unwrap()
        .rect;
    assert_eq!(hovered, bounds);
    rt.pointer_move(700.0, 100.0);
    rt.frame(0.0);
    assert!(rt.state.hovered_series.is_none());
}

#[test]
fn patient_details_float_top_right_and_gain_lens_only_on_hover() {
    let mut rt = runtime(imported_study(), (1000.0, 680.0));
    rt.track_marks = true;
    rt.frame(0.0);
    let panel = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("patient-info"))
        .unwrap();
    let bounds = panel.rect;
    assert_eq!(panel.fill.a, 0.0);
    assert_eq!(bounds[0] + bounds[2], 976.0);
    assert_eq!(bounds[1], 63.0);
    assert!(bounds[2] <= 304.0);
    assert!(!rt.marks.contains(&glass::PATIENT));
    rt.pointer_move(bounds[0] + bounds[2] * 0.5, bounds[1] + bounds[3] * 0.5);
    rt.frame(0.0);
    assert!(rt.state.hovered_patient_info);
    assert!(rt.marks.contains(&glass::PATIENT));
    assert_eq!(rt.presented("patient-info").unwrap().0, bounds);
    click(&mut rt, "privacy-toggle");
    assert!(rt.state.privacy);
    assert!(!rt.state.hovered_patient_info);
    assert!(rt.find("patient-info").is_none());
    assert!(!rt.marks.contains(&glass::PATIENT));
    assert!(!exposed_text(&rt).contains("PRIVATE-"));
}

#[test]
fn embedded_library_logo_decodes() {
    let cache = rune::ImageCache::new(Arc::new(|| {}));
    let image = cache.load(include_bytes!("../../assets/titlebar-icon.png").as_slice());
    image.wait().expect("embedded logo decodes");
    assert!(image.error().is_none());
    assert_eq!(image.size(), Some([128, 128]));
}

#[test]
fn split_dock_activation_resize_and_retention_use_real_pointer_input() {
    for size in [(1000.0, 680.0), (1600.0, 1000.0)] {
        let mut rt = runtime(imported_study(), size);
        rt.frame(0.0);
        let frame = rt.state.frame.clone().unwrap();
        click(&mut rt, "toggle-split");
        assert!(rt.state.split);
        let left = rt.presented("pane-0").unwrap().0;
        let right = rt.presented("pane-1").unwrap().0;
        assert!((left[2] - right[2]).abs() < 1.0, "{left:?} {right:?}");
        let (x, y) = rt.find("pane-divider").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x + 60.0, y);
        rt.pointer_up(x + 60.0, y);
        rt.frame(0.0);
        assert!(rt.state.split_ratio > 0.5 && rt.state.split_ratio < 0.7);
        click(&mut rt, "pane-1");
        assert_eq!(rt.state.active, 1);
        rt.update(|s, _| {
            s.selected = Some(0);
            s.frame = Some(frame.clone());
            s.zoom = 2.3;
        });
        rt.frame(0.0);
        click(&mut rt, "invert");
        assert!(rt.state.panes[1].invert);
        assert!(!rt.state.panes[0].invert);
        click(&mut rt, "toggle-split");
        assert!(!rt.state.split);
        assert_eq!(rt.state.active, 0);
        assert_eq!(rt.state.zoom, 2.3);
        assert!(rt.state.invert);
        assert!(Arc::ptr_eq(rt.state.frame.as_ref().unwrap(), &frame));
        assert!(rt.state.panes[1].frame.is_none());
    }
}

#[test]
fn runtime_pointer_drag_loads_target_then_keys_follow_active_pane_and_privacy_hides_both() {
    let state = draggable_study();
    let first = state.frame.clone().unwrap();
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.track_marks = true;
    rt.frame(0.0);
    rt.frame(0.0);
    let (source_x, source_y) = rt.find("series-study/second").unwrap();
    let at = rt.find("pane-1").unwrap();
    rt.pointer_down(source_x, source_y);
    rt.pointer_move(at.0, at.1);
    rt.frame(0.0);
    assert_eq!(rt.state.hovered_drop, Some(1));
    assert!(rt.marks.contains(&glass::DROP));
    assert!(!rt.events.iter().any(|event| event.starts_with("drag-out ")));
    rt.pointer_up(at.0, at.1);
    rt.frame(0.0);
    assert_eq!(rt.state.active, 1);
    assert_eq!(rt.state.panes[0].selected, Some(0));
    assert_eq!(rt.state.panes[1].selected, Some(1));
    assert!(Arc::ptr_eq(
        rt.state.panes[0].frame.as_ref().unwrap(),
        &first
    ));
    assert!(rt.state.hovered_drop.is_none());
    assert!(rt.state.series_drag.is_none());
    // Focus remains on the source row, but image shortcuts belong to the dropped pane.
    rt.key(Key::Down.into());
    rt.frame(0.0);
    rt.key(Key::Text("z".into()).into());
    rt.frame(0.0);
    assert_eq!(rt.state.panes[1].index, 1);
    assert!(rt.state.panes[1].tool == Tool::Zoom);
    assert_eq!(rt.state.panes[0].index, 0);
    assert!(rt.state.panes[0].tool == Tool::Window);
    rt.update(|s, _| {
        s.render.cancel();
        s.frame = Some(Arc::new(crate::synthetic_frame(4)));
    });
    rt.frame(0.0);
    assert!(exposed_text(&rt).contains("PRIVATE-SECOND-SERIES"));
    click(&mut rt, "privacy-toggle");
    assert!(!exposed_text(&rt).contains("PRIVATE-"));
    assert!(rt.state.panes.iter().all(|p| p.frame.is_some()));
    // A different patient replaces the shared patient context, never leaving mixed panes.
    rt.update(|s, _| {
        s.series[1].patient_id = "OTHER-PATIENT".into();
        s.load_series(1, 1);
    });
    rt.frame(0.0);
    assert!(rt.state.panes[0].selected.is_none());
    assert!(rt.state.panes[0].frame.is_none());
    assert!(rt.state.privacy);
}

#[test]
fn split_gestures_and_shared_controls_do_not_mutate_inactive_presentation() {
    let mut state = imported_study();
    state.split = true;
    state.panes[1] = crate::Pane {
        demo: true,
        frame: state.frame.clone(),
        ..crate::Pane::default()
    };
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    let (x, y) = rt.find("pane-1").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.key(Key::Text("p".into()).into());
    rt.frame(0.0);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 20.0, y + 15.0);
    rt.pointer_up(x + 20.0, y + 15.0);
    rt.frame(0.0);
    assert_eq!(rt.state.panes[1].pan, (20.0, 15.0));
    assert_eq!(rt.state.panes[0].pan, (0.0, 0.0));
    rt.wheel_event(rune::WheelEvent::new(0.0, -16.0).at(x, y));
    rt.frame(0.0);
    assert_eq!(rt.state.panes[1].index, 1);
    assert_eq!(rt.state.panes[0].index, 0);
    // A window edit belongs only to pane 1, including its revision counter.
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.key(Key::Text("w".into()).into());
    rt.frame(0.0);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 40.0, y - 20.0);
    rt.pointer_up(x + 40.0, y - 20.0);
    rt.frame(0.0);
    assert!(rt.state.panes[1].window > 400.0);
    assert!(rt.state.panes[1].window_revision > 0);
    assert_eq!(rt.state.panes[0].window, 400.0);
    assert_eq!(rt.state.panes[0].window_revision, 0);
    click(&mut rt, "rotate");
    assert_eq!(rt.state.panes[1].rotation, std::f32::consts::FRAC_PI_2);
    assert_eq!(rt.state.panes[0].rotation, 0.0);
}

#[test]
fn patient_matching_is_conservative_for_missing_or_conflicting_identifiers() {
    let s = imported_study();
    let a = &s.series[0];
    let mut b = a.clone();
    b.id = "other-study/second".into();
    assert!(a.same_patient(&b));
    b.patient_issuer = "other-hospital".into();
    assert!(!a.same_patient(&b));
    b.patient_id.clear();
    assert!(!a.same_patient(&b));
    b.id = "study/second".into();
    assert!(a.same_patient(&b));
    b.patient_name = "other-patient".into();
    assert!(!a.same_patient(&b));
}

fn draggable_study() -> State {
    let mut state = imported_study();
    let mut second = state.series[0].clone();
    second.id = "study/second".into();
    second.description = "PRIVATE-SECOND-SERIES".into();
    second.files = vec![
        "PRIVATE-SECOND.dcm".into(),
        "PRIVATE-THIRD.dcm".into(),
        "PRIVATE-FOURTH.dcm".into(),
    ];
    state.series.push(second);
    state.split = true;
    state
}

#[test]
fn complete_pointer_drags_reach_either_pane_without_os_sessions_or_private_drag_labels() {
    for pane in 0..2 {
        for size in [(1000.0, 680.0), (1600.0, 1000.0)] {
            let mut state = draggable_study();
            state.privacy = true;
            let retained = state.frame.clone().unwrap();
            state.panes[1].selected = Some(0);
            state.panes[1].frame = Some(retained.clone());
            state.active = 1 - pane;
            let mut rt = runtime(state, size);
            rt.track_marks = true;
            rt.frame(0.0);
            rt.frame(0.0);
            let source = rt.find("series-study/second").unwrap();
            let at = rt.find(&format!("pane-{pane}")).unwrap();
            rt.pointer_down(source.0, source.1);
            rt.frame(0.0);
            rt.pointer_move(at.0, at.1);
            rt.frame(0.0);
            assert_eq!(rt.state.hovered_drop, Some(pane));
            assert!(rt.marks.contains(&glass::DROP));
            assert!(!exposed_text(&rt).contains("PRIVATE-"));
            rt.pointer_up(at.0, at.1);
            rt.frame(0.0);
            assert_eq!(rt.state.active, pane);
            assert_eq!(rt.state.panes[pane].selected, Some(1));
            assert_eq!(rt.state.panes[1 - pane].selected, Some(0));
            assert!(Arc::ptr_eq(
                rt.state.panes[1 - pane].frame.as_ref().unwrap(),
                &retained
            ));
            assert!(rt.state.series_drag.is_none());
            assert!(rt.state.hovered_drop.is_none());
            assert!(!rt.events.iter().any(|event| event.starts_with("drag-out ")));
        }
    }
}

#[test]
fn series_click_threshold_outside_escape_and_return_to_source_are_distinct() {
    for finish in ["click", "outside", "escape", "source"] {
        let state = draggable_study();
        let frame = state.frame.clone().unwrap();
        let mut rt = runtime(state, (1000.0, 680.0));
        rt.frame(0.0);
        rt.frame(0.0);
        let source = rt.find("series-study/second").unwrap();
        let target = rt.find("pane-1").unwrap();
        rt.pointer_down(source.0, source.1);
        rt.frame(0.0);
        if finish == "click" {
            rt.pointer_move(source.0 + 2.0, source.1);
            rt.frame(0.0);
            assert!(rt.state.hovered_drop.is_none());
            rt.pointer_up(source.0 + 2.0, source.1);
        } else {
            rt.pointer_move(target.0, target.1);
            rt.frame(0.0);
            assert_eq!(rt.state.hovered_drop, Some(1));
            match finish {
                "outside" => {
                    rt.pointer_move(1020.0, 700.0);
                    rt.frame(0.0);
                    assert!(rt.state.hovered_drop.is_none());
                    rt.pointer_up(1020.0, 700.0);
                }
                "escape" => {
                    rt.key(Key::Escape.into());
                    rt.frame(0.0);
                    rt.pointer_up(target.0, target.1);
                }
                "source" => {
                    rt.pointer_move(source.0, source.1);
                    rt.frame(0.0);
                    rt.pointer_up(source.0, source.1);
                }
                _ => unreachable!(),
            }
        }
        rt.frame(0.0);
        assert!(rt.state.hovered_drop.is_none(), "{finish}");
        assert!(rt.state.series_drag.is_none(), "{finish}");
        assert!(rt.state.panes[1].selected.is_none(), "{finish}");
        assert_eq!(
            rt.state.panes[0].selected,
            Some(if finish == "click" { 1 } else { 0 }),
            "{finish}"
        );
        if finish != "click" {
            assert!(Arc::ptr_eq(rt.state.frame.as_ref().unwrap(), &frame));
        }
    }
}

#[test]
fn keys_follow_active_pane_after_wheel_or_pinch_despite_left_keyboard_focus() {
    let mut state = draggable_study();
    state.panes[1].selected = Some(1);
    state.panes[1].frame = state.frame.clone();
    let mut rt = runtime(state, (1000.0, 680.0));
    rt.frame(0.0);
    let left = rt.find("pane-0").unwrap();
    let right = rt.find("pane-1").unwrap();
    rt.pointer_down(left.0, left.1);
    rt.pointer_up(left.0, left.1);
    rt.frame(0.0);
    rt.wheel_event(rune::WheelEvent::new(0.0, -16.0).at(right.0, right.1));
    rt.frame(0.0);
    rt.key(Key::Down.into());
    rt.frame(0.0);
    rt.key(Key::Text("p".into()).into());
    rt.frame(0.0);
    assert_eq!(rt.state.active, 1);
    assert_eq!(rt.state.panes[1].index, 2);
    assert!(rt.state.panes[1].tool == Tool::Pan);
    rt.magnify_event(rune::MagnifyEvent::new(0.25).at(right.0, right.1));
    rt.frame(0.0);
    rt.key(Key::Up.into());
    rt.frame(0.0);
    rt.key(Key::Text("z".into()).into());
    rt.frame(0.0);
    assert_eq!(rt.state.active, 1);
    assert_eq!(rt.state.panes[1].index, 1);
    assert_eq!(rt.state.panes[1].zoom, 1.25);
    assert!(rt.state.panes[1].tool == Tool::Zoom);
    assert_eq!(rt.state.panes[0].index, 0);
    assert_eq!(rt.state.panes[0].zoom, 1.0);
    assert!(rt.state.panes[0].tool == Tool::Window);
}

#[test]
fn ct_native_menu_and_auto_are_active_pane_controls_without_privacy_leaks_or_dock_overlap() {
    for size in [(1000.0, 680.0), (1600.0, 1000.0)] {
        for active in 0..2 {
            let mut state = draggable_study();
            state.privacy = true;
            state.series[0].modality = "CT".into();
            state.series[0].files = vec!["first".into(), "next".into()];
            state.panes[1].selected = Some(0);
            state.panes[1].frame = state.panes[0].frame.clone();
            state.active = active;
            state
                .series_windows
                .insert("study/series".into(), (321.0, 123.0));
            let frame = state.frame.clone().unwrap();
            let mut rt = runtime(state, size);
            rt.frame(0.0);
            let dock = rt.presented("tool-dock").unwrap().0;
            assert!(dock[0] >= 240.0 && dock[0] + dock[2] <= size.0);
            for metrics in rt
                .elements()
                .into_iter()
                .filter(|e| matches!(e.key.as_deref(), Some("image-metrics" | "image-index")))
            {
                assert!(
                    metrics.rect[1] + metrics.rect[3] <= dock[1],
                    "metrics must stay above dock"
                );
            }
            click(&mut rt, "ct-presets");
            let menu = rt.popup_menu().expect("real Rune popup request");
            assert!(menu.find("soft").unwrap().title().contains("400"));
            assert!(menu.find("lung").unwrap().title().contains("1500"));
            assert!(menu.find("bone").unwrap().is_enabled());
            rt.resolve_popup(Some("bone")).unwrap();
            rt.frame(0.0);
            assert_eq!((rt.state.window, rt.state.level), (2000.0, 400.0));
            assert_eq!(rt.state.panes[1 - active].window_revision, 0);
            assert!(!rt.state.render.is_pending());
            assert!(Arc::ptr_eq(rt.state.frame.as_ref().unwrap(), &frame));
            rt.update(|s, _| s.step(1));
            rt.frame(0.0);
            assert_eq!((rt.state.window, rt.state.level), (2000.0, 400.0));
            rt.update(|s, _| s.render.cancel());
            click(&mut rt, "auto-contrast");
            assert_eq!((rt.state.window, rt.state.level), (321.0, 123.0));
            assert_eq!(
                (
                    rt.state.panes[1 - active].window,
                    rt.state.panes[1 - active].level
                ),
                (400.0, 40.0)
            );
            assert!(
                !rt.state.contrast.is_pending(),
                "completed result is reused"
            );
            assert!(!exposed_text(&rt).contains("PRIVATE-"));
            rt.update(|s, _| s.series[0].modality = "MR".into());
            rt.frame(0.0);
            assert!(rt.find("ct-presets").is_none());
            assert!(rt.find("auto-contrast").is_some());
        }
    }
}
