use std::{env, fs, path::PathBuf};

fn main() {
    let source = "../toki-gui/icons/128x128.png";
    println!("cargo:rerun-if-changed={source}");
    let decoder = png::Decoder::new(fs::File::open(source).expect("shared application icon"));
    let mut reader = decoder.read_info().expect("valid application PNG");
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .expect("application icon pixels");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.width, info.height);
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("tray-icon.rgba"), &pixels[..info.buffer_size()]).unwrap();
    fs::write(
        out.join("tray-icon-size.rs"),
        format!("const SOURCE_SIZE: usize = {};", info.width),
    )
    .unwrap();
}
