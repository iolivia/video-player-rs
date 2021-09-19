use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

use ffmpeg_next::{
    codec::decoder::audio::Audio as AudioDecoder,
    codec::decoder::video::Video as VideoDecoder,
    decoder,
    format::{
        context::{input::PacketIter, Input},
        sample::Type as AudioType,
        Sample,
    },
    frame::{self, Audio, Video},
    media::Type,
    Frame, Packet, Stream,
};
use sdl2::{
    audio::{AudioQueue, AudioSpecDesired},
    event::Event,
    keyboard::Keycode,
    pixels::{Color, PixelFormatEnum},
    render::{Texture, TextureCreator},
    video::WindowContext,
    AudioSubsystem,
};

struct AudioRenderer {
    audio_device: AudioQueue<f32>,
}

impl AudioRenderer {
    pub fn new(audio_subsystem: &AudioSubsystem) -> Self {
        let audio_spec = AudioSpecDesired {
            freq: None, //Some(44100 / 2),
            channels: Some(2),
            samples: None,
        };

        let audio_device = audio_subsystem
            .open_queue::<f32, _>(None, &audio_spec)
            .unwrap();

        AudioRenderer { audio_device }
    }

    pub fn initialize(&mut self) {
        self.audio_device.resume();
    }

    pub fn render_frame(&mut self, frame: &Audio) {
        self.audio_device.queue(frame.plane::<f32>(0));
    }
}

struct VideoRenderer<'a> {
    texture: Texture<'a>,
    width: u32,
    height: u32,
}

impl<'a> VideoRenderer<'a> {
    pub fn new(texture_creator: &'a TextureCreator<WindowContext>, asset: &PlaybackAsset) -> Self {
        let width = asset.width();
        let height = asset.height();

        let texture = texture_creator
            .create_texture_streaming(PixelFormatEnum::YV12, width, height)
            .unwrap();

        VideoRenderer {
            texture,
            width,
            height,
        }
    }

    pub fn initialize(&mut self) {}

    pub fn render_frame(&mut self, frame: &Video) {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.extend_from_slice(frame.data(0));
        buffer.extend_from_slice(frame.data(2));
        buffer.extend_from_slice(frame.data(1));

        self.texture
            .update(None, &buffer, self.width as usize)
            .unwrap();
    }

    pub fn texture(&self) -> &Texture<'a> {
        &self.texture
    }
}

struct VideoRenderingBuffer {
    frames: VecDeque<frame::Video>,
}

struct AudioRenderingBuffer {
    frames: VecDeque<frame::Audio>,
}

struct PlayerBuffer {
    video_buffer: VecDeque<Packet>,
    audio_buffer: VecDeque<Packet>,
    video_stream_index: usize,
    audio_stream_index: usize,
}

// Encoded buffers
impl PlayerBuffer {
    pub fn new(asset: &PlaybackAsset) -> Self {
        PlayerBuffer {
            video_buffer: VecDeque::new(),
            audio_buffer: VecDeque::new(),
            video_stream_index: asset.video_stream_index(),
            audio_stream_index: asset.audio_stream_index(),
        }
    }

    pub fn push_packet(&mut self, stream_index: usize, packet: Packet) {
        match stream_index {
            idx if idx == self.video_stream_index => {
                self.push_video_packet(packet);
            }
            idx if idx == self.audio_stream_index => {
                self.push_audio_packet(packet);
            }
            _ => panic!("unrecognized stream index for packet"),
        }
    }

    pub fn video_packets(&mut self) -> &mut VecDeque<Packet> {
        &mut self.video_buffer
    }

    pub fn audio_packets(&mut self) -> &mut VecDeque<Packet> {
        &mut self.audio_buffer
    }

    fn push_video_packet(&mut self, packet: Packet) {
        self.video_buffer.push_back(packet);
    }

    fn push_audio_packet(&mut self, packet: Packet) {
        self.audio_buffer.push_back(packet);
    }
}

struct PlayerDecoder {
    video_decoder: VideoDecoder,
    audio_decoder: AudioDecoder,
}

impl PlayerDecoder {
    pub fn new(asset: &PlaybackAsset) -> Self {
        PlayerDecoder {
            video_decoder: asset.video_decoder(),
            audio_decoder: asset.audio_decoder(),
        }
    }

    pub fn decode_video_packet(&mut self, packet: Packet) -> Video {
        // Send packet to the decoder
        self.video_decoder
            .send_packet(&packet)
            .expect("Failed to send packet to video decoder");

        // Get frame
        let mut frame = frame::Video::empty();

        self.video_decoder.receive_frame(&mut frame).ok();

        frame
    }

    pub fn decode_audio_packet(&mut self, packet: Packet) -> Audio {
        // Send packet to the decoder
        self.audio_decoder
            .send_packet(&packet)
            .expect("Failed to send packet to audio decoder");

        // Get frame
        let mut frame = frame::Audio::empty();
        frame.set_format(Sample::F32(AudioType::Packed));

        self.audio_decoder.receive_frame(&mut frame).ok();

        frame
    }
}

struct Player {}

impl Player {
    pub fn new() -> Self {
        Player {}
    }

    pub fn play(&mut self, asset: &mut PlaybackAsset) {
        let mut player_buffer = PlayerBuffer::new(&asset);

        let mut video_rendering_buffer = VideoRenderingBuffer {
            frames: VecDeque::new(),
        };
        let mut audio_rendering_buffer = AudioRenderingBuffer {
            frames: VecDeque::new(),
        };

        let mut player_decoder = PlayerDecoder::new(&asset);

        // Buffer packets
        for _ in 0..500 {
            let packet = asset.packets().next();
            if let Some((stream, packet)) = packet {
                player_buffer.push_packet(stream.index(), packet);
            }
        }

        // Decode video frames
        for packet in player_buffer.video_packets().drain(..) {
            let frame = player_decoder.decode_video_packet(packet);
            video_rendering_buffer.frames.push_back(frame);
        }

        // Decode audio frames
        for packet in player_buffer.audio_packets().drain(..) {
            let frame = player_decoder.decode_audio_packet(packet);
            audio_rendering_buffer.frames.push_back(frame);
        }

        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();
        let audio_subsystem = sdl_context.audio().unwrap();

        let window = video_subsystem
            .window("rust-sdl2 demo: Video", asset.width(), asset.height())
            .position_centered()
            .opengl()
            .build()
            .map_err(|e| e.to_string())
            .unwrap();

        let mut canvas = window
            .into_canvas()
            .build()
            .map_err(|e| e.to_string())
            .unwrap();

        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();
        canvas.present();
        let mut event_pump = sdl_context.event_pump().unwrap();
        event_pump.pump_events(); // warm up the event pump

        let mut audio_renderer = AudioRenderer::new(&audio_subsystem);
        audio_renderer.initialize();

        let texture_creator = canvas.texture_creator();
        let mut video_renderer = VideoRenderer::new(&texture_creator, &asset);
        video_renderer.initialize();

        let playback_start_time = Instant::now();

        'running: loop {
            if let Some(frame) = video_rendering_buffer.frames.front() {
                if self.should_render_video_frame(frame, asset, playback_start_time) {
                    let frame = video_rendering_buffer.frames.pop_front().unwrap();
                    video_renderer.render_frame(&frame);
                    canvas.copy(video_renderer.texture(), None, None).unwrap();
                    canvas.present();
                }
            }

            if let Some(frame) = audio_rendering_buffer.frames.front() {
                if self.should_render_audio_frame(frame, asset, playback_start_time) {
                    let frame = audio_rendering_buffer.frames.pop_front().unwrap();
                    audio_renderer.render_frame(&frame);
                }
            }

            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. }
                    | Event::KeyDown {
                        keycode: Some(Keycode::Escape),
                        ..
                    } => break 'running,
                    _ => {}
                }
            }

            let seconds_duration = Duration::from_millis(1);
            ::std::thread::sleep(seconds_duration);
        }
    }

    pub fn should_render_video_frame(
        &self,
        frame: &Video,
        asset: &PlaybackAsset,
        playback_start_time: Instant,
    ) -> bool {
        self.should_render_frame(frame, asset.video_time_base(), playback_start_time)
    }

    pub fn should_render_audio_frame(
        &self,
        frame: &Audio,
        asset: &PlaybackAsset,
        playback_start_time: Instant,
    ) -> bool {
        self.should_render_frame(frame, asset.audio_time_base(), playback_start_time)
    }

    fn should_render_frame(
        &self,
        frame: &Frame,
        time_base: f64,
        playback_start_time: Instant,
    ) -> bool {
        if let Some(pts) = frame.pts() {
            let pts = pts as f64 * time_base * 1000_f64;
            let show_time = Duration::from_millis(pts as u64);
            let playback_time_elapsed = Instant::now().duration_since(playback_start_time);

            playback_time_elapsed > show_time
        } else {
            false
        }
    }
}

struct PlaybackAsset {
    input: Input,
    width: u32,
    height: u32,
}

impl PlaybackAsset {
    pub fn new(path: &str) -> Self {
        // Init ffmpeg
        ffmpeg_next::init().expect("Failed to initialize ffmpeg");

        // Read input video
        let input =
            ffmpeg_next::format::input(&Path::new(path)).expect("Failed to open input video");

        // Get stream
        let video_stream = input.streams().best(Type::Video).unwrap();

        let video_decoder = video_stream.codec().decoder().video().unwrap();
        let width = video_decoder.width();
        let height = video_decoder.height();

        PlaybackAsset {
            input,
            width,
            height,
        }
    }

    fn video_stream(&self) -> Stream {
        self.input.streams().best(Type::Video).unwrap()
    }

    fn audio_stream(&self) -> Stream {
        self.input.streams().best(Type::Audio).unwrap()
    }

    pub fn video_stream_index(&self) -> usize {
        self.video_stream().index()
    }

    pub fn audio_stream_index(&self) -> usize {
        self.audio_stream().index()
    }

    pub fn packets(&mut self) -> PacketIter {
        self.input.packets()
    }

    pub fn video_decoder(&self) -> decoder::Video {
        self.video_stream().codec().decoder().video().unwrap()
    }

    pub fn audio_decoder(&self) -> decoder::Audio {
        self.audio_stream().codec().decoder().audio().unwrap()
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn video_time_base(&self) -> f64 {
        let time_base = self.video_stream().time_base();
        time_base.numerator() as f64 / time_base.denominator() as f64
    }

    pub fn audio_time_base(&self) -> f64 {
        let time_base = self.audio_stream().time_base();
        time_base.numerator() as f64 / time_base.denominator() as f64
    }
}

fn main() {
    let video_path = "resources/tears-of-steel_teaser.mp4";
    let mut asset = PlaybackAsset::new(video_path);

    let mut player = Player::new();
    player.play(&mut asset);
}
