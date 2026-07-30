//! Text measurement and encoding for the base-14 fonts.
//!
//! Elrond uses Helvetica and Helvetica-Bold, which every PDF viewer provides
//! without embedding. That keeps generated pages small and, more importantly,
//! keeps output byte-identical between builds: an embedded font subset would vary
//! with the glyphs used.

/// Advance widths for Helvetica, characters 32 through 126, in 1/1000 em.
///
/// The standard Adobe metrics. Needed to centre and wrap text, since a PDF has no
/// layout engine of its own.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Advance widths for Helvetica-Bold, characters 32 through 126.
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Which of the two faces to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Body text.
    Regular,
    /// Headings and emphasis.
    Bold,
}

impl Face {
    /// The PDF resource name this face is registered under.
    pub const fn resource(self) -> &'static str {
        match self {
            Self::Regular => "ElrondR",
            Self::Bold => "ElrondB",
        }
    }

    /// The base-14 font name.
    pub const fn base_font(self) -> &'static str {
        match self {
            Self::Regular => "Helvetica",
            Self::Bold => "Helvetica-Bold",
        }
    }

    /// Advance width of one WinAnsi byte, in 1/1000 em.
    fn advance(self, byte: u8) -> u16 {
        let table = match self {
            Self::Regular => &HELVETICA,
            Self::Bold => &HELVETICA_BOLD,
        };
        // Outside the measured range, fall back to the width of a lowercase "n",
        // which is close to the average for Latin-1 accented letters.
        match byte {
            0x20..=0x7e => table[usize::from(byte) - 32],
            _ => table[usize::from(b'n') - 32],
        }
    }
}

/// Width of `text` at `size` points.
pub fn width_of(text: &str, face: Face, size: f32) -> f32 {
    let encoded = to_win_ansi(text);
    let thousandths: u32 = encoded
        .iter()
        .map(|byte| u32::from(face.advance(*byte)))
        .sum();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a string long enough to lose precision here would not fit on any page"
    )]
    let width = thousandths as f32;
    width * size / 1000.0
}

/// Breaks `text` into lines that fit within `max_width`.
///
/// Wraps on spaces; a single word longer than the line is broken mid-word rather
/// than allowed to overflow the page.
pub fn wrap(text: &str, face: Face, size: f32, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };

        if width_of(&candidate, face, size) <= max_width {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }

        // The word alone may still be too wide.
        if width_of(word, face, size) <= max_width {
            word.clone_into(&mut current);
        } else {
            let mut chunk = String::new();
            for character in word.chars() {
                let mut probe = chunk.clone();
                probe.push(character);
                if width_of(&probe, face, size) > max_width && !chunk.is_empty() {
                    lines.push(std::mem::take(&mut chunk));
                }
                chunk.push(character);
            }
            current = chunk;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Encodes text as WinAnsi bytes.
///
/// WinAnsi covers Latin-1 plus the common typographic characters, which is what
/// the base-14 fonts provide. Anything outside it becomes a question mark rather
/// than a broken glyph or a corrupt stream.
pub fn to_win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| match character {
            // Directly representable.
            '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => {
                u8::try_from(u32::from(character)).unwrap_or(b'?')
            }
            // The WinAnsi-specific block at 0x80-0x9f.
            '\u{20ac}' => 0x80,
            '\u{201a}' => 0x82,
            '\u{0192}' => 0x83,
            '\u{201e}' => 0x84,
            '\u{2026}' => 0x85,
            '\u{2020}' => 0x86,
            '\u{2021}' => 0x87,
            '\u{02c6}' => 0x88,
            '\u{2030}' => 0x89,
            '\u{0160}' => 0x8a,
            '\u{2039}' => 0x8b,
            '\u{0152}' => 0x8c,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201c}' => 0x93,
            '\u{201d}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{2122}' => 0x99,
            '\u{0161}' => 0x9a,
            '\u{203a}' => 0x9b,
            '\u{0153}' => 0x9c,
            '\u{0178}' => 0x9f,
            // Tabs and newlines have no meaning inside a PDF text string.
            '\t' | '\n' | '\r' => b' ',
            _ => b'?',
        })
        .collect()
}

/// Escapes a byte string for use as a PDF literal string.
///
/// Backslashes and unbalanced parentheses would otherwise terminate the string
/// early and corrupt the content stream.
pub fn escape_literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 8);
    for byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(*byte);
            }
            _ => out.push(*byte),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_grows_with_length_and_size() {
        let short = width_of("Policy", Face::Regular, 12.0);
        let long = width_of("Retention Policy", Face::Regular, 12.0);
        assert!(long > short);
        assert!(width_of("Policy", Face::Regular, 24.0) > short);
    }

    #[test]
    fn bold_is_wider_than_regular_for_the_same_text() {
        assert!(
            width_of("Retention", Face::Bold, 12.0) > width_of("Retention", Face::Regular, 12.0)
        );
    }

    #[test]
    fn an_empty_string_has_no_width() {
        assert!(width_of("", Face::Regular, 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_space_is_the_documented_helvetica_width() {
        // 278/1000 em at 1000pt is exactly 278pt. A wrong table would show up here
        // before it showed up as slightly-off centring on a cover.
        assert!((width_of(" ", Face::Regular, 1000.0) - 278.0).abs() < 0.01);
    }

    #[test]
    fn wrapping_respects_the_measure() {
        let text = "The committee resolved to adopt the retention schedule without amendment";
        let lines = wrap(text, Face::Regular, 12.0, 200.0);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(
                width_of(line, Face::Regular, 12.0) <= 200.0,
                "line overflows: {line:?}"
            );
        }
        // No words are lost or duplicated by wrapping.
        assert_eq!(
            lines.join(" ").split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_overlong_word_is_broken_rather_than_overflowing() {
        let lines = wrap(&"A".repeat(200), Face::Regular, 12.0, 100.0);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(width_of(line, Face::Regular, 12.0) <= 100.0);
        }
    }

    #[test]
    fn wrapping_always_yields_at_least_one_line() {
        assert_eq!(wrap("", Face::Regular, 12.0, 100.0), vec![String::new()]);
        assert_eq!(wrap("   ", Face::Regular, 12.0, 100.0), vec![String::new()]);
    }

    #[test]
    fn ascii_encodes_unchanged() {
        assert_eq!(to_win_ansi("Policy 2026"), b"Policy 2026".to_vec());
    }

    #[test]
    fn accented_latin_survives() {
        // Latin-1 maps straight through, so "Résumé" keeps its accents.
        assert_eq!(
            to_win_ansi("Résumé"),
            vec![b'R', 0xe9, b's', b'u', b'm', 0xe9]
        );
    }

    #[test]
    fn typographic_punctuation_maps_into_the_winansi_block() {
        assert_eq!(to_win_ansi("\u{2019}"), vec![0x92]);
        assert_eq!(to_win_ansi("\u{2014}"), vec![0x97]);
        assert_eq!(to_win_ansi("\u{2026}"), vec![0x85]);
    }

    #[test]
    fn unrepresentable_characters_degrade_to_a_question_mark() {
        // A CJK glyph has no place in a base-14 font; a placeholder is better than
        // a broken stream.
        assert_eq!(to_win_ansi("\u{6587}"), vec![b'?']);
    }

    #[test]
    fn newlines_become_spaces_rather_than_breaking_the_stream() {
        assert_eq!(to_win_ansi("a\nb\tc"), b"a b c".to_vec());
    }

    #[test]
    fn literal_escaping_protects_the_content_stream() {
        // An unescaped ")" would close the string early and turn the rest of the
        // title into PDF operators.
        assert_eq!(escape_literal(b"a(b)c"), b"a\\(b\\)c".to_vec());
        assert_eq!(escape_literal(br"back\slash"), br"back\\slash".to_vec());
        assert_eq!(escape_literal(b"plain"), b"plain".to_vec());
    }
}
