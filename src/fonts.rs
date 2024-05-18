use embedded_graphics::{
    geometry::Point,
    pixelcolor::{Rgb888, RgbColor, WebColors},
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
        '0' => draw_zero(gr, start),
        '1' => draw_one(gr, start),
        '2' => draw_two(gr, start),
        '3' => draw_three(gr, start),
        '4' => draw_four(gr, start),
        '5' => draw_five(gr, start),
        '6' => draw_six(gr, start),
        '7' => draw_seven(gr, start),
        '8' => draw_eight(gr, start),
        _ => draw_eight(gr, start),
    }
}

pub fn draw_zero(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == end - 1 {
                match y {
                    1..=9 => gr.set_pixel(get_point(x, y), Rgb888::CSS_PURPLE),
                    _ => {}
                }
            } else if x == start + 1 || x == end - 2 {
                gr.set_pixel(get_point(x, y), Rgb888::CSS_PURPLE);
            } else {
                if y <= 1 || y >= 9 {
                    gr.set_pixel(get_point(x, y), Rgb888::CSS_PURPLE);
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

pub fn draw_two(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if y == 0 {
                if x > start && x < start + 5 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 1 {
                gr.set_pixel(get_point(x, y), Rgb888::BLUE);
            } else if y == 2 {
                if x < start + 2 || x > start + 3 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 3 || y == 4 {
                if x > start + 3 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 5 {
                if x > start + 1 && x < start + 5 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 6 {
                if x > start && x < start + 4 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 7 {
                if x < start + 3 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else if y == 8 {
                if x < start + 2 {
                    gr.set_pixel(get_point(x, y), Rgb888::BLUE);
                }
            } else {
                gr.set_pixel(get_point(x, y), Rgb888::BLUE);
            }
        }
    }
}

pub fn draw_three(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    1 | 2 | 8 | 9 => gr.set_pixel(get_point(x, y), Rgb888::YELLOW),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    0 | 1 | 2 | 5 | 8 | 9 | 10 => gr.set_pixel(get_point(x, y), Rgb888::YELLOW),
                    _ => {}
                }
            } else if x == end - 1 {
                if y != 0 && y != 10 {
                    gr.set_pixel(get_point(x, y), Rgb888::YELLOW);
                }
            } else if x == end - 2 {
                gr.set_pixel(get_point(x, y), Rgb888::YELLOW);
            } else {
                if y <= 1 || y >= 9 {
                    gr.set_pixel(get_point(x, y), Rgb888::YELLOW);
                } else if y == 5 {
                    gr.set_pixel(get_point(x, y), Rgb888::YELLOW);
                }
            }
        }
    }
}

pub fn draw_four(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    4..=7 => gr.set_pixel(get_point(x, y), Rgb888::GREEN),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    3..=7 => gr.set_pixel(get_point(x, y), Rgb888::GREEN),
                    _ => {}
                }
            } else if x == start + 2 {
                match y {
                    2 | 3 | 6 | 7 => gr.set_pixel(get_point(x, y), Rgb888::GREEN),
                    _ => {}
                }
            } else if x == start + 3 {
                match y {
                    1 | 2 | 6 | 7 => gr.set_pixel(get_point(x, y), Rgb888::GREEN),
                    _ => {}
                }
            } else {
                gr.set_pixel(get_point(x, y), Rgb888::GREEN);
            }
        }
    }
}

pub fn draw_five(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    0..=4 => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                    7..=9 => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    6 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                }
            } else if x == start + 2 || x == start + 3 {
                match y {
                    0 | 1 | 4 | 5 | 9 | 10 => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                    _ => {}
                }
            } else if x == start + 4 {
                match y {
                    2 | 3 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                }
            } else {
                match y {
                    2 | 3 | 4 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_PINK),
                }
            }
        }
    }
}

pub fn draw_six(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start {
                match y {
                    1..=9 => gr.set_pixel(get_point(x, y), Rgb888::CSS_AQUA),
                    _ => {}
                }
            } else if x == start + 1 {
                match y {
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_AQUA),
                }
            } else if x == start + 2 || x == start + 3 {
                match y {
                    0 | 1 | 4 | 5 | 9 | 10 => gr.set_pixel(get_point(x, y), Rgb888::CSS_AQUA),
                    _ => {}
                }
            } else if x == start + 4 {
                match y {
                    3 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_AQUA),
                }
            } else {
                match y {
                    0 | 3 | 4 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_AQUA),
                }
            }
        }
    }
}

pub fn draw_seven(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        gr.set_pixel(get_point(x, 0), Rgb888::CSS_DARK_KHAKI);
        gr.set_pixel(get_point(x, 1), Rgb888::CSS_DARK_KHAKI);

        for y in 0..11 {
            if x == start + 5 {
                gr.set_pixel(get_point(x, 2), Rgb888::CSS_DARK_KHAKI);
            } else if x == start + 4 {
                gr.set_pixel(get_point(x, 2), Rgb888::CSS_DARK_KHAKI);
                gr.set_pixel(get_point(x, 3), Rgb888::CSS_DARK_KHAKI);
            } else if x == start + 3 {
                match y {
                    0..=2 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_DARK_KHAKI),
                }
            } else if x == start + 2 {
                match y {
                    0..=3 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_DARK_KHAKI),
                }
            }
        }
    }
}

pub fn draw_eight(gr: &mut UnicornGraphics<WIDTH, HEIGHT>, start: u32) {
    let end = start + 6;
    for x in start..end {
        for y in 0..11 {
            if x == start || x == start + 5 {
                match y {
                    0 | 5 | 10 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_GOLD),
                }
            } else if x == start + 1 || x == start + 4 {
                gr.set_pixel(get_point(x, y), Rgb888::CSS_GOLD);
            } else if x == start + 2 || x == start + 3 {
                match y {
                    2 | 3 | 7 | 8 => {}
                    _ => gr.set_pixel(get_point(x, y), Rgb888::CSS_GOLD),
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
