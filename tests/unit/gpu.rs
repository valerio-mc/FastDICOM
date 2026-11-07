#[cfg(target_os = "macos")]
#[test]
fn two_gpu_images_have_independent_auto_uniforms_and_resident_uploads() {
    use rune::compose::{Compositor, Frame};
    use rune::render::{Gpu, Offscreen, Renderer};
    use rune::{Cx, El, Env, Runtime, el};
    use std::sync::Arc;
    fn view(_: &crate::State, _: &mut Cx<crate::State>) -> El {
        el().size(128.0, 64.0)
            .fill(rune::Color::BLACK)
            .child(el().absolute(0.0, 0.0).size(64.0, 64.0).effect(1))
            .child(el().absolute(64.0, 0.0).size(64.0, 64.0).effect(2))
    }
    let scalar = |value| {
        Arc::new(crate::dicom::Frame {
            width: 1,
            height: 1,
            values: vec![value],
            window: 100.0,
            level: 50.0,
            supplied_window: true,
            contrast_samples: vec![value],
            inverted: false,
            pixel_aspect: 1.0,
        })
    };
    let mut state = crate::State {
        split: true,
        ..crate::State::default()
    };
    state.panes[0].frame = Some(scalar(0.0));
    state.panes[1].frame = Some(scalar(100.0));
    for p in &mut state.panes {
        p.window = 100.0;
        p.level = 50.0;
    }
    let gpu = Gpu::new(Gpu::instance(), None);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = Offscreen::with_format(&gpu.device, 128, 64, format);
    let mut ui = Renderer::new(&gpu.device, format);
    let mut rt = Runtime::new(state, view, (128.0, 64.0), Env::new(true, 1.0));
    rt.track_marks = true;
    let mut compositor = super::ImagingCompositor::default();
    for stage in 0..4 {
        if stage == 1 {
            rt.update(|s, _| {
                s.active = 1;
                s.request_auto(1, true);
            });
        }
        if stage == 2 {
            rt.update(|s, _| {
                s.panes[0].frame = Some(scalar(100.0));
            });
        }
        rt.frame(0.0);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        compositor.render(&mut Frame {
            device: &gpu.device,
            queue: &gpu.queue,
            encoder: &mut encoder,
            target: &target.view,
            format,
            size: (128, 64),
            dt: 0.0,
            update_ms: 0.0,
            rt: &mut rt,
            ui: &mut ui,
        });
        target.copy(&mut encoder);
        ui.submit(&gpu.queue, encoder);
        let image = target.read(&gpu.device);
        let red = |x: usize| image[(32 * 128 + x) * 4];
        assert_eq!(
            red(32),
            if stage >= 2 { 255 } else { 0 },
            "stage {stage} left"
        );
        assert_eq!(
            red(96),
            if stage == 0 { 255 } else { 0 },
            "stage {stage} right"
        );
        assert_eq!(
            compositor.gpu.as_ref().unwrap().uploads,
            [if stage >= 2 { 2 } else { 1 }, 1]
        );
        assert!(image.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }
}

/// Exercise the real compositor's no-image path, not just rune-fx in isolation. Frost must
/// sample the already-painted black/white boundary, and must not make the window translucent.
#[cfg(target_os = "macos")]
#[test]
fn glass_samples_and_frosts_app_content_without_a_loaded_image() {
    use rune::compose::{Compositor, Frame};
    use rune::render::{Gpu, Offscreen, Renderer};
    use rune::{Color, Cx, El, Env, Runtime, el};

    fn view(state: &crate::State, _: &mut Cx<crate::State>) -> El {
        let mut root = el()
            .size(160.0, 96.0)
            .fill(Color::BLACK)
            .child(el().absolute(80.0, 0.0).size(80.0, 96.0).fill(Color::WHITE));
        if state.demo {
            root = root.child(crate::glass::surface(
                el().absolute(16.0, 8.0).size(128.0, 80.0).radius(12.0),
                crate::glass::DOCK,
                0.0,
            ));
        }
        root
    }

    let gpu = Gpu::new(Gpu::instance(), None);
    for srgb in [false, true] {
        let format = if srgb {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };
        let target = Offscreen::with_format(&gpu.device, 160, 96, format);
        let mut ui = Renderer::new(&gpu.device, format);
        ui.set_srgb_blend(srgb);
        let mut runtime = Runtime::new(
            crate::State::default(),
            view,
            (160.0, 96.0),
            Env {
                scale: 1.0,
                system_dark: true,
            },
        );
        runtime.set_srgb_blend(srgb);
        runtime.track_marks = true;
        let mut compositor = super::ImagingCompositor::default();
        let mut images = Vec::new();
        for has_glass in [false, true] {
            runtime.update(|state, tx| {
                state.demo = has_glass;
                tx.instant();
            });
            runtime.frame(0.0);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            compositor.render(&mut Frame {
                device: &gpu.device,
                queue: &gpu.queue,
                encoder: &mut encoder,
                target: &target.view,
                format,
                size: (160, 96),
                dt: 0.0,
                update_ms: 0.0,
                rt: &mut runtime,
                ui: &mut ui,
            });
            target.copy(&mut encoder);
            ui.submit(&gpu.queue, encoder);
            images.push(target.read(&gpu.device));
            assert!(
                !compositor.animating(),
                "static lenses must let the host sleep"
            );
        }
        let red = |image: &[u8], x: usize| image[(48 * 160 + x) * 4];
        assert_eq!(red(&images[0], 78), 0);
        assert!(
            red(&images[1], 78) > 12,
            "frost must mix white into the black side, srgb={srgb}"
        );
        assert!(
            red(&images[1], 108) > 200,
            "glass must retain the white backdrop, srgb={srgb}"
        );
        assert!(
            images[1]
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255),
            "glass must not punch through the opaque app"
        );
    }
}

#[test]
fn fit_preserves_anisotropic_pixel_proportions() {
    let mut frame = crate::synthetic_frame(0);
    frame.pixel_aspect = 2.0;
    assert_eq!(
        super::fitted_size(&frame, 800.0, 800.0, 0.0),
        (400.0, 800.0)
    );
    frame.pixel_aspect = 0.5;
    assert_eq!(
        super::fitted_size(&frame, 800.0, 800.0, 0.0),
        (800.0, 400.0)
    );
}

#[test]
fn rotated_fit_keeps_landscape_portrait_and_anisotropic_images_visible() {
    let mut frame = crate::synthetic_frame(0);
    // Each expected pair is the quad size BEFORE rotation. Its rotated bounds must fit
    // the entire 800x600 viewport, without also being constrained by an unrotated layer.
    for (columns, rows, aspect, expected_zero, expected_quarter) in [
        (1600, 800, 1.0, (800.0, 400.0), (600.0, 300.0)),
        (800, 1600, 1.0, (300.0, 600.0), (400.0, 800.0)),
        (800, 800, 2.0, (300.0, 600.0), (400.0, 800.0)),
    ] {
        frame.width = columns;
        frame.height = rows;
        frame.pixel_aspect = aspect;
        for (rotation, expected) in [
            (0.0, expected_zero),
            (std::f32::consts::FRAC_PI_2, expected_quarter),
        ] {
            let actual = super::fitted_size(&frame, 800.0, 600.0, rotation);
            assert!(
                (actual.0 - expected.0).abs() < 0.001,
                "{actual:?} vs {expected:?}"
            );
            assert!(
                (actual.1 - expected.1).abs() < 0.001,
                "{actual:?} vs {expected:?}"
            );
            let (sin, cos) = rotation.sin_cos();
            let bounds = (
                cos.abs() * actual.0 + sin.abs() * actual.1,
                sin.abs() * actual.0 + cos.abs() * actual.1,
            );
            assert!(
                bounds.0 <= 800.001 && bounds.1 <= 600.001,
                "clipped: {bounds:?}"
            );
        }
    }
}

#[test]
fn scalar_shader_validates_with_rune_clipping() {
    let source = format!(
        "{}\n{}",
        rune::render::CLIP_WGSL,
        include_str!("../../src/image.wgsl")
    );
    let module = wgpu::naga::front::wgsl::parse_str(&source).expect("valid image WGSL");
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        // Rune's packed clip edges use unpack2x16float, not f16 shader arithmetic.
        wgpu::naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
    )
    .validate(&module)
    .expect("valid scalar image shader with Rune packed clipping");
}
