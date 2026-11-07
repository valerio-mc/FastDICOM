//! In-app refractive glass. The native window remains opaque: only Rune's painter-stream
//! backdrop is sampled, never the desktop or other applications.
use rune::{
    Color, El,
    compose::Frame,
    render::{EffectPhase, EffectSite, KIND_EFFECT},
};
use rune_fx::{Fx, FxFrame, FxOptions, FxRenderer, FxSlot, LensParams, Shape, rgb};

pub const DOCK: u32 = 10;
pub const PATIENT: u32 = 11;
pub const SELECTED_ROW: u32 = 12;
pub const HOVER_ROW: u32 = 13;
pub const DROP: u32 = 14;

/// Reserve a refractive surface before its children, retaining the element's key, layout,
/// interaction handlers and corner geometry. Tint is a subtle material tint, not a cover fill.
pub fn surface(el: El, mark: u32, tint: f32) -> El {
    el.fill(Color::TRANSPARENT)
        .effect(mark)
        .effect_backdrop(true)
        .effect_data([tint.clamp(0.0, 1.0), 0.0, 0.0, 0.0])
        .effect_bleed(16.0)
}

fn is_glass(mark: u32) -> bool {
    matches!(mark, DOCK | PATIENT | SELECTED_ROW | HOVER_ROW | DROP)
}

fn lens(tint: f32) -> LensParams {
    LensParams {
        bevel: 12.0,
        depth: 18.0,
        dispersion: 0.0,
        magnification: 1.03,
        frost: 0.18,
        shadow: 0.3,
        rim: 1.4,
        light_angle: -2.09,
        tint: rgb(0xD9FF67),
        tint_amount: tint.clamp(0.0, 1.0),
        ..Default::default()
    }
}

/// Owned by the image compositor's device-scoped resources. Presented instance indices
/// distinguish surfaces with the same role; the slot list is rebuilt from visible instances.
#[derive(Default)]
pub(super) struct GlassRenderer {
    renderer: Option<FxRenderer>,
    indices: Vec<u32>,
}

impl GlassRenderer {
    pub(super) fn prepare<S: 'static>(&mut self, f: &Frame<'_, S>) {
        self.indices.clear();
        let mut layers = Vec::with_capacity(4);
        let mut slots = Vec::with_capacity(4);
        for (index, instance) in f.rt.instances.iter().enumerate() {
            if instance.params[1] != KIND_EFFECT {
                continue;
            }
            let mark = f.rt.marks.get(index).copied().unwrap_or(0);
            if !is_glass(mark) {
                continue;
            }
            let scale = instance.params[3];
            if !scale.is_finite() || scale <= 0.0 {
                continue;
            }
            let [x, y, width, height] = instance.rect.map(|v| v / scale);
            if width <= 0.0 || height <= 0.0 {
                continue;
            }
            // These surfaces use uniform circular corners; preserve their presented radius
            // and per-slot scale instead of assuming the window's DPI or unanimated layout.
            let radius = instance.radii.iter().fold(0.0_f32, |r, v| r.max(v.abs())) / scale;
            layers.push(
                Fx::lens(lens(instance.uv[0]))
                    .on(Shape::rounded(x, y, width, height, radius))
                    .at(f.rt.t as f32),
            );
            slots.push(
                FxSlot::default()
                    .scale(scale)
                    .clip(instance.clip, instance.fade),
            );
            self.indices.push(index as u32);
        }
        if layers.is_empty() {
            // Keep expensive pipelines for the next hover, without preparing or blurring an
            // empty frame. FxRenderer reuses one resize-aware, half-resolution blur pyramid.
            return;
        }
        let renderer = self
            .renderer
            .get_or_insert_with(|| FxRenderer::new(f.device, f.format, FxOptions { dither: true }));
        renderer.set_srgb_blend(f.srgb_blend());
        renderer.add_format(f.device, f.format);
        renderer.add_format(f.device, wgpu::TextureFormat::Rgba16Float);
        renderer.prepare_with_slots(
            f.device,
            f.queue,
            &FxFrame {
                viewport: [f.size.0, f.size.1],
                scale: 1.0,
                layers: &layers,
                backdrop: None,
                sources: &[],
            },
            &slots,
        );
    }

    pub(super) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        phase: EffectPhase<'_, '_>,
        site: &EffectSite<'_>,
    ) {
        let Some(layer) = self.indices.iter().position(|index| *index == site.index) else {
            return;
        };
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match phase {
            EffectPhase::Backdrop(encoder) => {
                // Rune closes the pass and copies exactly the content beneath this slot.
                // Frost runs here, not while the target is being used as a render attachment.
                if let Some(backdrop) = site.backdrop {
                    renderer.blur_backdrop(device, queue, encoder, backdrop, layer);
                }
            }
            EffectPhase::Draw(pass) => {
                renderer.draw_layer_in(pass, site.format, site.origin, layer);
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/glass.rs"]
mod tests;
