use std::{
    io::{BufReader, Read},
    process::{Command, Stdio},
};

fn main() {
    println!("Hello, world!");

    // Demux - split video into audio and video tracks
    // ffmpeg -i video-orig.mp4 -an -vcodec copy video-demuxed.m4v
    let pwd_command = Command::new("pwd").output().unwrap();
    let pwd = String::from_utf8(pwd_command.stdout).unwrap();
    println!("pwd {}", pwd);

    let input_file = "Big_Buck_Bunny_360_10s_1MB.mp4";
    let output_file = "video_Big_Buck_Bunny_360_10s_1MB.mp4";
    let mut command = Command::new("ffmpeg");
    command.args(&["-i", input_file, "-an", "-vcodec", "copy", output_file]);

    println!("command {:?}", command);

    let mut output = command
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to execute command");

    let mut buffer: [u8; 65536] = [0; 65536];
    let buffer_size = output
        .stdout
        .take()
        .unwrap()
        .read(&mut buffer)
        .expect("Failed to read buffer");

    println!("Buffer size {}", buffer_size);

    println!("Finished without errors");

    // Decode each track - from compressed format to uncompressed
    // Get video frames
    // Render video frames
}
