//! wgpu pipelines that turn a [`Frame`] into instanced quads.
//!
//! Draw order: the background image, cell backgrounds, images under text,
//! glyphs, images over text, then decorations and the cursor. Each layer is one instanced draw
//! call (images: one per texture), so the cost per frame is a buffer upload
//! proportional to the number of non-empty cells.

use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::render::background::BackgroundImage;
use crate::render::decorations;
use crate::render::frame::{CellFlags, Color, CursorShape, Frame};
use crate::render::glyphs::{AtlasFull, FontConfig, Fonts, GlyphCache, GlyphKey};
use crate::render::images::ImageLayer;
use crate::render::metrics::CellMetrics;

const ATLAS_WIDTH: u32 = 1024;
const ATLAS_INITIAL_HEIGHT: u32 = 512;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    srgb: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RectInstance {
    pos: [f32; 2],
    size: [f32; 2],
    color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphInstance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
    kind: u32,
}

fn rgba(c: Color) -> [u8; 4] {
    [c.r, c.g, c.b, 0xff]
}

/// A vertex buffer that grows to the largest frame seen so far.
pub(crate) struct InstanceBuffer {
    label: &'static str,
    pub(crate) buffer: wgpu::Buffer,
}

impl InstanceBuffer {
    pub(crate) fn new(device: &wgpu::Device, label: &'static str) -> Self {
        Self {
            label,
            buffer: Self::alloc(device, label, 4096),
        }
    }

    fn alloc(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub(crate) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        let len = data.len() as u64;
        if len > self.buffer.size() {
            self.buffer = Self::alloc(device, self.label, len.next_power_of_two());
        }
        if len > 0 {
            queue.write_buffer(&self.buffer, 0, data);
        }
    }
}

/// Draws [`Frame`]s with wgpu. The renderer does not own the surface: the
/// caller acquires a texture, then either calls [`Renderer::render`] or
/// embeds [`Renderer::prepare`] + [`Renderer::draw`] in its own pass.
pub struct Renderer {
    fonts: Fonts,
    font: FontConfig,
    cache: GlyphCache,
    scale_factor: f32,
    padding: f32,

    srgb: bool,
    rect_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    atlas: wgpu::Texture,

    rect_buffer: InstanceBuffer,
    glyph_buffer: InstanceBuffer,
    images: ImageLayer,
    rects: Vec<RectInstance>,
    glyphs: Vec<GlyphInstance>,
    /// `rects[..under]` go below the glyphs, the rest above.
    under: u32,
}

/// Returns a builder of pipelines that draw one triangle strip per instance
/// of `stride` bytes, blended over `format` with premultiplied alpha.
pub(crate) fn instanced_pipelines<'a>(
    device: &'a wgpu::Device,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> impl Fn(&str, &str, &str, &[wgpu::VertexAttribute], usize) -> wgpu::RenderPipeline + 'a {
    move |label, vs, fs, attributes, stride| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
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
    }
}

impl Renderer {
    /// `format` is the color format of the textures passed to `render` /
    /// used by the pass given to `draw`. Sizes in `font` are logical pixels.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        font: FontConfig,
        scale_factor: f64,
    ) -> Self {
        let scale_factor = scale_factor as f32;
        let mut fonts = Fonts::new(font.family.clone(), font.size * scale_factor);
        if (font.cell_width, font.line_height) != (1.0, 1.0) {
            fonts.set_spacing(font.cell_width, font.line_height);
            fonts.set_size(font.size * scale_factor);
        }

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("batune-render globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("batune-render"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = instanced_pipelines(device, &layout, &shader, format);
        let rect_pipeline = pipeline(
            "batune-render rects",
            "vs_rect",
            "fs_rect",
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4],
            size_of::<RectInstance>(),
        );
        let glyph_pipeline = pipeline(
            "batune-render glyphs",
            "vs_glyph",
            "fs_glyph",
            &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Unorm8x4, 4 => Uint32
            ],
            size_of::<GlyphInstance>(),
        );

        // Nearest: glyph quads are placed on whole pixels, so filtering
        // would only blur them.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("batune-render atlas"),
            ..Default::default()
        });
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("batune-render globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [1.0, 1.0],
                srgb: 0,
                _pad: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let images = ImageLayer::new(device, format, &shader, &bind_group_layout);
        let atlas = create_atlas(device, ATLAS_WIDTH, ATLAS_INITIAL_HEIGHT);
        let bind_group = create_bind_group(device, &bind_group_layout, &globals, &atlas, &sampler);

        Self {
            fonts,
            font,
            cache: GlyphCache::new(ATLAS_WIDTH, ATLAS_INITIAL_HEIGHT),
            scale_factor,
            padding: 0.0,
            srgb: format.is_srgb(),
            rect_pipeline,
            glyph_pipeline,
            bind_group_layout,
            bind_group,
            sampler,
            globals,
            atlas,
            rect_buffer: InstanceBuffer::new(device, "batune-render rect instances"),
            glyph_buffer: InstanceBuffer::new(device, "batune-render glyph instances"),
            images,
            rects: Vec::new(),
            glyphs: Vec::new(),
            under: 0,
        }
    }

    /// Cell size in physical pixels.
    pub fn cell_metrics(&self) -> CellMetrics {
        self.fonts.cell_metrics()
    }

    /// Space between the surface edge and the grid, in logical pixels.
    pub fn set_padding(&mut self, padding: f32) {
        self.padding = padding;
    }

    /// Padding in physical pixels.
    pub fn padding(&self) -> u32 {
        (self.padding * self.scale_factor).round() as u32
    }

    /// Columns and rows that fit in a surface of `width` x `height`
    /// physical pixels.
    pub fn grid_size(&self, width: u32, height: u32) -> (u16, u16) {
        self.cell_metrics().grid_size(width, height, self.padding())
    }

    /// Cell under a physical-pixel position, e.g. for mouse reporting.
    /// Positions in the padding clamp to the nearest cell.
    pub fn cell_at(&self, x: f64, y: f64, cols: u16, rows: u16) -> (u16, u16) {
        let m = self.cell_metrics();
        let pad = f64::from(self.padding());
        let pick = |v: f64, cell: u32, n: u16| {
            let i = ((v - pad) / f64::from(cell)).floor();
            i.clamp(0.0, f64::from(n.saturating_sub(1))) as u16
        };
        (pick(x, m.width, cols), pick(y, m.height, rows))
    }

    pub fn scale_factor(&self) -> f64 {
        f64::from(self.scale_factor)
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor as f32;
        self.reload_font();
    }

    pub fn set_font(&mut self, font: FontConfig) {
        if font.family != self.font.family {
            self.fonts = Fonts::new(font.family.clone(), font.size * self.scale_factor);
        }
        self.font = font;
        self.reload_font();
    }

    /// Replaces the image drawn behind the grid; `None` removes it.
    pub fn set_background_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: Option<&BackgroundImage>,
    ) {
        self.images.set_background(device, queue, image);
    }

    fn reload_font(&mut self) {
        self.fonts
            .set_spacing(self.font.cell_width, self.font.line_height);
        self.fonts.set_size(self.font.size * self.scale_factor);
        self.cache.clear();
    }

    /// Builds this frame's instances and uploads them along with any newly
    /// rasterized glyphs. `size` is the target size in physical pixels.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        size: (u32, u32),
    ) {
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [size.0.max(1) as f32, size.1.max(1) as f32],
                srgb: u32::from(self.srgb),
                _pad: 0,
            }),
        );

        // A frame can need more glyphs than the atlas holds even after
        // growing; then the cache is reset and the frame rebuilt once, so
        // only glyphs visible right now occupy the atlas.
        let mut retried = false;
        loop {
            match self.build(device, queue, frame) {
                Ok(()) => break,
                Err(AtlasFull) if !retried => {
                    retried = true;
                    self.cache.clear();
                }
                // The build stopped part way; `under` still counts the last
                // frame's rects, which would draw some of these above glyphs.
                Err(AtlasFull) => {
                    self.under = self.rects.len() as u32;
                    break;
                }
            }
        }
        self.flush_uploads(queue);
        self.rect_buffer
            .write(device, queue, bytemuck::cast_slice(&self.rects));
        self.glyph_buffer
            .write(device, queue, bytemuck::cast_slice(&self.glyphs));
        let m = self.cell_metrics();
        let cell = (m.width as f32, m.height as f32);
        let surface = (size.0.max(1) as f32, size.1.max(1) as f32);
        self.images
            .prepare(device, queue, frame, self.padding() as f32, cell, surface);
    }

    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
    ) -> Result<(), AtlasFull> {
        let m = self.cell_metrics();
        let pad = self.padding() as f32;
        let (cw, ch) = (m.width as f32, m.height as f32);
        let stroke = m.stroke as f32;
        let cursor_thickness = (2.0 * self.scale_factor).round().max(1.0);

        self.rects.clear();
        self.glyphs.clear();
        let mut over = Vec::new();

        for row in 0..frame.rows {
            for col in 0..frame.cols {
                let Some(cell) = frame.cell(col, row) else {
                    continue;
                };
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    continue;
                }
                let span = if cell.flags.contains(CellFlags::WIDE) && col + 1 < frame.cols {
                    2.0
                } else {
                    1.0
                };
                let x = pad + f32::from(col) * cw;
                let y = pad + f32::from(row) * ch;
                let w = cw * span;
                let (mut fg, mut bg) = cell.effective_colors();
                let cursor = frame.cursor.filter(|c| c.col == col && c.row == row);
                if let Some(c) = cursor.filter(|c| c.shape == CursorShape::Block) {
                    fg = bg;
                    bg = c.color;
                }
                if bg != frame.background {
                    self.rects.push(RectInstance {
                        pos: [x, y],
                        size: [w, ch],
                        color: rgba(bg),
                    });
                }
                let blank =
                    (cell.ch.is_whitespace() || cell.ch == '\0') && cell.zerowidth.is_none();
                if !cell.flags.contains(CellFlags::HIDDEN) && !blank {
                    let key = GlyphKey {
                        ch: cell.ch,
                        zerowidth: cell.zerowidth.clone(),
                        bold: cell.flags.contains(CellFlags::BOLD),
                        italic: cell.flags.contains(CellFlags::ITALIC),
                    };
                    // A wide glyph spans the extra space of both cells.
                    let gx = x + m.glyph_x as f32 * span;
                    self.push_glyph(device, queue, &key, gx, y, fg)?;
                }
                if let Some(style) = decorations::Style::of(cell.flags) {
                    let color = rgba(cell.underline_color.unwrap_or(fg));
                    let grid_x = u32::from(col) * m.width;
                    for [dx, dy, dw, dh] in decorations::underline(style, &m, grid_x, w as u32) {
                        over.push(RectInstance {
                            pos: [x + dx, y + dy],
                            size: [dw, dh],
                            color,
                        });
                    }
                }
                if cell.flags.contains(CellFlags::STRIKETHROUGH) {
                    over.push(RectInstance {
                        pos: [x, y + m.strikeout_y as f32],
                        size: [w, stroke],
                        color: rgba(fg),
                    });
                }
                if let Some(c) = cursor {
                    let t = cursor_thickness;
                    let color = rgba(c.color);
                    let mut rect = |pos: [f32; 2], size: [f32; 2]| {
                        over.push(RectInstance { pos, size, color })
                    };
                    match c.shape {
                        CursorShape::Block => {}
                        CursorShape::Underline => rect([x, y + ch - t], [w, t]),
                        CursorShape::Bar => rect([x, y], [t, ch]),
                        CursorShape::HollowBlock => {
                            rect([x, y], [w, t]);
                            rect([x, y + ch - t], [w, t]);
                            rect([x, y], [t, ch]);
                            rect([x + w - t, y], [t, ch]);
                        }
                    }
                }
            }
        }
        self.under = self.rects.len() as u32;
        self.rects.append(&mut over);
        Ok(())
    }

    fn push_glyph(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &GlyphKey,
        x: f32,
        y: f32,
        fg: Color,
    ) -> Result<(), AtlasFull> {
        let glyph = match self.cache.get(&mut self.fonts, key) {
            Ok(glyph) => glyph,
            Err(AtlasFull) => {
                if !self.grow_atlas(device, queue) {
                    return Err(AtlasFull);
                }
                self.cache.get(&mut self.fonts, key)?
            }
        };
        let baseline = self.cell_metrics().baseline as f32;
        for g in glyph.iter() {
            self.glyphs.push(GlyphInstance {
                pos: [x + g.left as f32, y + baseline - g.top as f32],
                size: [g.rect.w as f32, g.rect.h as f32],
                uv: [g.rect.x as f32, g.rect.y as f32],
                color: rgba(fg),
                kind: u32::from(g.color),
            });
        }
        Ok(())
    }

    /// Doubles the atlas height, keeping existing glyphs in place.
    fn grow_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let (width, height) = self.cache.size();
        let new_height = height * 2;
        if new_height > device.limits().max_texture_dimension_2d {
            return false;
        }
        // Pending uploads target the old texture; write them before copying.
        self.flush_uploads(queue);
        let atlas = create_atlas(device, width, new_height);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            self.atlas.as_image_copy(),
            atlas.as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        self.atlas = atlas;
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &self.globals,
            &self.atlas,
            &self.sampler,
        );
        self.cache.grow(new_height);
        true
    }

    fn flush_uploads(&mut self, queue: &wgpu::Queue) {
        for upload in self.cache.take_uploads() {
            let r = upload.rect;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: r.x,
                        y: r.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(r.w * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: r.w,
                    height: r.h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Records the draw calls for the last prepared frame.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let total = self.rects.len() as u32;
        pass.set_bind_group(0, &self.bind_group, &[]);
        self.images.draw_background(pass);
        if self.under > 0 {
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_vertex_buffer(0, self.rect_buffer.buffer.slice(..));
            pass.draw(0..4, 0..self.under);
        }
        self.images.draw(pass, false);
        if !self.glyphs.is_empty() {
            pass.set_pipeline(&self.glyph_pipeline);
            pass.set_vertex_buffer(0, self.glyph_buffer.buffer.slice(..));
            pass.draw(0..4, 0..self.glyphs.len() as u32);
        }
        self.images.draw(pass, true);
        if total > self.under {
            pass.set_pipeline(&self.rect_pipeline);
            pass.set_vertex_buffer(0, self.rect_buffer.buffer.slice(..));
            pass.draw(0..4, self.under..total);
        }
    }

    /// Prepares `frame`, clears `view` with the frame background and draws.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        frame: &Frame,
        size: (u32, u32),
    ) {
        self.prepare(device, queue, frame, size);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("batune-render"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("batune-render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color(frame.background)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw(&mut pass);
        }
        queue.submit([encoder.finish()]);
    }

    /// The frame background as a clear color for this target's format.
    pub fn clear_color(&self, c: Color) -> wgpu::Color {
        let conv = |v: u8| {
            let v = f64::from(v) / 255.0;
            if !self.srgb {
                v
            } else if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        wgpu::Color {
            r: conv(c.r),
            g: conv(c.g),
            b: conv(c.b),
            a: 1.0,
        }
    }
}

fn create_atlas(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("batune-render glyph atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not an sRGB format: glyph coverage is linear alpha, and color
        // glyphs are linearized in the shader only when the target needs it.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    atlas: &wgpu::Texture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = atlas.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("batune-render"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
