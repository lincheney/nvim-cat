use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct SynAttr {
    pub fg: String,
    pub bg: String,
    pub bold: &'static str,
    pub reverse: &'static str,
    pub italic: &'static str,
    pub underline: &'static str,
}

const BOLD: &'static str = "1";
const NOBOLD: &'static str = "21";
const REVERSE: &'static str = "7";
const NOREVERSE: &'static str = "27";
const ITALIC: &'static str = "3";
const NOITALIC: &'static str = "23";
const UNDERLINE: &'static str = "4";
const NOUNDERLINE: &'static str = "24";
const NOFG: &'static str = "39";
const NOBG: &'static str = "49";

lazy_static! {
    static ref COLOUR_MAP: HashMap<&'static str, u8> = {
        let mut m = HashMap::new();
        m.insert("black", 0);
        m.insert("darkblue", 4);
        m.insert("darkgreen", 2);
        m.insert("darkcyan", 6);
        m.insert("darkred", 1);
        m.insert("darkmagenta", 5);
        m.insert("darkyellow", 3);
        m.insert("brown", 3);
        m.insert("lightgray", 7);
        m.insert("lightgrey", 7);
        m.insert("gray", 7);
        m.insert("grey", 7);
        m.insert("darkgray", 8);
        m.insert("darkgrey", 8);
        m.insert("blue", 12);
        m.insert("lightblue", 12);
        m.insert("green", 10);
        m.insert("lightgreen", 10);
        m.insert("cyan", 14);
        m.insert("lightcyan", 14);
        m.insert("red", 9);
        m.insert("lightred", 9);
        m.insert("magenta", 13);
        m.insert("lightmagenta", 13);
        m.insert("yellow", 11);
        m.insert("lightyellow", 11);
        m.insert("white", 15);
        m
    };
}

pub fn default_attr() -> SynAttr {
    SynAttr{
        fg: NOFG.to_string(),
        bg: NOBG.to_string(),
        bold: NOBOLD,
        reverse: NOREVERSE,
        italic: NOITALIC,
        underline: NOUNDERLINE,
    }
}

fn parse_colour(string: &str) -> Option<String> {
    if string.is_empty() { return None; }

    if string.chars().next() == Some('#') {
        // rgb
        let i = i64::from_str_radix(&string[1..], 16).expect("expected a hex string");
        return Some(format!("2;{};{};{}", i>>16, (i>>8)&0xff, i&0xff));
    }

    let string = string.to_ascii_lowercase();
    let num = string.parse::<u8>().ok()
        .or(COLOUR_MAP.get(&string[..]).copied());
    num.map(|i| format!("5;{}", i))
}


impl SynAttr {
    pub fn new(fg: &str, bg: &str, bold: &str, reverse: &str, italic: &str, underline: &str) -> Self {
        let fg = parse_colour(fg);
        let bg = parse_colour(bg);

        SynAttr{
            fg: if let Some(fg) = fg { format!("38;{}", fg) } else { NOFG.to_string() },
            bg: if let Some(bg) = bg { format!("48;{}", bg) } else { NOBG.to_string() },
            bold: if bold.is_empty() { NOBOLD } else { BOLD },
            reverse: if reverse.is_empty() { NOREVERSE } else { REVERSE },
            italic: if italic.is_empty() { NOITALIC } else { ITALIC },
            underline: if underline.is_empty() { NOUNDERLINE } else { UNDERLINE },
        }
    }
}
