use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, RgbColor},
};
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

fn draw_number_todo(mut num: u32) -> [u8; 10] {
    // Initialize an array to hold the ASCII representations of the digits
    let mut digits_ascii = [b'0'; 10];
    let mut index = 9; // Start from the least significant digit

    // Special case for 0
    if num == 0 {
        digits_ascii[9] = b'0';
        return digits_ascii;
    }

    // Extract digits and convert to ASCII
    while num > 0 {
        let digit = (num % 10) as u8; // Extract least significant digit
        digits_ascii[index] = digit + b'0'; // Convert to ASCII
        num /= 10; // Shift rightward to get next digit
        if index == 0 {
            break; // Stop when the last digit is reached
        }
        index -= 1; // Move to the next index
    }

    digits_ascii
}

pub fn draw_number(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, num: u32, start: u32) {
    match num {
        1 => draw_one(gr, start),
        _ => draw_one(gr, start),
    }
}

pub fn draw_one(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == start + 1 {
                match y {
                    2..=3 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    9..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    _ => {}
                }
            } else if x == start + 2 {
                match y {
                    1..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    _ => {}
                }
            } else if x == start + 3 {
                match y {
                    0..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    _ => {}
                }
            } else {
                match y {
                    9..=11 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    _ => {}
                }
            }
        }
    }
}

fn get_point(x: u32, y: u32) -> Point {
    Point {
        x: x as i32,
        y: y as i32,
    }
}
