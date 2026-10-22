//! 解碼前檢查 WebP 的內外層尺寸。
//!
//! image-webp 會在比對 VP8X canvas 與 VP8 frame 尺寸之前配置 frame buffer；
//! ImageReader 的尺寸限制僅看到 canvas，因此也須檢查每個實際影像 chunk。

fn u24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

fn dimensions(width: u32, height: u32) -> Result<(u32, u32), ()> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(());
    }
    Ok((width, height))
}

pub(super) fn validate(bytes: &[u8]) -> Result<(), ()> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(());
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| ())?);
    if u64::from(declared) + 8 != bytes.len() as u64 {
        return Err(());
    }
    chunks(&bytes[12..], None, true)?;
    Ok(())
}

fn chunks(bytes: &[u8], mut canvas: Option<(u32, u32)>, allow_frames: bool) -> Result<(), ()> {
    let mut offset = 0usize;
    let mut found_image = false;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset.checked_add(8).ok_or(())?)
            .ok_or(())?;
        let size = u32::from_le_bytes(header[4..8].try_into().map_err(|_| ())?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or(())?;
        let data = bytes.get(start..end).ok_or(())?;
        offset = end.checked_add(size % 2).ok_or(())?;
        if offset > bytes.len() {
            return Err(());
        }
        match &header[..4] {
            b"VP8X" => {
                if !allow_frames || canvas.is_some() || found_image || data.len() != 10 {
                    return Err(());
                }
                canvas = Some(dimensions(u24(&data[4..7]) + 1, u24(&data[7..10]) + 1)?);
            }
            b"VP8 " | b"VP8L" => {
                let actual = if &header[..4] == b"VP8 " {
                    if data.len() < 10 || data[0] & 1 != 0 || &data[3..6] != b"\x9d\x01\x2a" {
                        return Err(());
                    }
                    dimensions(
                        u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff),
                        u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff),
                    )?
                } else {
                    if data.len() < 5 || data[0] != 0x2f {
                        return Err(());
                    }
                    let bits = u32::from_le_bytes(data[1..5].try_into().map_err(|_| ())?);
                    dimensions((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)?
                };
                if found_image || canvas.is_some_and(|expected| expected != actual) {
                    return Err(());
                }
                canvas = Some(actual);
                found_image = true;
            }
            b"ANMF" => {
                if !allow_frames || data.len() < 16 {
                    return Err(());
                }
                let (width, height) = dimensions(u24(&data[6..9]) + 1, u24(&data[9..12]) + 1)?;
                let (canvas_width, canvas_height) = canvas.ok_or(())?;
                let x = u24(&data[..3]) * 2;
                let y = u24(&data[3..6]) * 2;
                if x + width > canvas_width || y + height > canvas_height {
                    return Err(());
                }
                chunks(&data[16..], Some((width, height)), false)?;
                found_image = true;
            }
            _ => {}
        }
    }
    if !found_image {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn riff(chunks: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend(((chunks.len() + 4) as u32).to_le_bytes());
        out.extend(b"WEBP");
        out.extend(chunks);
        out
    }

    #[test]
    fn rejects_large_vp8_hidden_by_small_extended_canvas() {
        let mut data = chunk(b"VP8X", &[0; 10]);
        data.extend(chunk(
            b"VP8 ",
            &[0, 0, 0, 0x9d, 1, 0x2a, 0xff, 0x3f, 0xff, 0x3f],
        ));
        assert!(validate(&riff(&data)).is_err());
    }

    #[test]
    fn rejects_large_nested_animation_frames_and_truncated_containers() {
        let mut data = chunk(b"VP8X", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut frame = vec![0; 16];
        frame.extend(chunk(b"VP8L", &[0x2f, 0xff, 0xff, 0xff, 0x0f]));
        data.extend(chunk(b"ANMF", &frame));
        assert!(validate(&riff(&data)).is_err());
        assert!(validate(b"RIFF\xff\xff\xff\xffWEBP").is_err());
    }
}
