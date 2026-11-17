//! A tiny built-in bitmap font for the tray digits: 3×5 for small icons, 5×7 for larger ones.
//! Each glyph is a list of rows; in each row the leftmost pixel is the highest bit.

pub struct Font {
    pub width: u32,
    pub height: u32,
    glyphs: &'static [(char, &'static [u8])],
}

impl Font {
    /// The rows of a glyph, or `None` for characters the font does not have.
    pub fn glyph(&self, c: char) -> Option<&'static [u8]> {
        self.glyphs.iter().find(|(g, _)| *g == c).map(|(_, rows)| *rows)
    }

    /// Whether the pixel at (x, y) of glyph `c` is set.
    pub fn pixel(&self, c: char, x: u32, y: u32) -> bool {
        self.glyph(c).is_some_and(|rows| rows[y as usize] >> (self.width - 1 - x) & 1 == 1)
    }
}

pub const SMALL: Font = Font {
    width: 3,
    height: 5,
    glyphs: &[
        ('0', &[0b111, 0b101, 0b101, 0b101, 0b111]),
        ('1', &[0b010, 0b110, 0b010, 0b010, 0b111]),
        ('2', &[0b111, 0b001, 0b111, 0b100, 0b111]),
        ('3', &[0b111, 0b001, 0b111, 0b001, 0b111]),
        ('4', &[0b101, 0b101, 0b111, 0b001, 0b001]),
        ('5', &[0b111, 0b100, 0b111, 0b001, 0b111]),
        ('6', &[0b111, 0b100, 0b111, 0b101, 0b111]),
        ('7', &[0b111, 0b001, 0b001, 0b001, 0b001]),
        ('8', &[0b111, 0b101, 0b111, 0b101, 0b111]),
        ('9', &[0b111, 0b101, 0b111, 0b001, 0b111]),
        ('!', &[0b010, 0b010, 0b010, 0b000, 0b010]),
        ('-', &[0b000, 0b000, 0b111, 0b000, 0b000]),
    ],
};

pub const LARGE: Font = Font {
    width: 5,
    height: 7,
    glyphs: &[
        ('0', &[0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
        ('1', &[0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
        ('2', &[0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
        ('3', &[0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110]),
        ('4', &[0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
        ('5', &[0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
        ('6', &[0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
        ('7', &[0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
        ('8', &[0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
        ('9', &[0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
        ('!', &[0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100]),
        ('-', &[0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000]),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_fits_its_font() {
        for font in [&SMALL, &LARGE] {
            for c in "0123456789!-".chars() {
                let rows = font.glyph(c).unwrap_or_else(|| panic!("missing {c}"));
                assert_eq!(rows.len() as u32, font.height);
                assert!(rows.iter().all(|r| u32::from(*r) < 1 << font.width), "{c} is too wide");
            }
        }
        assert!(SMALL.glyph('x').is_none());
    }

    #[test]
    fn reads_pixels_left_to_right() {
        // "1" in the small font: .#. / ##. / .#. / .#. / ###
        assert!(!SMALL.pixel('1', 0, 0) && SMALL.pixel('1', 1, 0) && !SMALL.pixel('1', 2, 0));
        assert!(SMALL.pixel('1', 0, 1) && SMALL.pixel('1', 1, 1));
        assert!((0..3).all(|x| SMALL.pixel('1', x, 4)));
    }
}
