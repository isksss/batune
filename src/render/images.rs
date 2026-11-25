//! Image quads: one texture per image, drawn under or over the text, and
//! the background image under everything.

use std::borrow::Cow;
use std::collections::HashMap;
use std::mem::size_of;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};

use crate::render::background::{self, BackgroundFit, BackgroundImage};
use crate::render::frame::{Frame, ImageData};
use crate::render::renderer::InstanceBuffer;

/// Textures not used by the current frame are kept up to this many bytes,
/// so scrolling back to an image does not upload it again.
const CACHE_BYTES: usize = 128 << 20;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageInstance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 4],
}

/// The background image with the color it is mixed into: `color` is the
/// frame background, its alpha the image's opacity.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BackgroundInstance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 4],
    color: [u8; 4],
}

struct Texture {
    bind_group: wgpu::BindGroup,
    bytes: usize,
    last_used: u64,
}

/// The background image's texture. Kept apart from the cache so that
/// eviction never drops it.
struct Background {
    bind_group: wgpu::BindGroup,
    /// Size of the source image, not of the texture: the aspect ratio of a
    /// texture shrunk to the GPU limit is off by rounding.
    size: (f32, f32),
    fit: BackgroundFit,
    opacity: f32,
}

/// A run of instances using one texture.
struct Draw {
    id: u64,
    instances: Range<u32>,
    above_text: bool,
}

pub(crate) struct ImageLayer {
    pipeline: wgpu::RenderPipeline,
    background_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<u64, Texture>,
    background: Option<Background>,
    /// Holds one [`BackgroundInstance`], rewritten every frame.
    background_buffer: wgpu::Buffer,
    frame_no: u64,
    buffer: InstanceBuffer,
    instances: Vec<ImageInstance>,
    draws: Vec<Draw>,
}

impl ImageLayer {
    /// `globals` is the layout of bind group 0, shared with the other
    /// pipelines so the renderer sets it once per pass.
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        shader: &wgpu::ShaderModule,
        globals: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("batune-render image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("batune-render images"),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label, vs, fs, attributes: &[wgpu::VertexAttribute], stride| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let background_pipeline = pipeline(
            "batune-render background image",
            "vs_background",
            "fs_background",
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Unorm8x4],
            size_of::<BackgroundInstance>(),
        );
        let pipeline = pipeline(
            "batune-render images",
            "vs_image",
            "fs_image",
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
            size_of::<ImageInstance>(),
        );
        // Linear: unlike glyphs, images are scaled to the cell grid.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("batune-render image"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            background_pipeline,
            layout,
            sampler,
            textures: HashMap::new(),
            background: None,
            background_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("batune-render background instance"),
                size: size_of::<BackgroundInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            frame_no: 0,
            buffer: InstanceBuffer::new(device, "batune-render image instances"),
            instances: Vec::new(),
            draws: Vec::new(),
        }
    }

    /// Replaces the background image; `None` removes it.
    pub(crate) fn set_background(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: Option<&BackgroundImage>,
    ) {
        self.background = image.map(|image| {
            let (bind_group, _) =
                self.create_texture(device, queue, &image.rgba, image.width, image.height);
            Background {
                bind_group,
                size: (image.width as f32, image.height as f32),
                fit: image.fit,
                opacity: image.opacity.clamp(0.0, 1.0),
            }
        });
    }

    /// `origin` is the top-left of the grid and `cell` the cell size, and
    /// `surface` the size of the target, in physical pixels.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        origin: f32,
        cell: (f32, f32),
        surface: (f32, f32),
    ) {
        self.frame_no += 1;
        self.instances.clear();
        self.draws.clear();
        if let Some(b) = &self.background {
            let (pos, size, uv) = background::place(b.fit, b.size, surface);
            let bg = frame.background;
            let instance = BackgroundInstance {
                pos,
                size,
                uv,
                color: [bg.r, bg.g, bg.b, (b.opacity * 255.0).round() as u8],
            };
            queue.write_buffer(&self.background_buffer, 0, bytemuck::bytes_of(&instance));
        }
        let mut quads: Vec<_> = frame.images.iter().collect();
        // Stable: quads of one image and layer stay in the caller's order.
        quads.sort_by_key(|q| (q.z, q.image.id));
        for q in quads {
            self.upload(device, queue, &q.image);
            let n = self.instances.len() as u32;
            self.instances.push(ImageInstance {
                pos: [
                    origin + (f32::from(q.col) + q.offset[0]) * cell.0,
                    origin + (f32::from(q.row) + q.offset[1]) * cell.1,
                ],
                size: [q.width * cell.0, q.height * cell.1],
                uv: q.uv,
            });
            let above_text = q.z >= 0;
            match self.draws.last_mut() {
                Some(d) if d.id == q.image.id && d.above_text == above_text => {
                    d.instances.end = n + 1;
                }
                _ => self.draws.push(Draw {
                    id: q.image.id,
                    instances: n..n + 1,
                    above_text,
                }),
            }
        }
        self.evict();
        self.buffer
            .write(device, queue, bytemuck::cast_slice(&self.instances));
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &ImageData) {
        if let Some(t) = self.textures.get_mut(&image.id) {
            t.last_used = self.frame_no;
            return;
        }
        let (bind_group, bytes) =
            self.create_texture(device, queue, &image.rgba, image.width, image.height);
        self.textures.insert(
            image.id,
            Texture {
                bind_group,
                bytes,
                last_used: self.frame_no,
            },
        );
    }

    /// Uploads straight-alpha RGBA pixels; returns the bind group and the
    /// bytes the texture takes.
    fn create_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> (wgpu::BindGroup, usize) {
        let max = device.limits().max_texture_dimension_2d;
        let (rgba, width, height) = fit(rgba, width, height, max);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("batune-render image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Not sRGB, like the glyph atlas: the shader linearizes only
            // when the target needs it.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            size,
        );
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("batune-render image"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        (bind_group, rgba.len())
    }

    /// Drops the least recently used textures not in this frame while the
    /// cache is over [`CACHE_BYTES`].
    fn evict(&mut self) {
        let mut total: usize = self.textures.values().map(|t| t.bytes).sum();
        if total <= CACHE_BYTES {
            return;
        }
        let mut old: Vec<_> = self
            .textures
            .iter()
            .filter(|(_, t)| t.last_used != self.frame_no)
            .map(|(&id, t)| (t.last_used, id))
            .collect();
        old.sort_unstable();
        for (_, id) in old {
            if total <= CACHE_BYTES {
                break;
            }
            if let Some(t) = self.textures.remove(&id) {
                total -= t.bytes;
            }
        }
    }

    /// Records the background image's draw; bind group 0 must already be set.
    pub(crate) fn draw_background(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(b) = &self.background else {
            return;
        };
        pass.set_pipeline(&self.background_pipeline);
        pass.set_vertex_buffer(0, self.background_buffer.slice(..));
        pass.set_bind_group(1, &b.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }

    /// Records the draws of one layer; bind group 0 must already be set.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, above_text: bool) {
        let mut draws = self
            .draws
            .iter()
            .filter(|d| d.above_text == above_text)
            .peekable();
        if draws.peek().is_none() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.buffer.buffer.slice(..));
        for d in draws {
            if let Some(t) = self.textures.get(&d.id) {
                pass.set_bind_group(1, &t.bind_group, &[]);
                pass.draw(0..4, d.instances.clone());
            }
        }
    }
}

/// Scales an image down (nearest neighbor) when a side exceeds the GPU's
/// texture limit. Not resampled properly: such images are far larger than
/// any cell area they could be shown in, so the detail lost is invisible.
fn fit(rgba: &[u8], width: u32, height: u32, max: u32) -> (Cow<'_, [u8]>, u32, u32) {
    if width <= max && height <= max {
        return (Cow::Borrowed(rgba), width, height);
    }
    let scale = f64::from(max) / f64::from(width.max(height));
    let w = ((f64::from(width) * scale) as u32).clamp(1, max);
    let h = ((f64::from(height) * scale) as u32).clamp(1, max);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let sy = (u64::from(y) * u64::from(height) / u64::from(h)) as usize;
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(width) / u64::from(w)) as usize;
            let i = (sy * width as usize + sx) * 4;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    (Cow::Owned(out), w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_are_used_as_is() {
        let px = [1u8; 2 * 3 * 4];
        let (out, w, h) = fit(&px, 2, 3, 4);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!((w, h), (2, 3));
    }

    #[test]
    fn images_over_the_texture_limit_shrink_keeping_the_aspect_ratio() {
        // 4x2: left half red, right half blue.
        let mut px = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                px.extend_from_slice(if x < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        let (out, w, h) = fit(&px, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(&out[..], &[255, 0, 0, 255, 0, 0, 255, 255]);
    }
}
