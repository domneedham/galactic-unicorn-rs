use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, RgbColor},
};
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

pub fn draw_str(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, text: &str, mut start: u32) {
    for character in text.chars() {
        draw_char(gr, character, start);
        start += 7;
    }
}

pub fn draw_char(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, character: char, start: u32) {
    match character {
        '1' => draw_one(gr, start),
        _ => draw_one(gr, start),
    }
}

pub fn draw_zero(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == end {
                match y {
                    1..=10 => gr.set_pixel(get_point(x, y), Rgb888::RED),
                    _ => {}
                }
            } else {
                if y == 0 || y == 11 {
                    gr.set_pixel(get_point(x, y), Rgb888::RED);
                }
            }
        }
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
