use camera_controller::CameraController;
use wgpu::{include_wgsl, util::DeviceExt};
use winit::{
  event::*,
  event_loop::{ControlFlow, EventLoop},
  window::{Window, WindowBuilder},
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

mod camera_controller;
mod texture;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
  cfg_if::cfg_if! {
      if #[cfg(target_arch = "wasm32")] {
          std::panic::set_hook(Box::new(console_error_panic_hook::hook));
          console_log::init_with_level(log::Level::Warn).expect("Could't initialize logger");
      } else {
          env_logger::init();
      }
  }

  let event_loop = EventLoop::new();
  let window = WindowBuilder::new().build(&event_loop).unwrap();

  #[cfg(target_arch = "wasm32")]
  {
    // Winit prevents sizing with CSS, so we have to set
    // the size manually when on web.
    use winit::dpi::PhysicalSize;
    window.set_inner_size(PhysicalSize::new(450, 400));

    use winit::platform::web::WindowExtWebSys;
    web_sys::window()
      .and_then(|win| win.document())
      .and_then(|doc| {
        let dst = doc.get_element_by_id("wasm-example")?;
        let canvas = web_sys::Element::from(window.canvas());
        dst.append_child(&canvas).ok()?;
        Some(())
      })
      .expect("Couldn't append canvas to document body.");
  }

  let mut state = State::new(&window).await;

  event_loop.run(move |event, _, control_flow| {
    match event {
      Event::WindowEvent {
        ref event,
        window_id,
      } if window_id == window.id() => {
        if !state.input(event) {
          match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
              input:
                KeyboardInput {
                  state: ElementState::Pressed,
                  virtual_keycode: Some(VirtualKeyCode::Escape),
                  ..
                },
              ..
            } => *control_flow = ControlFlow::Exit,
            WindowEvent::Resized(physical_size) => {
              state.resize(*physical_size);
            }
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
              // new_inner_size is &&mut so we have to dereference it twice
              state.resize(**new_inner_size);
            }
            _ => {}
          }
        }
      }
      Event::RedrawRequested(window_id) if window_id == window.id() => {
        state.update();
        match state.render() {
          Ok(_) => {}
          Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
          Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
          Err(e) => eprintln!("{:?}", e),
        }
      }
      Event::MainEventsCleared => {
        // RedrawRequested will only trigger once, unless we manually request it.
        window.request_redraw();
      }
      _ => {}
    }
  });
}

struct State {
  camera: Camera,
  camera_bind_group: wgpu::BindGroup,
  camera_buffer: wgpu::Buffer,
  camera_controller: CameraController,
  camera_uniform: CameraUniform,
  config: wgpu::SurfaceConfiguration,
  device: wgpu::Device,
  diffuse_bind_group: wgpu::BindGroup,
  index_buffer: wgpu::Buffer,
  num_indices: u32,
  queue: wgpu::Queue,
  render_pipeline: wgpu::RenderPipeline,
  size: winit::dpi::PhysicalSize<u32>,
  surface: wgpu::Surface,
  vertex_buffer: wgpu::Buffer,
}

impl State {
  async fn new(window: &Window) -> Self {
    let size = window.inner_size();

    // The instance is a handle to our GPU
    // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
    let instance = wgpu::Instance::new(wgpu::Backends::all());
    let surface = unsafe { instance.create_surface(window) };
    let adapter = instance
      .request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
      })
      .await
      .unwrap();

    let (device, queue) = adapter
      .request_device(
        &wgpu::DeviceDescriptor {
          features: wgpu::Features::empty(),
          // WebGL doesn't support all of wgpu's features...
          limits: if cfg!(target_arch = "wasm32") {
            wgpu::Limits::downlevel_webgl2_defaults()
          } else {
            wgpu::Limits::default()
          },
          label: None,
        },
        None, // Trace path
      )
      .await
      .unwrap();

    let config = wgpu::SurfaceConfiguration {
      usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
      format: surface.get_preferred_format(&adapter).unwrap(),
      width: size.width,
      height: size.height,
      present_mode: wgpu::PresentMode::Fifo,
    };
    surface.configure(&device, &config);

    let camera_controller = CameraController::new(0.2);

    let diffuse_bytes = include_bytes!("../happy-tree.png");
    let diffuse_texture =
      texture::Texture::from_bytes(&device, &queue, diffuse_bytes, "happy-tree.png").unwrap();

    let texture_bind_group_layout =
      device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
          wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
              multisampled: false,
              view_dimension: wgpu::TextureViewDimension::D2,
              sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
          },
          wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            // This should match the filterable field of the corresponding Texture entry above.
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
          },
        ],
        label: Some("texture_bind_group_layout"),
      });

    let diffuse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      layout: &texture_bind_group_layout,
      entries: &[
        wgpu::BindGroupEntry {
          binding: 0,
          resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
        },
        wgpu::BindGroupEntry {
          binding: 1,
          resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
        },
      ],
      label: Some("diffuse_bind_group"),
    });

    let camera = Camera {
      eye: (0.0, 1.0, 2.0).into(),
      target: (0.0, 0.0, 0.0).into(),
      up: cgmath::Vector3::unit_y(),
      aspect: config.width as f32 / config.height as f32,
      fovy: 45.0,
      znear: 0.1,
      zfar: 100.0,
    };

    let mut camera_uniform = CameraUniform::new();
    camera_uniform.update_view_proj(&camera);

    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
      label: Some("Camera Buffer"),
      contents: bytemuck::cast_slice(&[camera_uniform]),
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // buffers
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
      label: Some("Vertex Buffer"),
      contents: bytemuck::cast_slice(VERTICES),
      usage: wgpu::BufferUsages::VERTEX,
    });

    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
      label: Some("Index Buffer"),
      contents: bytemuck::cast_slice(INDICES),
      usage: wgpu::BufferUsages::INDEX,
    });

    let camera_bind_group_layout =
      device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[wgpu::BindGroupLayoutEntry {
          binding: 0,
          visibility: wgpu::ShaderStages::VERTEX,
          ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
          },
          count: None,
        }],
        label: Some("camera_bind_group_layout"),
      });

    let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("camera_bind_group"),
      layout: &camera_bind_group_layout,
      entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: camera_buffer.as_entire_binding(),
      }],
    });

    let num_indices = INDICES.len() as u32;

    let shader = device.create_shader_module(&include_wgsl!("shader.wgsl"));

    let render_pipline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some("Render Pipeline Layout"),
      bind_group_layouts: &[&texture_bind_group_layout, &camera_bind_group_layout],
      push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
      label: Some("Render Pipeline"),
      layout: Some(&render_pipline_layout),
      vertex: wgpu::VertexState {
        module: &shader,
        entry_point: "vs_main",
        buffers: &[Vertex::desc()],
      },
      fragment: Some(wgpu::FragmentState {
        module: &shader,
        entry_point: "fs_main",
        targets: &[wgpu::ColorTargetState {
          format: config.format,
          blend: Some(wgpu::BlendState::REPLACE),
          write_mask: wgpu::ColorWrites::ALL,
        }],
      }),
      primitive: wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        strip_index_format: None,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: Some(wgpu::Face::Back),
        polygon_mode: wgpu::PolygonMode::Fill,
        unclipped_depth: false,
        conservative: false,
      },
      depth_stencil: None,
      multisample: wgpu::MultisampleState {
        count: 1,
        mask: !0,
        alpha_to_coverage_enabled: false,
      },
      multiview: None,
    });

    Self {
      camera,
      camera_bind_group,
      camera_buffer,
      camera_controller,
      camera_uniform,
      config,
      device,
      diffuse_bind_group,
      index_buffer,
      num_indices,
      queue,
      render_pipeline,
      size,
      surface,
      vertex_buffer,
    }
  }

  fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
    if new_size.width > 0 && new_size.height > 0 {
      self.size = new_size;
      self.config.width = new_size.width;
      self.config.height = new_size.height;
      self.surface.configure(&self.device, &self.config);
    }
  }

  fn input(&mut self, event: &WindowEvent) -> bool {
    self.camera_controller.process_events(event)
  }

  fn update(&mut self) {
    self.camera_controller.update_camera(&mut self.camera);
    self.camera_uniform.update_view_proj(&self.camera);
    self.queue.write_buffer(
      &self.camera_buffer,
      0,
      bytemuck::cast_slice(&[self.camera_uniform]),
    );
  }

  fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
    let output = self.surface.get_current_texture()?;

    let view = output
      .texture
      .create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
      });

    {
      let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Render Pass"),
        color_attachments: &[wgpu::RenderPassColorAttachment {
          view: &view,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color {
              r: 0.1,
              g: 0.1,
              b: 0.1,
              a: 1.0,
            }),
            store: true,
          },
        }],
        depth_stencil_attachment: None,
      });
      render_pass.set_pipeline(&self.render_pipeline);
      render_pass.set_bind_group(0, &self.diffuse_bind_group, &[]);
      render_pass.set_bind_group(1, &self.camera_bind_group, &[]);
      render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
      render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
      render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
    }

    // submit will accept anyting that implments IntoIter
    self.queue.submit(std::iter::once(encoder.finish()));
    output.present();

    Ok(())
  }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
  position: [f32; 3],
  tex_coords: [f32; 2],
}

impl Vertex {
  const ATTRIBS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];

  fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
    use std::mem;

    wgpu::VertexBufferLayout {
      array_stride: mem::size_of::<Self>() as wgpu::BufferAddress,
      step_mode: wgpu::VertexStepMode::Vertex,
      attributes: &Self::ATTRIBS,
    }
  }
}

const VERTICES: &[Vertex] = &[
  Vertex {
    position: [-0.0868241, 0.49240386, 0.0],
    tex_coords: [0.4131759, 1.0 - 0.99240386],
  }, // A
  Vertex {
    position: [-0.49513406, 0.06958647, 0.0],
    tex_coords: [0.0048659444, 1.0 - 0.56958647],
  }, // B
  Vertex {
    position: [-0.21918549, -0.44939706, 0.0],
    tex_coords: [0.28081453, 1.0 - 0.05060294],
  }, // C
  Vertex {
    position: [0.35966998, -0.3473291, 0.0],
    tex_coords: [0.85967, 1.0 - 0.1526709],
  }, // D
  Vertex {
    position: [0.44147372, 0.2347359, 0.0],
    tex_coords: [0.9414737, 1.0 - 0.7347359],
  }, // E
];

const INDICES: &[u16] = &[0, 1, 4, 1, 2, 4, 2, 3, 4];

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);

pub struct Camera {
  pub eye: cgmath::Point3<f32>,
  pub target: cgmath::Point3<f32>,
  pub up: cgmath::Vector3<f32>,
  pub aspect: f32,
  pub fovy: f32,
  pub znear: f32,
  pub zfar: f32,
}

impl Camera {
  fn build_view_projection_matrix(&self) -> cgmath::Matrix4<f32> {
    // 1.
    let view = cgmath::Matrix4::look_at_rh(self.eye, self.target, self.up);
    // 2.
    let proj = cgmath::perspective(cgmath::Deg(self.fovy), self.aspect, self.znear, self.zfar);

    // 3.
    return OPENGL_TO_WGPU_MATRIX * proj * view;
  }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
  // We can't use cgmath with bytemuck directly so convert the Matrix4 into a 4x4 f32 array
  view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
  fn new() -> Self {
    use cgmath::SquareMatrix;
    Self {
      view_proj: cgmath::Matrix4::identity().into(),
    }
  }

  fn update_view_proj(&mut self, camera: &Camera) {
    self.view_proj = camera.build_view_projection_matrix().into()
  }
}
