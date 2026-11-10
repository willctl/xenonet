//! Ids and names in a reader's order, as ICU's root collation orders
//! them: letters without case first, then accents, then case (lowercase
//! first); white space and punctuation before digits before letters. Past
//! ASCII and the accented Latin letters, characters go by their code
//! points.

// Where an ASCII character falls in the root collation (CLDR's, which
// ICU uses): white space, punctuation and symbols in
// their own order, then digits, then letters without case; its case
// second.
// Latin letters with a mark, as the letter they're made on (NFD's first).
const MARKED: &[(char, char)] = &[
    ('À', 'A'),
    ('Á', 'A'),
    ('Â', 'A'),
    ('Ã', 'A'),
    ('Ä', 'A'),
    ('Å', 'A'),
    ('Ç', 'C'),
    ('È', 'E'),
    ('É', 'E'),
    ('Ê', 'E'),
    ('Ë', 'E'),
    ('Ì', 'I'),
    ('Í', 'I'),
    ('Î', 'I'),
    ('Ï', 'I'),
    ('Ñ', 'N'),
    ('Ò', 'O'),
    ('Ó', 'O'),
    ('Ô', 'O'),
    ('Õ', 'O'),
    ('Ö', 'O'),
    ('Ù', 'U'),
    ('Ú', 'U'),
    ('Û', 'U'),
    ('Ü', 'U'),
    ('Ý', 'Y'),
    ('à', 'a'),
    ('á', 'a'),
    ('â', 'a'),
    ('ã', 'a'),
    ('ä', 'a'),
    ('å', 'a'),
    ('ç', 'c'),
    ('è', 'e'),
    ('é', 'e'),
    ('ê', 'e'),
    ('ë', 'e'),
    ('ì', 'i'),
    ('í', 'i'),
    ('î', 'i'),
    ('ï', 'i'),
    ('ñ', 'n'),
    ('ò', 'o'),
    ('ó', 'o'),
    ('ô', 'o'),
    ('õ', 'o'),
    ('ö', 'o'),
    ('ù', 'u'),
    ('ú', 'u'),
    ('û', 'u'),
    ('ü', 'u'),
    ('ý', 'y'),
    ('ÿ', 'y'),
    ('Ā', 'A'),
    ('ā', 'a'),
    ('Ă', 'A'),
    ('ă', 'a'),
    ('Ą', 'A'),
    ('ą', 'a'),
    ('Ć', 'C'),
    ('ć', 'c'),
    ('Ĉ', 'C'),
    ('ĉ', 'c'),
    ('Ċ', 'C'),
    ('ċ', 'c'),
    ('Č', 'C'),
    ('č', 'c'),
    ('Ď', 'D'),
    ('ď', 'd'),
    ('Ē', 'E'),
    ('ē', 'e'),
    ('Ĕ', 'E'),
    ('ĕ', 'e'),
    ('Ė', 'E'),
    ('ė', 'e'),
    ('Ę', 'E'),
    ('ę', 'e'),
    ('Ě', 'E'),
    ('ě', 'e'),
    ('Ĝ', 'G'),
    ('ĝ', 'g'),
    ('Ğ', 'G'),
    ('ğ', 'g'),
    ('Ġ', 'G'),
    ('ġ', 'g'),
    ('Ģ', 'G'),
    ('ģ', 'g'),
    ('Ĥ', 'H'),
    ('ĥ', 'h'),
    ('Ĩ', 'I'),
    ('ĩ', 'i'),
    ('Ī', 'I'),
    ('ī', 'i'),
    ('Ĭ', 'I'),
    ('ĭ', 'i'),
    ('Į', 'I'),
    ('į', 'i'),
    ('İ', 'I'),
    ('Ĵ', 'J'),
    ('ĵ', 'j'),
    ('Ķ', 'K'),
    ('ķ', 'k'),
    ('Ĺ', 'L'),
    ('ĺ', 'l'),
    ('Ļ', 'L'),
    ('ļ', 'l'),
    ('Ľ', 'L'),
    ('ľ', 'l'),
    ('Ń', 'N'),
    ('ń', 'n'),
    ('Ņ', 'N'),
    ('ņ', 'n'),
    ('Ň', 'N'),
    ('ň', 'n'),
    ('Ō', 'O'),
    ('ō', 'o'),
    ('Ŏ', 'O'),
    ('ŏ', 'o'),
    ('Ő', 'O'),
    ('ő', 'o'),
    ('Ŕ', 'R'),
    ('ŕ', 'r'),
    ('Ŗ', 'R'),
    ('ŗ', 'r'),
    ('Ř', 'R'),
    ('ř', 'r'),
    ('Ś', 'S'),
    ('ś', 's'),
    ('Ŝ', 'S'),
    ('ŝ', 's'),
    ('Ş', 'S'),
    ('ş', 's'),
    ('Š', 'S'),
    ('š', 's'),
    ('Ţ', 'T'),
    ('ţ', 't'),
    ('Ť', 'T'),
    ('ť', 't'),
    ('Ũ', 'U'),
    ('ũ', 'u'),
    ('Ū', 'U'),
    ('ū', 'u'),
    ('Ŭ', 'U'),
    ('ŭ', 'u'),
    ('Ů', 'U'),
    ('ů', 'u'),
    ('Ű', 'U'),
    ('ű', 'u'),
    ('Ų', 'U'),
    ('ų', 'u'),
    ('Ŵ', 'W'),
    ('ŵ', 'w'),
    ('Ŷ', 'Y'),
    ('ŷ', 'y'),
    ('Ÿ', 'Y'),
    ('Ź', 'Z'),
    ('ź', 'z'),
    ('Ż', 'Z'),
    ('ż', 'z'),
    ('Ž', 'Z'),
    ('ž', 'z'),
];

fn collation(c: char) -> (u32, u32, u32, u32) {
    const ORDER: &str = "\t\n\x0b\x0c\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789";
    let (base, marked) = match MARKED.iter().find(|(m, _)| *m == c) {
        Some((_, b)) => (*b, c as u32),
        None => (c, 0),
    };
    if base.is_ascii_alphabetic() {
        let upper = base.is_ascii_uppercase() || (marked != 0 && c.is_uppercase());
        return (1, 100 + (base.to_ascii_lowercase() as u32 - 'a' as u32), marked, u32::from(upper));
    }
    match ORDER.find(c) {
        Some(i) => (1, i as u32, 0, 0),
        None => (2, c as u32, 0, 0),
    }
}

/// Two ids or names in a reader's order.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let ka: Vec<(u32, u32, u32, u32)> = a.chars().map(collation).collect();
    let kb: Vec<(u32, u32, u32, u32)> = b.chars().map(collation).collect();
    let primary = |k: &[(u32, u32, u32, u32)]| k.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>();
    primary(&ka).cmp(&primary(&kb)).then_with(|| ka.iter().map(|x| x.2).cmp(kb.iter().map(|x| x.2))).then_with(|| ka.iter().map(|x| x.3).cmp(kb.iter().map(|x| x.3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn in_a_readers_order() {
        assert_eq!(compare("a", "B"), Ordering::Less);
        assert_eq!(compare("b", "B"), Ordering::Less);
        assert_eq!(compare("e", "é"), Ordering::Less);
        assert_eq!(compare("_x", "1"), Ordering::Less);
        assert_eq!(compare("9", "a"), Ordering::Less);
    }
}
