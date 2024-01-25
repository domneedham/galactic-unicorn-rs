pub mod colors {
    use core::str::FromStr;

    use embedded_graphics_core::pixelcolor::{Rgb888, RgbColor, WebColors};
    use heapless::String;

    pub trait Rgb888Str {
        fn from_str(text: &str) -> Option<Rgb888>;
    }

    impl Rgb888Str for Rgb888 {
        fn from_str(text: &str) -> Option<Rgb888> {
            let mut heapless_text: String<32> = match heapless::String::from_str(text) {
                Ok(t) => t,
                Err(_) => return None,
            };

            heapless_text.make_ascii_uppercase();

            match heapless_text.as_str() {
                "RED" => Some(Rgb888::RED),
                "BLUE" => Some(Rgb888::BLUE),
                "GREEN" => Some(Rgb888::GREEN),
                "ORANGE" => Some(Rgb888::CSS_ORANGE),
                "YELLOW" => Some(Rgb888::YELLOW),
                "PURPLE" => Some(Rgb888::CSS_PURPLE),
                "PINK" => Some(Rgb888::CSS_PINK),
                "WHITE" => Some(Rgb888::WHITE),
                "CYAN" => Some(Rgb888::CYAN),
                "GOLD" => Some(Rgb888::CSS_GOLD),
                _ => None,
            }
        }
    }
}
