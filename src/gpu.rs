//! Scalar image rendering inside Rune's painter stream, including its transformed layers.
use std::{collections::HashMap, sync::Arc};

use rune::{
    compose::{Compositor, DeviceKey, Frame},
    render::{CLIP_WGSL, EffectPhase, EffectSite},
};

use crate::{State, dicom, glass::GlassRenderer};

const UNIFORM_BYTES: u64 = 128;

#[derive(Default)]
pub(super) struct ImagingCompositor {
    gpu: Option<Gpu>,
    error_redraw: bool,
}

struct Gpu {
    key: DeviceKey,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    images: [Option<Image>; 2],
    slots: HashMap<(usize, u32), Slot>,
    glass: GlassRenderer,
    #[cfg(test)]
    uploads: [usize; 2],
}

struct Image {
    // Holding the Arc makes pointer identity reliable even after the app replaces its frame.
    frame: Arc<dicom::Frame>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Slot {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Compositor<State> for ImagingCompositor {
    fn animating(&self) -> bool {
        self.error_redraw
    }

    fn render(&mut self, f: &mut Frame<'_, State>) {
        self.error_redraw = false;
        let limit = f.device.limits().max_texture_dimension_2d;
        let key = f.device_key();
        if self.gpu.as_ref().is_none_or(|gpu| gpu.key != key) {
            self.gpu = Some(Gpu::new(f.device, key));
        }
        let gpu = self.gpu.as_mut().expect("GPU initialized above");
        for pane in 0..2 {
            let mut frame = f.rt.state.panes[pane].frame.clone();
            if let Some(oversize) = frame
                .as_ref()
                .filter(|frame| frame.width > limit || frame.height > limit)
            {
                let message = format!(
                    "Image {}×{} exceeds this GPU's {limit}-pixel maximum side",
                    oversize.width, oversize.height
                );
                f.rt.update(move |state, _| {
                    state.panes[pane].error = Some(message);
                    state.panes[pane].frame = None;
                });
                // Runtime::update commits on the next frame. Keep the host awake for that one
                // frame so the error UI appears even when the user has stopped interacting.
                self.error_redraw = true;
                frame = None;
            }
            if let Some(frame) = frame {
                gpu.upload(f.device, f.queue, pane, frame);
            } else {
                gpu.images[pane] = None;
                gpu.slots.retain(|(owner, _), _| *owner != pane);
            }
        }
        gpu.glass.prepare(f);
        let presentation =
            f.rt.state
                .panes
                .each_ref()
                .map(|p| (p.window, p.level, p.invert, p.rotation, p.horizontal_flip));
        let clear = f.rt.clear_color();
        f.ui.prepare(
            f.device,
            f.queue,
            &mut f.rt.text.atlas,
            &f.rt.instances,
            f.size,
        );
        let mut seen = Vec::new();
        f.ui.draw_with_backdrop(f.encoder, f.target, clear, &mut |phase, site| {
            if !(1..=2).contains(&site.mark) {
                gpu.glass.draw(f.device, f.queue, phase, site);
                return;
            }
            let pane = (site.mark - 1) as usize;
            let (window, level, invert, rotation, horizontal_flip) = presentation[pane];
            let EffectPhase::Draw(pass) = phase else {
                return;
            };
            if gpu.images[pane].is_none() {
                return;
            }
            let [_, _, width, height] = site.instance.rect;
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            gpu.ensure_pipeline(f.device, site.format);
            let image = gpu.images[pane].as_ref().expect("image uploaded above");
            let slot = gpu.slots.entry((pane, site.index)).or_insert_with(|| {
                let uniform = f.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("image slot uniforms"),
                    size: UNIFORM_BYTES,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bind_group = f.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("image slot"),
                    layout: &gpu.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&image.view),
                        },
                    ],
                });
                Slot {
                    uniform,
                    bind_group,
                }
            });
            let bytes = uniforms(
                site,
                &image.frame,
                window,
                level,
                invert,
                rotation,
                horizontal_flip,
            );
            f.queue.write_buffer(&slot.uniform, 0, &bytes);
            pass.set_pipeline(&gpu.pipelines[&site.format]);
            pass.set_bind_group(0, &slot.bind_group, &[]);
            pass.draw(0..6, 0..1);
            seen.push((pane, site.index));
        });
        gpu.slots.retain(|index, _| seen.contains(index));
    }
}

impl Gpu {
    fn new(device: &wgpu::Device, key: DeviceKey) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scalar image bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scalar image pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("DICOM LINEAR window shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{CLIP_WGSL}\n{}", include_str!("image.wgsl")).into(),
            ),
        });
        Self {
            key,
            layout,
            pipeline_layout,
            shader,
            pipelines: HashMap::new(),
            images: [None, None],
            slots: HashMap::new(),
            glass: GlassRenderer::default(),
            #[cfg(test)]
            uploads: [0; 2],
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pane: usize,
        frame: Arc<dicom::Frame>,
    ) {
        if self.images[pane]
            .as_ref()
            .is_some_and(|image| Arc::ptr_eq(&image.frame, &frame))
        {
            return;
        }
        let size = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        };
        let reuse = self.images[pane]
            .as_ref()
            .is_some_and(|image| image.texture.size() == size);
        if !reuse {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("modality-rescaled scalar slice"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.images[pane] = Some(Image {
                frame: frame.clone(),
                texture,
                view,
            });
            self.slots.retain(|(owner, _), _| *owner != pane);
        }
        let image = self.images[pane].as_mut().expect("texture created above");
        // SAFETY: f32 has no padding; the immutable byte view lives only for this copying upload.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                frame.values.as_ptr().cast::<u8>(),
                std::mem::size_of_val(frame.values.as_slice()),
            )
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &image.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            size,
        );
        image.frame = frame;
        #[cfg(test)]
        {
            self.uploads[pane] += 1;
        }
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scalar image pipeline"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
    }
}

fn uniforms(
    site: &EffectSite<'_>,
    frame: &dicom::Frame,
    window: f32,
    level: f32,
    invert: bool,
    rotation: f32,
    horizontal_flip: bool,
) -> [u8; UNIFORM_BYTES as usize] {
    let [x, y, width, height] = site.instance.rect;
    let (w, h) = fitted_size(frame, width, height, rotation);
    // Scale/pan are in rect already. Rotate only the image quad, not the viewport layer:
    // otherwise a portrait/landscape image can be clipped before or after layer rotation.
    let words = [
        [x + (width - w) * 0.5, y + (height - h) * 0.5, w, h],
        [
            site.origin[0],
            site.origin[1],
            site.size[0] as f32,
            site.size[1] as f32,
        ],
        [frame.width as f32, frame.height as f32, window, level],
        [
            if frame.inverted ^ invert { 1.0 } else { 0.0 },
            if site.srgb_blend { 1.0 } else { 0.0 },
            if horizontal_flip { 1.0 } else { 0.0 },
            rotation,
        ],
        site.instance.clip,
        site.instance.fade.map(f32::from_bits),
        site.instance.rect,
        site.instance.radii,
    ];
    let mut bytes = [0; UNIFORM_BYTES as usize];
    for (value, dst) in words
        .into_iter()
        .flatten()
        .zip(bytes.as_chunks_mut::<4>().0.iter_mut())
    {
        dst.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn fitted_size(frame: &dicom::Frame, width: f32, height: f32, rotation: f32) -> (f32, f32) {
    let physical_width = frame.width as f32;
    let physical_height = frame.height as f32 * frame.pixel_aspect;
    let (sin, cos) = rotation.sin_cos();
    let rotated_width = cos.abs() * physical_width + sin.abs() * physical_height;
    let rotated_height = sin.abs() * physical_width + cos.abs() * physical_height;
    let fit = (width / rotated_width).min(height / rotated_height);
    (physical_width * fit, physical_height * fit)
}

#[cfg(test)]
#[path = "../tests/unit/gpu.rs"]
mod tests;
