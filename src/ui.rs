use super::{CtPreset, State, Tool, icons, open_dialog};
use crate::glass;
use rune::{
    Color, Cx, Drag, El, GesturePhase, Key, KeyEvent, Phase, ScrollSpec, TextStyle, Tx, col, el,
    row, text,
};

fn ink(hex: &str) -> Color {
    rune::color::hex(hex).expect("authored color")
}
fn foreground() -> Color {
    ink("#EDF1F5")
}
fn muted() -> Color {
    ink("#AAB4C1")
}
fn accent() -> Color {
    ink("#D9FF67")
}
fn selection() -> Color {
    accent()
}
fn frost() -> Color {
    accent().alpha(0.08)
}
fn line() -> Color {
    ink("#282E37")
}
fn label(value: impl Into<String>, size: f32, weight: f32, color: Color) -> El {
    text(value).font(TextStyle::new(size, weight)).color(color)
}

fn control(key: &str, title: &str, active: bool) -> El {
    row()
        .key(key)
        .h(34.0)
        .px(12.0)
        .center()
        .gap(7.0)
        .radius(8.0)
        .fill(if active {
            accent().alpha(0.13)
        } else {
            Color::TRANSPARENT
        })
        .hover(frost())
        .press(Color::WHITE.alpha(0.12))
        .focus_ring(1.5, 2.0, accent())
        .role("button")
        .label(title)
        .help(title)
        .focusable(true)
}

fn button(key: &str, title: &str, active: bool) -> El {
    control(key, title, active).child(
        label(
            title,
            12.0,
            550.0,
            if active { accent() } else { foreground() },
        )
        .a11y_hidden(),
    )
}

fn icon_button(key: &str, title: &str, glyph: &'static rune::Icon, active: bool) -> El {
    control(key, title, active).w(34.0).px(0.0).child(
        rune::icon(glyph, 18.0)
            .color(if active { accent() } else { foreground() })
            .a11y_hidden(),
    )
}

fn open_button(key: &str, primary: bool) -> El {
    let color = if primary {
        ink("#14180D")
    } else {
        foreground()
    };
    control(key, "Open folder", false)
        .fill(if primary {
            accent()
        } else {
            Color::WHITE.alpha(0.06)
        })
        .hover(if primary { ink("#E4FF91") } else { frost() })
        .border(
            1.0,
            if primary {
                accent().alpha(0.22)
            } else {
                Color::WHITE.alpha(0.12)
            },
        )
        .shortcut_hint("⌘O")
        .child(rune::icon(&icons::FOLDER, 16.0).color(color).a11y_hidden())
        .child(label("Open folder", 12.0, 550.0, color).a11y_hidden())
        .on_press_tx(open_dialog)
}

fn separator() -> El {
    el().w(1.0).h(20.0).fill(line()).mx(5.0).a11y_hidden()
}

fn reset(s: &mut State) {
    s.reset_geometry();
    s.invert = false;
    if let Some(frame) = s.frame.clone() {
        let window = s
            .selected
            .and_then(|i| s.series.get(i))
            .and_then(|srs| s.series_windows.get(&srs.id))
            .copied();
        (s.window, s.level) = if frame.supplied_window {
            (frame.window, frame.level)
        } else {
            window.unwrap_or((frame.window, frame.level))
        };
    }
    s.apply_window();
}

pub(super) fn view(s: &State, cx: &mut Cx<State>) -> El {
    cx.foreground(foreground());
    cx.focus_ring(1.5, 2.0, accent());
    col()
        .key("fastdicom")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(ink("#050607"))
        .on_key(viewport_key)
        .child(header(s))
        .child(
            row()
                .key("workspace")
                .grow(1.0)
                .min_h(0.0)
                .min_w(0.0)
                .children((s.sidebar_visible && !s.series.is_empty()).then(|| sidebar(s)))
                .child(viewports(s)),
        )
}

fn header(s: &State) -> El {
    row()
        .key("titlebar")
        .w_full()
        .h(40.0)
        .pl(96.0)
        .pr(12.0)
        .gap(6.0)
        .center_y()
        .fill(ink("#101318"))
        .window_drag()
        .child(row().key("app-identity").center_y().gap(7.0)
            .child(app_logo(s))
            .child(label("FastDICOM", 14.0, 500.0, foreground())))
        .child(el().grow(1.0))
        .child(icon_button("toggle-sidebar", "Toggle sidebar · ⌘B", &icons::SIDEBAR, s.sidebar_visible)
            .size(28.0, 28.0)
            .checked(s.sidebar_visible)
            .shortcut_hint("⌘B")
            .on_press_tx(|s: &mut State, tx| {
                s.sidebar_visible = !s.sidebar_visible;
                tx.instant();
            }))
        .child(icon_button("open-folder-header", "Open folder · ⌘O", &icons::FOLDER, false)
            .size(28.0, 28.0)
            .shortcut_hint("⌘O")
            .help(if s.skipped_files > 0 {
                format!("Open folder · ⌘O · {} non-image or unreadable files skipped during import", s.skipped_files)
            } else { "Open folder · ⌘O".into() })
            .on_press_tx(open_dialog))
        .child(icon_button("privacy-toggle", "Hide patient details", if s.privacy { &icons::EYE_OFF } else { &icons::EYE }, s.privacy)
            .size(30.0, 28.0)
            .fill(if s.privacy { accent().alpha(0.10) } else { Color::WHITE.alpha(0.04) })
            .role("switch")
            .label("Hide patient details")
            .checked(s.privacy)
            .help(if s.privacy { "Show patient details · ⇧⌘P" } else { "Hide patient details · ⇧⌘P" })
            .description("Hides patient names, birth dates, study titles and dates, series descriptions, modalities, and error details. Does not remove text burned into images.")
            .shortcut_hint("⇧⌘P")
            .on_press_tx(|s: &mut State, tx| { s.toggle_privacy(); tx.instant(); }))
}

fn tool_dock(s: &State) -> El {
    let ready = s.frame.is_some() && s.error.is_none();
    let mut dock = glass::surface(
        row()
            .key("tool-dock")
            .padding(6.0)
            .gap(3.0)
            .center_y()
            .radius(12.0),
        glass::DOCK,
        0.025,
    )
    .child(
        el().key("tool-selection")
            .absolute(6.0, 6.0)
            .size(34.0, 34.0)
            .offset(
                match s.tool {
                    Tool::Window => 0.0,
                    Tool::Pan => 37.0,
                    Tool::Zoom => 74.0,
                },
                0.0,
            )
            .offset_motion(rune::Spring::new(0.18, 0.0))
            .radius(8.0)
            .fill(accent().alpha(0.13))
            .child(
                el().pin(Some(8.0), Some(0.0), Some(8.0), None)
                    .h(1.0)
                    .fill(accent().alpha(0.65)),
            )
            .a11y_hidden(),
    );
    for (key, title, tool, glyph) in [
        (
            "tool-window",
            "Window / level · W",
            Tool::Window,
            &icons::WINDOW,
        ),
        ("tool-pan", "Pan · P", Tool::Pan, &icons::PAN),
        ("tool-zoom", "Zoom · Z", Tool::Zoom, &icons::ZOOM),
    ] {
        dock = dock.child(
            icon_button(key, title, glyph, s.tool == tool)
                .fill(Color::TRANSPARENT)
                .checked(s.tool == tool)
                .on_press(move |s: &mut State| s.tool = tool),
        );
    }
    dock = dock.child(separator());
    dock = dock.child(
        icon_button(
            "toggle-split",
            "Toggle side-by-side panes",
            &icons::SPLIT,
            s.split,
        )
        .checked(s.split)
        .on_press_tx(|s: &mut State, tx| {
            s.toggle_split();
            tx.instant();
        }),
    );
    let fit = icon_button("fit", "Fit image · F", &icons::FIT, false);
    let reset_button = icon_button("reset", "Reset · R", &icons::RESET, false);
    let invert = icon_button("invert", "Invert · I", &icons::INVERT, s.invert).checked(s.invert);
    let flip = icon_button(
        "flip-horizontal",
        "Flip horizontally",
        &icons::FLIP_HORIZONTAL,
        s.horizontal_flip,
    )
    .checked(s.horizontal_flip);
    let rotate = icon_button("rotate", "Rotate clockwise 90°", &icons::ROTATE, false);
    let auto = icon_button("auto-contrast", "Auto contrast · A", &icons::AUTO, false)
        .help("Auto contrast · A · 1–99% of valid intensities from up to five representative slices; excludes declared padding.");
    dock = if ready {
        dock.child(fit.on_press_tx(|s: &mut State, tx| {
            s.reset_geometry();
            tx.instant();
        }))
        .child(reset_button.on_press_tx(|s: &mut State, tx| {
            reset(s);
            tx.instant();
        }))
        .child(auto.on_press_tx(|s: &mut State, tx| {
            s.request_auto(s.active, true);
            tx.instant();
        }))
        .child(invert.on_press(|s: &mut State| s.toggle_invert()))
        .child(flip.on_press_tx(|s: &mut State, tx| {
            s.horizontal_flip = !s.horizontal_flip;
            tx.instant();
        }))
        .child(rotate.on_press_tx(|s: &mut State, tx| {
            s.rotation = (s.rotation + std::f32::consts::FRAC_PI_2) % std::f32::consts::TAU;
            tx.instant();
        }))
    } else {
        dock.child(fit.disabled(true).dim(0.35))
            .child(reset_button.disabled(true).dim(0.35))
            .child(auto.disabled(true).dim(0.35))
            .child(invert.disabled(true).dim(0.35))
            .child(flip.disabled(true).dim(0.35))
            .child(rotate.disabled(true).dim(0.35))
    };
    if s.is_ct() {
        dock = dock.child(
            icon_button("ct-presets", "CT window presets", &icons::PRESETS, false)
                .disabled(!ready)
                .dim(if ready { 1.0 } else { 0.35 })
                .on_press_tx(|s: &mut State, tx| {
                    let mut menu = rune::PopupMenu::new();
                    for (id, title, preset) in [
                        ("soft", "Soft tissue · W 400 / L 40", CtPreset::SoftTissue),
                        ("lung", "Lung · W 1500 / L −600", CtPreset::Lung),
                        ("bone", "Bone · W 2000 / L 400", CtPreset::Bone),
                    ] {
                        menu = menu.item(
                            rune::PopupItem::new(id, title)
                                .checked((s.window, s.level) == preset.window()),
                        );
                    }
                    tx.popup_menu(menu, "ct-presets", |s: &mut State, picked, tx| {
                        if let Some(preset) = match picked {
                            Some("soft") => Some(CtPreset::SoftTissue),
                            Some("lung") => Some(CtPreset::Lung),
                            Some("bone") => Some(CtPreset::Bone),
                            _ => None,
                        } {
                            s.ct_preset(preset);
                            tx.instant();
                        }
                    });
                }),
        );
    }
    // Only the autosized dock paints/handles input; the centering row has no handler.
    row()
        .key("tool-dock-position")
        .pin(Some(0.0), None, Some(0.0), Some(16.0))
        .center()
        .child(dock)
}

fn app_logo(s: &State) -> El {
    let logo = el().key("app-logo").size(20.0, 20.0).a11y_hidden();
    if let Some(image) = &s.logo {
        logo.pixels_fit(image.pixels(), rune::SurfaceFit::Fit)
    } else {
        logo
    }
}

fn series_title(s: &State, index: usize) -> String {
    match s.series.get(index) {
        Some(series) if !s.privacy && !series.description.is_empty() => series.description.clone(),
        _ => format!("Series {:02}", index + 1),
    }
}

fn series_preview(s: &State, id: &str, modality: &str, selected: bool) -> El {
    let tile = col()
        .size(64.0, 64.0)
        .center()
        .radius(8.0)
        .fill(ink("#050607"))
        .clip_rounded(8.0)
        .a11y_hidden();
    if let Some(pixels) = s.thumbnails.as_ref().and_then(|previews| previews.get(id)) {
        tile.child(
            el().key("preview-image")
                .w_full()
                .h_full()
                .pixels_fit(pixels, rune::SurfaceFit::Fit),
        )
    } else {
        tile.child(
            label(
                modality,
                10.0,
                650.0,
                if selected { selection() } else { muted() },
            )
            .max_w(60.0)
            .truncate(),
        )
    }
}

fn image_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "image" } else { "images" })
}

fn study_header(s: &State, start: usize, end: usize, number: usize) -> El {
    let study = &s.series[start..end];
    let title = if s.privacy {
        None
    } else {
        study
            .iter()
            .find(|series| !series.study_description.is_empty())
    }
    .map(|series| series.study_description.clone())
    .unwrap_or_else(|| format!("Study {number:02}"));
    let date = if s.privacy {
        "Date hidden".to_owned()
    } else {
        study
            .iter()
            .find_map(|series| series.study_date.as_ref())
            .map_or_else(|| "Date unavailable".into(), ToString::to_string)
    };
    let mut modalities = Vec::new();
    if !s.privacy {
        for series in study {
            if !series.modality.is_empty() && !modalities.contains(&series.modality.as_str()) {
                modalities.push(series.modality.as_str());
            }
        }
    }
    let modalities = if modalities.is_empty() {
        "DICOM".into()
    } else {
        modalities.join(" / ")
    };
    col()
        .key(format!("study-{}", study[0].study_uid))
        .w_full()
        .h(60.0)
        .mt(if number == 1 { 0.0 } else { 8.0 })
        .px(8.0)
        .py(8.0)
        .gap(5.0)
        .child(label(title, 12.0, 600.0, foreground()).w_full().truncate())
        .child(
            label(format!("{date} · {modalities}"), 10.0, 450.0, muted())
                .w_full()
                .truncate(),
        )
        .child(
            el().pin(Some(8.0), None, Some(8.0), Some(0.0))
                .h(1.0)
                .fill(Color::WHITE.alpha(0.10))
                .a11y_hidden(),
        )
}

fn sidebar(s: &State) -> El {
    let mut list = col().key("series-list").w_full().gap(8.0).padding(8.0);
    let mut study_number = 0;
    for (index, series) in s.series.iter().enumerate() {
        if index == 0 || s.series[index - 1].study_uid != series.study_uid {
            study_number += 1;
            let end = s.series[index..]
                .iter()
                .position(|next| next.study_uid != series.study_uid)
                .map_or(s.series.len(), |offset| index + offset);
            list = list.child(study_header(s, index, end, study_number));
        }
        let selected = s.selected == Some(index);
        let hovered = s.hovered_series.as_deref() == Some(series.id.as_str());
        let raised = selected || hovered;
        // Modality is metadata, not a fabricated image preview.
        let key = &series.id;
        let series_id = series.id.clone();
        let hover_id = series.id.clone();
        let drag_id = series.id.clone();
        let layout_id = series.id.clone();
        let name = series_title(s, index);
        let modality = if s.privacy { "DICOM" } else { &series.modality };
        let item = row()
            .key(format!("series-{key}"))
            .w_full()
            .h(86.0)
            .padding(8.0)
            .gap(9.0)
            .center_y()
            .radius(10.0)
            .on_hover_change(move |s: &mut State, hovering, _| {
                if hovering {
                    s.hovered_series = Some(hover_id.clone());
                } else if s.hovered_series.as_deref() == Some(hover_id.as_str()) {
                    s.hovered_series = None;
                }
            })
            .focus_ring(1.5, 1.0, accent())
            .role("button")
            .label(format!(
                "{name}, {}, {}",
                modality,
                image_count(series.files.len())
            ))
            .checked(selected)
            .focusable(true)
            .on_drawn_layout_tx(move |s: &mut State, rect, tx| {
                s.series_rects.insert(layout_id.clone(), rect);
                tx.live_only();
            })
            .on_drag(move |s: &mut State, drag, tx| {
                s.drag_series(&drag_id, drag);
                tx.instant();
            })
            .on_press(move |s: &mut State| {
                if s.suppressed_series_press.take().as_deref() == Some(series_id.as_str()) {
                    return;
                }
                if let Some(index) = s.series.iter().position(|series| series.id == series_id) {
                    s.select(index);
                }
            })
            .children(selected.then(|| {
                el().pin(Some(0.0), Some(18.0), None, Some(18.0))
                    .w(2.0)
                    .radius(1.0)
                    .fill(selection().alpha(0.8))
                    .a11y_hidden()
            }))
            .child(series_preview(s, &series.id, modality, selected))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(6.0)
                    .child(label(name, 12.0, 600.0, foreground()).w_full().truncate())
                    .child(label(
                        format!("{modality} · {}", image_count(series.files.len())),
                        10.0,
                        450.0,
                        muted(),
                    )),
            );
        list = list.child(if raised {
            glass::surface(
                item,
                if selected {
                    glass::SELECTED_ROW
                } else {
                    glass::HOVER_ROW
                },
                0.025,
            )
        } else {
            item
        });
    }
    if s.series.is_empty() {
        list = list.child(
            label("No series", 11.0, 400.0, muted())
                .key("no-series")
                .padding(9.0),
        );
    }
    col()
        .key("sidebar")
        .w(240.0)
        .h_full()
        .min_h(0.0)
        .padding(8.0)
        .child(
            col()
                .key("library-surface")
                .w_full()
                .h_full()
                .min_h(0.0)
                .radius(12.0)
                .child(
                    col()
                        .key("series-scroll")
                        .grow(1.0)
                        .min_h(0.0)
                        .w_full()
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(list),
                ),
        )
}

fn empty_state() -> El {
    col()
        .key("empty-state")
        .w_full()
        .h_full()
        .center()
        .gap(20.0)
        .child(label("Open a DICOM folder", 22.0, 500.0, foreground()).key("empty-heading"))
        .child(
            row()
                .key("empty-actions")
                .gap(12.0)
                .center()
                .mt(12.0)
                .child(open_button("open-folder-empty", true))
                .child(
                    button("try-demo", "Demo", false)
                        .border(1.0, line())
                        .on_press(|s: &mut State| s.demo()),
                ),
        )
}

// A narrow glyph outline keeps floating text readable on either white or black
// image pixels without painting a permanent panel behind it.
fn floating_label(value: impl Into<String>, size: f32, weight: f32, color: Color) -> El {
    let value = value.into();
    let mut outlined = col().min_w(0.0).max_w(288.0);
    for (x, y) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        outlined = outlined.child(
            label(&value, size, weight, Color::BLACK.alpha(0.9))
                .max_w(288.0)
                .truncate()
                .absolute(x, y)
                .a11y_hidden(),
        );
    }
    outlined.child(label(value, size, weight, color).max_w(288.0).truncate())
}

fn patient_info(s: &State) -> Option<El> {
    let series = s
        .selected
        .or_else(|| s.panes.iter().find_map(|p| p.selected))
        .filter(|_| !s.privacy)
        .and_then(|i| s.series.get(i))?;
    if series.patient_name.is_empty()
        && series.patient_birth_date.is_none()
        && series.study_description.is_empty()
    {
        return None;
    }
    let panel = col()
        .key("patient-info")
        .pin(None, Some(23.0), Some(24.0), None)
        .max_w(304.0)
        .padding(8.0)
        .gap(6.0)
        .radius(12.0)
        .on_hover_change(|s: &mut State, hovering, _| s.hovered_patient_info = hovering)
        .children((!series.patient_name.is_empty()).then(|| {
            floating_label(&series.patient_name, 12.0, 600.0, foreground()).key("patient-name")
        }))
        .children(series.patient_birth_date.as_ref().map(|date| {
            floating_label(format!("Born {date}"), 11.0, 450.0, foreground())
                .key("patient-birth-date")
        }))
        .children((!series.study_description.is_empty()).then(|| {
            floating_label(&series.study_description, 11.0, 450.0, foreground())
                .key("patient-study")
        }));
    Some(if s.hovered_patient_info {
        glass::surface(panel, glass::PATIENT, 0.025)
    } else {
        panel
    })
}

fn viewports(s: &State) -> El {
    let mut panes = row().w_full().h_full().min_w(0.0).min_h(0.0);
    if s.split {
        panes = panes
            .child(viewport(s, 0).grow(s.split_ratio))
            .child(
                el().key("pane-divider")
                    .w(6.0)
                    .h_full()
                    .fill(line())
                    .hover(accent().alpha(0.4))
                    .cursor(rune::Cursor::ResizeColumn)
                    .role("separator")
                    .label("Resize image panes")
                    .description("Drag, or use Left and Right arrow keys to resize the panes.")
                    .focusable(true)
                    .on_key(|s: &mut State, event, tx| {
                        let delta = match event.key {
                            Key::Left => -0.05,
                            Key::Right => 0.05,
                            _ => return false,
                        };
                        s.resize_split(s.split_ratio + delta);
                        tx.instant();
                        true
                    })
                    .on_drag(|s: &mut State, drag, tx| {
                        if drag.phase == Phase::Down {
                            s.divider_origin = s.split_ratio;
                        }
                        // Drag coordinates are window points; the viewport width is provided by Rune.
                        if drag.cancelled() {
                            s.split_ratio = s.divider_origin;
                        } else {
                            s.resize_split(
                                s.divider_origin + drag.dx / (s.viewport_width - 6.0).max(1.0),
                            );
                        }
                        tx.instant();
                    }),
            )
            .child(viewport(s, 1).grow(1.0 - s.split_ratio));
    } else {
        panes = panes.child(viewport(s, 0));
    }
    let mut workspace = el()
        .key("viewport")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .on_layout(|s: &mut State, rect| {
            s.viewport_width = rect[2];
            if s.split {
                s.resize_split(s.split_ratio);
            }
        })
        .fill(ink("#050607"))
        .child(panes);
    workspace = workspace.children(patient_info(s));
    if s.panes.iter().any(|p| p.frame.is_some()) {
        workspace = workspace.child(tool_dock(s));
    }
    workspace
}

fn viewport(s: &State, pane: usize) -> El {
    let p = &s.panes[pane];
    let count = p.selected.map_or(0, |i| s.series[i].files.len());
    let pending = s.scan.is_pending()
        || p.render.is_pending()
        || (p.frame.is_some() && p.index != p.displayed_index);
    let mut canvas = el().key(format!("pane-{pane}")).grow(1.0).min_w(0.0).min_h(0.0).h_full()
        .border(if s.split { 1.0 } else { 0.0 }, if s.active == pane { accent().alpha(0.5) } else { line() })
        .fill(ink("#050607")).clip_rounded(0.0).focusable(true)
        .role("group").label("Image viewport")
        .description("Scroll to change slice. Drag to use the selected tool. W window, P pan, Z zoom, I invert, F fit, R reset.")
        .focus_ring(1.0, -1.0, accent().alpha(0.65))
        .on_press(move |s: &mut State| s.active = pane)
        .on_drawn_layout_tx(move |s: &mut State, rect, tx| {
            s.pane_rects[pane] = rect;
            tx.live_only();
        })
        .on_key(viewport_key)
        .on_drag(move |s: &mut State, drag, tx| { s.active = pane; viewport_drag(s, drag, tx); })
        .on_wheel(move |s: &mut State, event, tx| {
            s.active = pane;
            if s.count() == 0 { return false; }
            if event.phase == GesturePhase::Began { s.wheel_remainder = 0.0; }
            if event.dy != 0.0 {
                let steps = if event.precise {
                    s.wheel_remainder -= event.dy;
                    let steps = (s.wheel_remainder / 16.0).trunc() as i32;
                    s.wheel_remainder -= steps as f32 * 16.0;
                    steps
                } else {
                    (-event.dy / 16.0).abs().floor().max(1.0) as i32 * if event.dy < 0.0 { 1 } else { -1 }
                };
                if steps != 0 { s.step(steps); }
            }
            tx.instant(); true
        })
        .on_magnify(move |s: &mut State, event, tx| {
            s.active = pane;
            if s.count() == 0 { return false; }
            s.zoom = (s.zoom * (1.0 + event.delta).max(0.1)).clamp(0.1, 12.0);
            tx.instant(); true
        });
    if p.frame.is_some() && p.error.is_none() {
        canvas = canvas.child(
            el().key("image")
                .w_full()
                .h_full()
                .effect(1 + pane as u32)
                .scale(p.zoom)
                .offset(p.pan.0, p.pan.1)
                .a11y_hidden(),
        );
        let name = p
            .selected
            .map(|index| series_title(s, index))
            .unwrap_or_default();
        canvas = canvas.child(
            col()
                .key("image-info")
                .absolute(
                    if s.split { 8.0 } else { 24.0 },
                    if s.split { 86.0 } else { 23.0 },
                )
                .padding(8.0)
                .gap(7.0)
                .child(floating_label(name, 12.0, 600.0, foreground()))
                .child(floating_label("", 9.0, 550.0, muted())),
        );
        canvas = canvas.child(
            row()
                .key("image-metrics")
                .pin(Some(16.0), None, None, Some(78.0))
                .padding(8.0)
                .center_y()
                .gap(12.0)
                .child(floating_label(
                    format!("W {:.0}   L {:.0}", p.window, p.level),
                    11.0,
                    500.0,
                    muted(),
                ))
                .child(floating_label(
                    format!("{:.0}%", p.zoom * 100.0),
                    11.0,
                    500.0,
                    muted(),
                )),
        );
        canvas = canvas.child(
            row()
                .key("image-index")
                .pin(None, None, Some(16.0), Some(78.0))
                .padding(8.0)
                .child(floating_label(
                    format!("{:03} / {:03}", p.displayed_index + 1, count),
                    12.0,
                    600.0,
                    foreground(),
                )),
        );
    } else if let Some(error) = &p.error {
        canvas = canvas.child(
            col()
                .key("image-error")
                .w_full()
                .h_full()
                .center()
                .gap(16.0)
                .padding(40.0)
                .role("alert")
                .child(label(
                    "Unable to display this image",
                    23.0,
                    550.0,
                    foreground(),
                ))
                .child(
                    label(if s.privacy { "Error details hidden by Privacy mode. Turn it off to inspect the original message." } else { error }, 12.0, 400.0, ink("#F0BF76"))
                        .max_w(600.0)
                        .wrap(),
                )
                .child(open_button("open-folder-error", false)),
        );
    } else if pending || count > 0 {
        canvas = canvas.child(
            col()
                .key("image-loading")
                .w_full()
                .h_full()
                .center()
                .gap(15.0)
                .fill(ink("#050607"))
                .role("status")
                .children(
                    s.scan
                        .is_pending()
                        .then(|| label("Indexing your folder…", 15.0, 500.0, foreground())),
                ),
        );
    } else if s.split {
        canvas = canvas.child(
            col()
                .w_full()
                .h_full()
                .center()
                .gap(12.0)
                .child(label("Drag a series here", 18.0, 500.0, foreground()))
                .child(label(
                    "Or click this pane, then choose a series",
                    11.0,
                    400.0,
                    muted(),
                )),
        );
    } else {
        canvas = canvas.child(empty_state());
    }
    if pending {
        canvas = canvas.child(
            el().key("loading-progress")
                .pin(Some(0.0), Some(0.0), Some(0.0), None)
                .h(2.0)
                .fill(accent().alpha(0.12))
                .clip_rounded(0.0)
                .role("progressbar")
                .label(if s.scan.is_pending() {
                    "Indexing folder"
                } else {
                    "Loading image"
                })
                .child(
                    el().key("loading-segment")
                        .w_frac(0.28)
                        .h_full()
                        .fill(accent())
                        .looping(rune::LoopSpec::slide(1.1, [-1.0, 0.0], [3.6, 0.0]))
                        .a11y_hidden(),
                ),
        );
    }
    if p.contrast.is_pending() || p.contrast_error.is_some() {
        canvas = canvas.child(
            label(
                if p.contrast.is_pending() {
                    "Adjusting contrast…"
                } else {
                    "Auto contrast unavailable"
                },
                10.0,
                500.0,
                accent(),
            )
            .key("contrast-status")
            .absolute(16.0, if s.split { 142.0 } else { 90.0 })
            .max_w(240.0)
            .truncate()
            .role("status")
            .help(if s.privacy {
                "Contrast adjustment details hidden by Privacy mode."
            } else {
                p.contrast_error
                    .as_deref()
                    .unwrap_or("Sampling up to five representative slices in the background.")
            }),
        );
    }
    if s.hovered_drop == Some(pane) {
        canvas = canvas.child(glass::surface(
            el().key("series-drop-target")
                .pin(Some(12.0), Some(12.0), Some(12.0), Some(76.0))
                .radius(12.0)
                .border(1.0, accent().alpha(0.7))
                .a11y_hidden(),
            glass::DROP,
            0.025,
        ));
    }
    canvas
}

fn viewport_key(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.mods.cmd || event.mods.ctrl || event.mods.alt {
        return false;
    }
    match &event.key {
        Key::Up | Key::Left if s.count() > 0 => s.step(-1),
        Key::Down | Key::Right if s.count() > 0 => s.step(1),
        Key::Home if s.count() > 0 => s.step(-(s.index as i32)),
        Key::End if s.count() > 0 => s.step(s.count() as i32 - 1 - s.index as i32),
        Key::Text(key) => match key.to_ascii_lowercase().as_str() {
            "w" => s.tool = Tool::Window,
            "p" => s.tool = Tool::Pan,
            "z" => s.tool = Tool::Zoom,
            "a" if s.frame.is_some() => {
                if !tx.key_repeat() {
                    s.request_auto(s.active, true);
                }
            }
            "f" if s.count() > 0 => s.reset_geometry(),
            "r" if s.count() > 0 => {
                if !tx.key_repeat() {
                    reset(s);
                }
            }
            "i" if s.count() > 0 => {
                if !tx.key_repeat() {
                    s.toggle_invert();
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    tx.instant();
    true
}

fn viewport_drag(s: &mut State, drag: &mut Drag, tx: &mut Tx) {
    if s.count() == 0 {
        return;
    }
    if drag.phase == Phase::Down {
        s.drag_origin = (s.window, s.level, s.zoom, s.pan.0, s.pan.1);
        return;
    }
    let (window, level, zoom, x, y) = s.drag_origin;
    if drag.cancelled() {
        s.window = window;
        s.level = level;
        s.zoom = zoom;
        s.pan = (x, y);
        if s.tool == Tool::Window {
            s.apply_window();
        }
    } else {
        match s.tool {
            Tool::Window => {
                s.window = (window + drag.dx * window.max(100.0) / 240.0).max(1.0);
                s.level = level - drag.dy * window.max(100.0) / 240.0;
                s.apply_window();
            }
            Tool::Pan => s.pan = (x + drag.dx, y + drag.dy),
            Tool::Zoom => s.zoom = (zoom * (-drag.dy / 200.0).exp()).clamp(0.1, 12.0),
        }
    }
    tx.instant();
}

#[cfg(test)]
#[path = "../tests/unit/ui.rs"]
mod tests;
