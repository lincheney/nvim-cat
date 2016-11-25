pub enum Attr {
    BOLD = 1,
    ITALIC = 2,
    UNDERLINE = 4,
    REVERSE = 8,
}

#[derive(Debug, Clone)]
pub struct Highlight {
    pub fg: String,
    pub bg: String,
    pub attrs: u8,
}

impl Highlight {
    pub fn new() -> Self {
        Highlight{ fg: "255;255;255".to_string(), bg: "0;0;0".to_string(), attrs: 0 }
    }

    pub fn to_string(&self) -> String {
        format!("\x1b[0;{bold}{italic}{underline}38;2;{fg};48;2;{bg}m",
            bold = if self.attrs & (Attr::BOLD as u8) != 0 { "1;" } else { "" },
            italic = if self.attrs & (Attr::ITALIC as u8) != 0 { "3;" } else { "" },
            underline = if self.attrs & (Attr::UNDERLINE as u8) != 0 { "4;" } else { "" },
            fg = if self.attrs & (Attr::REVERSE as u8) == 0 { &self.fg } else { &self.bg },
            bg = if self.attrs & (Attr::REVERSE as u8) == 0 { &self.bg } else { &self.fg },
        )
    }

}

pub fn rgb_to_string(val: u32) -> String {
    format!("{};{};{}",
        val >> 16,
        (val & 0x00ff00) >> 8,
        val & 0xff,
    )
}
