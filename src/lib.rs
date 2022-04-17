use winit::{
  dpi::PhysicalPosition,
  event::*,
  event_loop::{ControlFlow, EventLoop},
  window::{Window, WindowBuilder},
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

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
  surface: wgpu::Surface,
  device: wgpu::Device,
  queue: wgpu::Queue,
  config: wgpu::SurfaceConfiguration,
  size: winit::dpi::PhysicalSize<u32>,
  clear_color: wgpu::Color,
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

    Self {
      surface,
      device,
      queue,
      config,
      size,
      clear_color: wgpu::Color {
        r: 0.4,
        g: 0.2,
        b: 0.3,
        a: 1.0,
      },
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
    match event {
      WindowEvent::CursorMoved { position, .. } => {
        let PhysicalPosition { x, y } = position;
        self.clear_color = wgpu::Color {
          r: x / f64::from(self.size.width),
          g: y / f64::from(self.size.height),
          b: 0.5,
          a: 1.0,
        };
        true
      }
      _ => false,
    }
  }

  fn update(&mut self) {
    // todo!()
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
      let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Render Pass"),
        color_attachments: &[wgpu::RenderPassColorAttachment {
          view: &view,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(self.clear_color),
            store: true,
          },
        }],
        depth_stencil_attachment: None,
      });
    }
    // submit will accept anyting that implments IntoIter
    self.queue.submit(std::iter::once(encoder.finish()));
    output.present();

    Ok(())
  }
}
