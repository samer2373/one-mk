pub fn gif(width: u32, height: u32, count: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for index in 0..count {
            let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([index as u8, 16, 32, 255]));
            encoder.encode_frame(image::Frame::from_parts(pixels, 0, 0, image::Delay::from_numer_denom_ms(100, 1))).unwrap();
        }
    }
    bytes
}

pub fn apng() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_animated(2, 1).unwrap();
        encoder.set_frame_delay(1, 10).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[8, 16, 32, 255].repeat(4)).unwrap();
        writer.write_image_data(&[32, 16, 8, 255].repeat(4)).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut bytes = kind.to_vec();
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(data);
    if data.len() & 1 != 0 { bytes.push(0); }
    bytes
}

pub fn webp() -> Vec<u8> {
    let mut payload = b"WEBP".to_vec();
    payload.extend(chunk(b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 1, 0, 0]));
    payload.extend(chunk(b"ANIM", &[0, 0, 0, 0, 1, 0]));
    for red in [8, 32] {
        let mut still = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut still).encode(&[red, 16, 32, 255].repeat(4), 2, 2, image::ExtendedColorType::Rgba8).unwrap();
        let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 100, 0, 0, 0];
        frame.extend_from_slice(&still[12..]);
        payload.extend(chunk(b"ANMF", &frame));
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}
