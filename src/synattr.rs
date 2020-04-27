#[derive(Clone, Debug)]
pub struct SynAttr {
    pub fg: String,
    pub bg: String,
    pub bold: &'static str,
    pub reverse: &'static str,
    pub italic: &'static str,
    pub underline: &'static str,
}

const BOLD: &str = "1";
const NOBOLD: &str = "22";
const REVERSE: &str = "7";
const NOREVERSE: &str = "27";
const ITALIC: &str = "3";
const NOITALIC: &str = "23";
const UNDERLINE: &str = "4";
const NOUNDERLINE: &str = "24";
const NOFG: &str = "39";
const NOBG: &str = "49";

fn parse_colour(string: &str, truecolor: bool) -> Option<String> {
    if string.is_empty() { return None; }

    if string.starts_with('#') {
        // rgb
        let i = i64::from_str_radix(&string[1..], 16).expect("expected a hex string");
        return Some(format!("2;{};{};{}", i>>16, (i>>8)&0xff, i&0xff));
    }

    if let Ok(n) = string.parse::<u8>() {
        return Some(format!("5;{}", n));
    }

    let string = string.to_ascii_lowercase();
    if truecolor {
        ::color::TRUECOLOR_MAP.get(&string[..]).map(|(r, g, b)| format!("2;{};{};{}", r, g, b))
    } else {
        ::color::COLOR_MAP.get(&string[..]).map(|n| format!("5;{}", n))
    }
}


impl SynAttr {
    pub fn new(
        fg: &str,
        bg: &str,
        bold: &str,
        reverse: &str,
        italic: &str,
        underline: &str,
        default: &SynAttr,
        truecolor: bool,
    ) -> Self {
        let fg = parse_colour(fg, truecolor);
        let bg = parse_colour(bg, truecolor);

        SynAttr{
            fg: if let Some(fg) = fg { format!("38;{}", fg) } else { default.fg.to_string() },
            bg: if let Some(bg) = bg { format!("48;{}", bg) } else { default.bg.to_string() },
            bold: if !bold.is_empty() { BOLD } else { default.bold },
            reverse: if !reverse.is_empty() { REVERSE } else { default.reverse },
            italic: if !italic.is_empty() { ITALIC } else { default.italic },
            underline: if !underline.is_empty() { UNDERLINE } else { default.underline },
        }
    }
}

impl std::default::Default for SynAttr {
    fn default() -> Self {
        SynAttr{
            fg: NOFG.to_string(),
            bg: NOBG.to_string(),
            bold: NOBOLD,
            reverse: NOREVERSE,
            italic: NOITALIC,
            underline: NOUNDERLINE,
        }
    }
}
