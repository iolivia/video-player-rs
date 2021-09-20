use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    thread,
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
    render::{Canvas, Texture, TextureCreator},
    video::{Window, WindowContext},
    AudioSubsystem, EventPump, Sdl, VideoSubsystem,
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
    pub fn new(
        texture_creator: &'a TextureCreator<WindowContext>,
        asset: &PlaybackAssetMetadata,
    ) -> Self {
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

impl VideoRenderingBuffer {
    pub fn is_full(&self) -> bool {
        self.frames.len() >= 10
    }

    pub fn is_empty(&self) -> bool {
        self.frames.len() == 0
    }
}

struct AudioRenderingBuffer {
    frames: VecDeque<frame::Audio>,
}

impl AudioRenderingBuffer {
    pub fn is_full(&self) -> bool {
        self.frames.len() >= 10
    }

    pub fn is_empty(&self) -> bool {
        self.frames.len() == 0
    }
}

struct PlayerBuffer {
    buffer: VecDeque<Packet>,
    ended: bool,
}

// Encoded buffers
impl PlayerBuffer {
    pub fn new() -> Self {
        PlayerBuffer {
            buffer: VecDeque::new(),
            ended: false,
        }
    }

    pub fn push_packet(&mut self, packet: Packet) {
        self.buffer.push_back(packet)
    }

    pub fn packets(&mut self) -> &mut VecDeque<Packet> {
        &mut self.buffer
    }

    pub fn endOfFile(&mut self) {
        self.ended = true;
    }

    pub fn has_ended(&self) -> bool {
        self.buffer.is_empty() && self.ended
    }
}

struct PlayerVideoDecoder {
    video_decoder: VideoDecoder,
}

struct PlayerAudioDecoder {
    audio_decoder: AudioDecoder,
}

impl PlayerVideoDecoder {
    pub fn new(video_decoder: VideoDecoder) -> Self {
        Self { video_decoder }
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
}

impl PlayerAudioDecoder {
    pub fn new(audio_decoder: AudioDecoder) -> Self {
        Self { audio_decoder }
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

    pub fn play(&mut self, mut asset: PlaybackAsset) {
        // Extract asset metadata
        let metadata = asset.metadata.clone();

        // Encoded buffers
        let mut video_player_buffer = Arc::new(Mutex::new(PlayerBuffer::new()));
        let mut audio_player_buffer = Arc::new(Mutex::new(PlayerBuffer::new()));

        // Rendering buffers
        let mut video_rendering_buffer = Arc::new(Mutex::new(VideoRenderingBuffer {
            frames: VecDeque::new(),
        }));
        let mut audio_rendering_buffer = Arc::new(Mutex::new(AudioRenderingBuffer {
            frames: VecDeque::new(),
        }));

        // Decoders
        let mut video_decoder = asset.video_decoder();
        let mut audio_decoder = asset.audio_decoder();

        // Buffer packets
        let buffer_thread = thread::spawn({
            println!("starting buffer thread");
            let video_buffer_ref_clone = Arc::clone(&video_player_buffer);
            let audio_buffer_ref_clone = Arc::clone(&audio_player_buffer);

            move || {
                // Buffer packets
                loop {
                    let packet = asset.packets().next();
                    if let Some((stream, packet)) = packet {
                        match stream.index() {
                            idx if idx == asset.metadata.video_stream_index() => {
                                println!("buffering video packet");
                                let mut buffer = video_buffer_ref_clone.lock().unwrap();
                                buffer.push_packet(packet);
                            }
                            idx if idx == asset.metadata.audio_stream_index() => {
                                println!("buffering audio packet");
                                let mut buffer = audio_buffer_ref_clone.lock().unwrap();
                                buffer.push_packet(packet);
                            }
                            _ => panic!("unrecognized stream index for packet"),
                        }
                    } else {
                        {
                            let mut buffer = video_buffer_ref_clone.lock().unwrap();
                            buffer.endOfFile();
                        }

                        {
                            let mut buffer = audio_buffer_ref_clone.lock().unwrap();
                            buffer.endOfFile();
                        }
                    }
                }
            }
        });

        let decode_video_thread = thread::spawn({
            println!("starting decode_video_thread");
            let buffer_ref_clone = Arc::clone(&video_player_buffer);
            let video_buffer_ref_clone = Arc::clone(&video_rendering_buffer);
            let mut decoder = PlayerVideoDecoder::new(video_decoder);

            move || {
                loop {
                    let mut buffer = buffer_ref_clone.lock().unwrap();

                    // Decode video frames
                    // take from encoded buffers, run through decoder and put into rendering buffer
                    if let Some(packet) = buffer.packets().pop_front() {
                        let frame = decoder.decode_video_packet(packet);

                        println!("pushing decoded video frame");
                        {
                            let mut b = video_buffer_ref_clone.lock().unwrap();

                            b.frames.push_back(frame);
                        }
                    }
                }
            }
        });

        let decode_audio_thread = thread::spawn({
            println!("starting decode_audio_thread");
            let buffer_ref_clone = Arc::clone(&audio_player_buffer);
            let audio_buffer_ref_clone = Arc::clone(&audio_rendering_buffer);
            let mut decoder = PlayerAudioDecoder::new(audio_decoder);
            // println!("decode_audio_thread arcs 1");

            move || {
                loop {
                    let mut buffer = buffer_ref_clone.lock().unwrap();

                    // Decode audio frames
                    // take from encoded buffers, run through decoder and put into rendering buffer
                    if let Some(packet) = buffer.packets().pop_front() {
                        let frame = decoder.decode_audio_packet(packet);
                        println!("pushing decoded audio frame");
                        {
                            let mut b = audio_buffer_ref_clone.lock().unwrap();

                            b.frames.push_back(frame);
                        }
                    }
                }
            }
        });

        // Initialize SDL things
        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();
        let audio_subsystem = sdl_context.audio().unwrap();

        let window = self.create_window(&video_subsystem, &metadata);
        let mut canvas = self.create_canvas(window);
        let mut event_pump = self.create_event_pump(&sdl_context);

        // Audio renderer
        let mut audio_renderer = AudioRenderer::new(&audio_subsystem);
        audio_renderer.initialize();

        // Video renderer
        let texture_creator = canvas.texture_creator();
        let mut video_renderer = VideoRenderer::new(&texture_creator, &metadata);
        video_renderer.initialize();

        // Playback time
        let playback_start_time = Instant::now();

        'running: loop {
            // maybe render video frame
            {
                let mut b = video_rendering_buffer.lock().unwrap();
                if let Some(frame) = b.frames.front() {
                    if self.should_render_video_frame(frame, &metadata, playback_start_time) {
                        let frame = b.frames.pop_front().unwrap();
                        video_renderer.render_frame(&frame);
                        canvas.copy(video_renderer.texture(), None, None).unwrap();
                        canvas.present();
                    }
                }
            }

            // maybe render audio frame
            {
                let mut b = audio_rendering_buffer.lock().unwrap();
                if let Some(frame) = b.frames.front() {
                    if self.should_render_audio_frame(frame, &metadata, playback_start_time) {
                        let frame = b.frames.pop_front().unwrap();
                        audio_renderer.render_frame(&frame);
                    }
                }
            }

            // handle events
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

            // close if we reached EOF
            {
                let vrb = video_rendering_buffer.lock().unwrap();
                let arb = audio_rendering_buffer.lock().unwrap();

                if vrb.is_empty() && arb.is_empty() {
                    let vb = video_player_buffer.lock().unwrap().has_ended();
                    let ab = audio_player_buffer.lock().unwrap().has_ended();

                    // end playback
                    return;
                }
            }

            let duration = Duration::from_millis(1);
            ::std::thread::sleep(duration);
        }
    }

    pub fn should_render_video_frame(
        &self,
        frame: &Video,
        asset: &PlaybackAssetMetadata,
        playback_start_time: Instant,
    ) -> bool {
        self.should_render_frame(frame, asset.video_time_base(), playback_start_time)
    }

    pub fn should_render_audio_frame(
        &self,
        frame: &Audio,
        asset: &PlaybackAssetMetadata,
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

    fn create_window(
        &self,
        video_subsystem: &VideoSubsystem,
        asset: &PlaybackAssetMetadata,
    ) -> Window {
        let window = video_subsystem
            .window("rust-sdl2 demo: Video", asset.width(), asset.height())
            .position_centered()
            .opengl()
            .build()
            .map_err(|e| e.to_string())
            .unwrap();

        window
    }

    fn create_canvas(&self, window: Window) -> Canvas<Window> {
        let mut canvas = window
            .into_canvas()
            .build()
            .map_err(|e| e.to_string())
            .unwrap();

        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();
        canvas.present();

        canvas
    }

    fn create_event_pump(&self, sdl_context: &Sdl) -> EventPump {
        let mut event_pump = sdl_context.event_pump().unwrap();

        // warm up the event pump
        event_pump.pump_events();

        event_pump
    }
}

#[derive(Clone, Copy)]
struct PlaybackAssetMetadata {
    video_stream_index: usize,
    audio_stream_index: usize,
    width: u32,
    height: u32,
    video_time_base: f64,
    audio_time_base: f64,
}

impl PlaybackAssetMetadata {
    pub fn video_stream_index(&self) -> usize {
        self.video_stream_index
    }

    pub fn audio_stream_index(&self) -> usize {
        self.audio_stream_index
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn video_time_base(&self) -> f64 {
        self.video_time_base
    }

    pub fn audio_time_base(&self) -> f64 {
        self.audio_time_base
    }
}

struct PlaybackAsset {
    input: Input,
    metadata: PlaybackAssetMetadata,
}

impl PlaybackAsset {
    pub fn new(path: &str) -> Self {
        // Init ffmpeg
        ffmpeg_next::init().expect("Failed to initialize ffmpeg");

        // Read input video
        let input =
            ffmpeg_next::format::input(&Path::new(path)).expect("Failed to open input video");

        // Get streams
        let video_stream = input.streams().best(Type::Video).unwrap();
        let audio_stream = input.streams().best(Type::Audio).unwrap();

        let video_decoder = video_stream.codec().decoder().video().unwrap();
        let width = video_decoder.width();
        let height = video_decoder.height();

        let video_time_base = {
            let time_base = video_stream.time_base();
            time_base.numerator() as f64 / time_base.denominator() as f64
        };
        let audio_time_base = {
            let time_base = audio_stream.time_base();
            time_base.numerator() as f64 / time_base.denominator() as f64
        };

        let metadata = PlaybackAssetMetadata {
            video_stream_index: video_stream.index(),
            audio_stream_index: audio_stream.index(),
            width,
            height,
            video_time_base,
            audio_time_base,
        };

        PlaybackAsset { input, metadata }
    }

    fn video_stream(&self) -> Stream {
        self.input.streams().best(Type::Video).unwrap()
    }

    fn audio_stream(&self) -> Stream {
        self.input.streams().best(Type::Audio).unwrap()
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
}

fn main() {
    let video_path = "resources/tears-of-steel_teaser.mp4";
    let mut asset = PlaybackAsset::new(video_path);

    let mut player = Player::new();
    player.play(asset);
}
