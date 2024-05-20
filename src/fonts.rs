use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, RgbColor, WebColors},
};
use galactic_unicorn_embassy::{HEIGHT, WIDTH};
use unicorn_graphics::UnicornGraphics;

pub fn draw_str(
    gr: &mut UnicornGraphics<WIDTH, HEIGHT>,
    text: &str,
    mut start: u32,
    color: Rgb888,
) {
    for character in text.chars() {
        draw_char(gr, character, start, color);
        start += 7;
    }
}

pub fn draw_char(
    gr: &mut UnicornGraphics<WIDTH, HEIGHT>,
    character: char,
    start: u32,
    color: Rgb888,
) {
    match character {
        '0' => draw_zero(gr, start, color),
        '1' => draw_one(gr, start, color),
        '2' => draw_two(gr, start, color),
        '3' => draw_three(gr, start, color),
        '4' => draw_four(gr, start, color),
        '5' => draw_five(gr, start, color),
        '6' => draw_six(gr, start, color),
        '7' => draw_seven(gr, start, color),
        '8' => draw_eight(gr, start, color),
        '9' => draw_nine(gr, start, color),
        _ => draw_eight(gr, start, color),
    }
}

pub fn draw_zero(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == end - 1 {
                match y {
                    1..=9 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 1 || x == end - 2 {
                gr.set_pixel(get_point(x, y), color);
            } else {
                if y <= 1 || y >= 9 {
                    gr.set_pixel(get_point(x, y), color);
                }
            }
        }
    }
}

pub fn draw_one(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == start + 1 {
                match y {
                    2..=3 => gr.set_pixel(get_point(x, y), color),
                    9..=11 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 2 {
                match y {
                    1..=11 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 3 {
                match y {
                    0..=11 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else {
                match y {
                    9..=11 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            }
        }
    }
}

pub fn draw_two(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if y == 0 {
                if x > start && x < start + 5 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 1 {
                gr.set_pixel(get_point(x, y), color);
            } else if y == 2 {
                if x < start + 2 || x > start + 3 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 3 || y == 4 {
                if x > start + 3 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 5 {
                if x > start + 1 && x < start + 5 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 6 {
                if x > start && x < start + 4 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 7 {
                if x < start + 3 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if y == 8 {
                if x < start + 2 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else {
                gr.set_pixel(get_point(x, y), color);
            }
        }
    }
}

pub fn draw_three(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    1 | 2 | 8 | 9 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    0 | 1 | 2 | 5 | 8 | 9 | 10 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == end - 1 {
                if y != 0 && y != 10 {
                    gr.set_pixel(get_point(x, y), color);
                }
            } else if x == end - 2 {
                gr.set_pixel(get_point(x, y), color);
            } else {
                if y <= 1 || y >= 9 {
                    gr.set_pixel(get_point(x, y), color);
                } else if y == 5 {
                    gr.set_pixel(get_point(x, y), color);
                }
            }
        }
    }
}

pub fn draw_four(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    4..=7 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    3..=7 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 2 {
                match y {
                    2 | 3 | 6 | 7 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 3 {
                match y {
                    1 | 2 | 6 | 7 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else {
                gr.set_pixel(get_point(x, y), color);
            }
        }
    }
}

pub fn draw_five(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    0..=4 => gr.set_pixel(get_point(x, y), color),
                    7..=9 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    6 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 2 || x == start + 3 {
                match y {
                    0 | 1 | 4 | 5 | 9 | 10 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 4 {
                match y {
                    2 | 3 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else {
                match y {
                    2 | 3 | 4 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            }
        }
    }
}

pub fn draw_six(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    1..=9 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 2 || x == start + 3 {
                match y {
                    0 | 1 | 4 | 5 | 9 | 10 => gr.set_pixel(get_point(x, y), color),
                    _ => {}
                }
            } else if x == start + 4 {
                match y {
                    3 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else {
                match y {
                    0 | 3 | 4 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            }
        }
    }
}

pub fn draw_seven(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        gr.set_pixel(get_point(x, 0), color);
        gr.set_pixel(get_point(x, 1), color);

        for y in 0..11 {
            if x == start + 5 {
                gr.set_pixel(get_point(x, 2), color);
            } else if x == start + 4 {
                gr.set_pixel(get_point(x, 2), color);
                gr.set_pixel(get_point(x, 3), color);
            } else if x == start + 3 {
                match y {
                    0..=2 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 2 {
                match y {
                    0..=3 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            }
        }
    }
}

pub fn draw_eight(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == start + 5 {
                match y {
                    0 | 5 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 1 || x == start + 4 {
                gr.set_pixel(get_point(x, y), color);
            } else if x == start + 2 || x == start + 3 {
                match y {
                    2 | 3 | 7 | 8 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            }
        }
    }
}

pub fn draw_nine(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32, color: Rgb888) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    0 | 5..=7 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 1 {
                match y {
                    6 | 7 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 2 || x == start + 3 {
                match y {
                    2 | 3 | 6..=8 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
                }
            } else if x == start + 4 {
                gr.set_pixel(get_point(x, y), color);
            } else if x == start + 5 {
                match y {
                    0 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), color),
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
