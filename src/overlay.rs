use std::cell::RefCell;
use std::ffi::c_void;
use std::io::Cursor;
use std::mem::ManuallyDrop;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rodio::{Decoder, OutputStream, Sink};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use crate::api;

const GIF_BYTES: &[u8] = include_bytes!("../assets/overlay.gif");
const SOUND_BYTES: &[u8] = include_bytes!("../assets/sound.mp3");

struct DecodedFrame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    delay_ms: u32,
}

fn decode_gif() -> Vec<DecodedFrame> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);

    let mut decoder = match options.read_info(GIF_BYTES) {
        Ok(d) => d,
        Err(e) => {
            api::log_warn(&format!("launch_overlay: gif decode init failed: {e}"));
            return Vec::new();
        }
    };

    let mut frames = Vec::new();
    loop {
        match decoder.read_next_frame() {
            Ok(Some(frame)) => {
                let delay_ms = (frame.delay as u32) * 10;
                frames.push(DecodedFrame {
                    pixels: frame.buffer.to_vec(),
                    width: frame.width as u32,
                    height: frame.height as u32,
                    delay_ms: delay_ms.max(20),
                });
            }
            Ok(None) => break,
            Err(e) => {
                api::log_warn(&format!("launch_overlay: gif frame read failed: {e}"));
                break;
            }
        }
    }
    frames
}

struct OverlayState {
    egui_ctx: egui::Context,
    renderer: Option<egui_directx11::Renderer>,
    device_context: Option<ID3D11DeviceContext>,
    frames: Vec<DecodedFrame>,
    textures: Vec<egui::TextureHandle>,
    total_loop_ms: u64,
    started: Instant,
    active_until: Option<Instant>,
    _audio_stream: Option<OutputStream>,
    audio_sink: Option<Sink>,
}

thread_local! {
    static STATE: RefCell<Option<OverlayState>> = RefCell::new(None);
}

static LOGGED_FIRST_FRAME: OnceLock<()> = OnceLock::new();

pub fn trigger() {
    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(state) = slot.as_mut() {
            state.active_until = Some(Instant::now() + Duration::from_secs(5));
            state.started = Instant::now();

            if let Some(sink) = &state.audio_sink {
                sink.stop();
                let cursor = Cursor::new(SOUND_BYTES);
                if let Ok(source) = Decoder::new(cursor) {
                    sink.set_volume(1.0); // Set max volume (100%)
                    sink.append(source);
                    sink.play();
                }
            }
        }
    });
}

fn current_frame_index(state: &OverlayState) -> usize {
    if state.frames.is_empty() || state.total_loop_ms == 0 {
        return 0;
    }
    let elapsed_ms = state.started.elapsed().as_millis() as u64 % state.total_loop_ms;
    let mut acc = 0u64;
    for (i, f) in state.frames.iter().enumerate() {
        acc += f.delay_ms as u64;
        if elapsed_ms < acc {
            return i;
        }
    }
    state.frames.len() - 1
}

pub unsafe extern "C" fn on_present(swapchain: *mut c_void, _userdata: *mut c_void) {
    if swapchain.is_null() {
        return;
    }

    // Automatically poll IL2CPP stats every frame!
    if crate::stats::check_career_stats() {
        trigger();
    }

    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();

        if slot.is_none() {
            let frames = decode_gif();
            let total_loop_ms: u64 = frames.iter().map(|f| f.delay_ms as u64).sum();

            let (stream, sink) = match OutputStream::try_default() {
                Ok((s, handle)) => (Some(s), Sink::try_new(&handle).ok()),
                Err(_) => (None, None),
            };

            *slot = Some(OverlayState {
                egui_ctx: egui::Context::default(),
                renderer: None,
                device_context: None,
                frames,
                textures: Vec::new(),
                total_loop_ms: total_loop_ms.max(1),
                started: Instant::now(),
                active_until: None,
                _audio_stream: stream,
                audio_sink: sink,
            });
        }

        let state = slot.as_mut().unwrap();

        let is_active = match state.active_until {
            Some(until) if Instant::now() < until => true,
            Some(_) => {
                state.active_until = None;
                false
            }
            None => false,
        };

        if !is_active {
            return;
        }

        let swap_chain: ManuallyDrop<IDXGISwapChain> =
            ManuallyDrop::new(IDXGISwapChain::from_raw_borrowed(&swapchain).unwrap().clone());

        if state.renderer.is_none() {
            let device: ID3D11Device = match (*swap_chain).GetDevice() {
                Ok(d) => d,
                Err(e) => {
                    api::log_warn(&format!("launch_overlay: GetDevice failed: {e}"));
                    return;
                }
            };
            let context = device.GetImmediateContext().ok();

            match egui_directx11::Renderer::new(&device) {
                Ok(renderer) => {
                    state.renderer = Some(renderer);
                    state.device_context = context;
                    for (i, f) in state.frames.iter().enumerate() {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [f.width as usize, f.height as usize],
                            &f.pixels,
                        );
                        let handle = state.egui_ctx.load_texture(
                            format!("launch_overlay_frame_{i}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        state.textures.push(handle);
                    }
                }
                Err(e) => {
                    api::log_warn(&format!(
                        "launch_overlay: egui_directx11::Renderer::new failed: {e}"
                    ));
                    return;
                }
            }
        }

        let frame_index = current_frame_index(state);
        let texture_id = state.textures.get(frame_index).map(|t| t.id());

        let (Some(renderer), Some(device_context)) =
            (state.renderer.as_mut(), state.device_context.as_ref())
        else {
            return;
        };
        if state.textures.is_empty() {
            return;
        }
        let Some(texture_id) = texture_id else {
            return;
        };

        let back_buffer: ID3D11Texture2D = match (*swap_chain).GetBuffer(0) {
            Ok(b) => b,
            Err(e) => {
                if LOGGED_FIRST_FRAME.set(()).is_ok() {
                    api::log_warn(&format!("launch_overlay: GetBuffer failed: {e}"));
                }
                return;
            }
        };
        let device: ID3D11Device = match device_context.GetDevice() {
            Ok(d) => d,
            Err(e) => {
                if LOGGED_FIRST_FRAME.set(()).is_ok() {
                    api::log_warn(&format!("launch_overlay: GetDevice failed: {e}"));
                }
                return;
            }
        };

        let mut render_target_view: Option<
            windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView,
        > = None;
        if let Err(e) =
            device.CreateRenderTargetView(&back_buffer, None, Some(&mut render_target_view))
        {
            if LOGGED_FIRST_FRAME.set(()).is_ok() {
                api::log_warn(&format!("launch_overlay: CreateRenderTargetView failed: {e}"));
            }
            return;
        }
        let Some(render_target_view) = render_target_view else {
            return;
        };

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { back_buffer.GetDesc(&mut desc) };
        let screen_size = egui::vec2(desc.Width as f32, desc.Height as f32);

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen_size)),
            ..Default::default()
        };

        let ctx = state.egui_ctx.clone();
        let full_output = ctx.run(raw_input, |ctx| {
            egui::Area::new(egui::Id::new("launch_overlay_fullscreen"))
                .fixed_pos(egui::Pos2::ZERO)
                .show(ctx, |ui| {
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(texture_id, screen_size))
                            .fit_to_exact_size(screen_size),
                    );
                });
        });

        let (renderer_output, _platform_output, _viewport_output) =
            egui_directx11::split_output(full_output);

        if let Err(e) = renderer.render(
            device_context,
            &render_target_view,
            &ctx,
            renderer_output,
        ) {
            if LOGGED_FIRST_FRAME.set(()).is_ok() {
                api::log_warn(&format!("launch_overlay: render failed: {e}"));
            }
        }
    });
}