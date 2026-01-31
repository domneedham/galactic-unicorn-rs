use embedded_graphics::{geometry::Point, pixelcolor::Rgb888};

use crate::display::GraphicsBufferWriter;

/// Protocol version
pub const VERSION_1: u8 = 0x01;

/// Command bytes
pub const CMD_CLEAR: u8 = 0x00;
pub const CMD_SET_PIXEL: u8 = 0x01;
pub const CMD_SET_PIXELS: u8 = 0x02;
pub const CMD_FILL: u8 = 0x03;
pub const CMD_PING: u8 = 0xFE;
pub const CMD_PONG: u8 = 0xFF;

/// Error type for command parsing
#[derive(Debug, Clone, Copy)]
pub enum ParseError {
    /// Not enough data available yet - need to wait for more bytes
    NeedMoreData,
    /// Invalid command or data
    Invalid(&'static str),
}

/// Try to parse a single command from the data buffer.
/// Returns the number of bytes consumed if successful, or an error.
///
/// This function is transport-agnostic - it doesn't send responses.
/// The caller is responsible for handling PING/PONG responses.
pub async fn try_parse_command(
    data: &[u8],
    graphics_buffer: &mut GraphicsBufferWriter,
) -> Result<usize, ParseError> {
    if data.len() < 2 {
        return Err(ParseError::NeedMoreData);
    }

    if data[0] != VERSION_1 {
        return Err(ParseError::Invalid("Unknown version"));
    }

    match data[1] {
        CMD_CLEAR => {
            // Clear and render
            graphics_buffer.clear().await;
            Ok(2) // Consumed 2 bytes: version + command
        }
        CMD_SET_PIXEL => {
            if data.len() < 7 {
                return Err(ParseError::NeedMoreData);
            }

            // Batch process consecutive SET_PIXEL commands with single mutex lock
            let mut offset = 0;
            let mut pixels = graphics_buffer.pixels_mut().await;

            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut max_x = i32::MIN;
            let mut max_y = i32::MIN;

            while offset + 7 <= data.len()
                && data[offset] == VERSION_1
                && data[offset + 1] == CMD_SET_PIXEL
            {
                let x = data[offset + 2] as i32;
                let y = data[offset + 3] as i32;
                let r = data[offset + 4];
                let g = data[offset + 5];
                let b = data[offset + 6];

                pixels.set_pixel(Point::new(x, y), Rgb888::new(r, g, b));

                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);

                offset += 7;
            }

            if min_x != i32::MAX && min_x >= 0 && max_x >= 0 && min_y >= 0 && max_y >= 0 {
                pixels.mark_dirty_region(
                    min_x as usize,
                    min_y as usize,
                    max_x as usize,
                    max_y as usize,
                );
            }

            drop(pixels);
            graphics_buffer.send();
            Ok(offset)
        }
        CMD_SET_PIXELS => {
            if data.len() < 3 {
                return Err(ParseError::NeedMoreData);
            }

            let num_pixels = data[2] as usize;
            let required_len = 3 + num_pixels * 5;
            if data.len() < required_len {
                return Err(ParseError::NeedMoreData);
            }

            let mut pixels = graphics_buffer.pixels_mut().await;

            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut max_x = i32::MIN;
            let mut max_y = i32::MIN;

            for i in 0..num_pixels {
                let offset = 3 + i * 5;
                let x = data[offset] as i32;
                let y = data[offset + 1] as i32;
                let r = data[offset + 2];
                let g = data[offset + 3];
                let b = data[offset + 4];

                pixels.set_pixel(Point::new(x, y), Rgb888::new(r, g, b));

                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }

            if min_x != i32::MAX && min_x >= 0 && max_x >= 0 && min_y >= 0 && max_y >= 0 {
                pixels.mark_dirty_region(
                    min_x as usize,
                    min_y as usize,
                    max_x as usize,
                    max_y as usize,
                );
            }

            drop(pixels);
            graphics_buffer.send();
            Ok(required_len)
        }
        CMD_FILL => {
            if data.len() < 5 {
                return Err(ParseError::NeedMoreData);
            }

            let color = Rgb888::new(data[2], data[3], data[4]);

            let mut pixels = graphics_buffer.pixels_mut().await;
            pixels.fill(color);
            pixels.mark_all_dirty();
            drop(pixels);

            graphics_buffer.send();
            Ok(5)
        }
        CMD_PING | CMD_PONG => {
            // Caller should handle PING/PONG responses if needed
            Ok(2) // Consumed 2 bytes: version + command
        }
        _ => Err(ParseError::Invalid("Unknown command")),
    }
}
