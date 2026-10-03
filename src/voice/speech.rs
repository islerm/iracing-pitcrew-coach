//! Turns coaching text written for the screen into text a voice engine reads naturally.
//!
//! Numbers, units and track references are spoken the way a race engineer would say them
//! over team radio ("four tenths", "ninety-one ninety-six revs", "Turn twelve"). The
//! scanner is hand-rolled: it walks the characters, and only rewrites tokens that start at
//! a word boundary and match a known shape, so anything unknown passes through unchanged.

#[rustfmt::skip]
const ONES: [&str; 20] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// Rewrites `text` for speaking: markdown stripped, numbers and units spelled out, and every
/// line turned into a sentence so the voice pauses between them.
pub fn for_speech(text: &str) -> String {
    let mut sentences = Vec::new();
    for raw in text.lines() {
        let line = strip_markdown(raw);
        if !line.chars().any(char::is_alphanumeric) {
            continue;
        }
        let spoken = finish_sentence(&tidy(&scan(&replace_symbols(&line))));
        if !spoken.is_empty() {
            sentences.push(spoken);
        }
    }
    sentences.join(" ")
}

// ---- number words ----------------------------------------------------------------------

/// Words for 0..=99; `hyphen` selects "ninety-six" over "ninety six".
fn below_100(n: u32, hyphen: bool) -> String {
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    let tens = TENS[(n / 10) as usize];
    match n % 10 {
        0 => tens.to_string(),
        o => format!(
            "{tens}{}{}",
            if hyphen { '-' } else { ' ' },
            ONES[o as usize]
        ),
    }
}

/// Words for 0..=99999 ("three hundred forty six", "ten thousand two hundred").
fn words(n: u32) -> String {
    match n {
        0..=99 => below_100(n, false),
        100..=999 => {
            let hundreds = format!("{} hundred", ONES[(n / 100) as usize]);
            match n % 100 {
                0 => hundreds,
                r => format!("{hundreds} {}", below_100(r, false)),
            }
        }
        _ => {
            let thousands = format!("{} thousand", words(n / 1000));
            match n % 1000 {
                0 => thousands,
                r => format!("{thousands} {}", words(r)),
            }
        }
    }
}

/// Digit string to words, or `None` when it is too large (over 99999) to read sensibly.
fn int_words(digits: &str) -> Option<String> {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.len() > 5 {
        return None;
    }
    Some(words(trimmed.parse().unwrap_or(0)))
}

/// Integer or decimal digit string to words ("8.9" -> "eight point nine"); left as digits when
/// too large.
fn plain(num: &str) -> String {
    match num.split_once('.') {
        None => int_words(num).unwrap_or_else(|| num.to_string()),
        Some((int, frac)) => match int_words(int) {
            Some(int) => {
                let digits: Vec<&str> = frac
                    .chars()
                    .map(|c| ONES[c.to_digit(10).unwrap_or(0) as usize])
                    .collect();
                format!("{int} point {}", digits.join(" "))
            }
            None => num.to_string(),
        },
    }
}

/// Engine speed the way crews say it: "9196" -> "ninety-one ninety-six", "8500" -> "eighty-five
/// hundred". Falls back to plain number words for anything else.
fn rpm_words(num: &str) -> String {
    match num.parse::<u32>() {
        Ok(n) if num.len() == 4 && n % 1000 != 0 => {
            let (head, tail) = (n / 100, n % 100);
            let second = match tail {
                0 => "hundred".to_string(),
                1..=9 => format!("oh {}", ONES[tail as usize]),
                _ => below_100(tail, true),
            };
            format!("{} {second}", below_100(head, true))
        }
        _ => plain(num),
    }
}

/// Time delta the way engineers speak it: tenths under a second, seconds above.
fn delta_words(num: &str) -> String {
    let v: f64 = num.parse().unwrap_or(0.0);
    if v >= 1.0 {
        return if num == "1" {
            "one second".to_string()
        } else {
            format!("{} seconds", plain(num))
        };
    }
    if v < 0.05 {
        return "a few hundredths".to_string();
    }
    if v < 0.075 {
        return "half a tenth".to_string();
    }
    match (v * 10.0).round() as u32 {
        10 => "one second".to_string(),
        1 => "a tenth".to_string(),
        t => format!("{} tenths", words(t)),
    }
}

// ---- line-level cleanup ----------------------------------------------------------------

/// Removes markdown decoration: heading hashes, bullets, numbered-list markers, bold/code marks.
fn strip_markdown(raw: &str) -> String {
    let mut s = raw.trim().trim_start_matches('#').trim_start();
    for bullet in ["- ", "* ", "• ", "+ "] {
        if let Some(rest) = s.strip_prefix(bullet) {
            s = rest.trim_start();
            break;
        }
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = &s[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            s = rest.trim_start();
        }
    }
    s.replace("**", "")
        .replace("__", "")
        .replace(['*', '`'], "")
        .trim()
        .to_string()
}

/// Plain-text substitutions applied before scanning.
fn replace_symbols(s: &str) -> String {
    s.replace("e.g.", "for example")
        .replace("i.e.", "that is")
        .replace(['·', '—', '(', ')'], ", ")
        .replace(" – ", ", ")
        .replace(" - ", ", ")
        .replace("->", " to ")
        .replace('→', " to ")
        .replace('&', " and ")
}

/// Collapses whitespace and tidies punctuation: no space before it, no doubled commas, and
/// `:`/`;` become commas so the voice pauses instead of announcing a list.
fn tidy(s: &str) -> String {
    let is_punct = |c: char| matches!(c, ',' | ';' | ':' | '.' | '!' | '?');
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        // Punctuation inside a token ("1,200", "v1.2") is left alone.
        let terminal = is_punct(c)
            && chars
                .get(i + 1)
                .is_none_or(|n| n.is_whitespace() || is_punct(*n));
        if !terminal {
            out.push(c);
            continue;
        }
        while out.ends_with(' ') {
            out.pop();
        }
        let stop = matches!(c, '.' | '!' | '?');
        match out.chars().last() {
            None => {}
            Some('.' | '!' | '?') => {}
            Some(',') if stop => {
                out.pop();
                out.push(c);
            }
            Some(',') => {}
            Some(_) => out.push(if stop { c } else { ',' }),
        }
        if !out.is_empty() {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Makes sure a line ends with sentence punctuation.
fn finish_sentence(s: &str) -> String {
    let s = s.trim_end_matches([',', ';', ':', ' ']);
    if s.is_empty() {
        String::new()
    } else if s.ends_with(['.', '!', '?']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

// ---- token scanner ---------------------------------------------------------------------

/// Walks the text, rewriting recognised tokens that start at a word boundary.
fn scan(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let at_boundary = i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_');
        if !at_boundary {
            out.push(ch);
            i += 1;
            continue;
        }
        let rest = &c[i..];
        if let Some((text, used)) = lap_time(rest)
            .or_else(|| track_ref(rest))
            .or_else(|| quantity(rest, &out))
        {
            out.push_str(&text);
            i += used;
        } else if ch.is_alphanumeric() {
            let len = rest.iter().take_while(|x| x.is_alphanumeric()).count();
            let word: String = rest[..len].iter().collect();
            if word == "vs" {
                out.push_str("versus");
                // swallow the abbreviation dot of "vs."
                i += len + usize::from(rest.get(len) == Some(&'.'));
            } else {
                out.push_str(&word);
                i += len;
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

fn digits_len(c: &[char]) -> usize {
    c.iter().take_while(|x| x.is_ascii_digit()).count()
}

/// `M:SS` or `M:SS.fff` lap times, read as "two oh four point seven".
fn lap_time(c: &[char]) -> Option<(String, usize)> {
    let m_len = digits_len(c);
    if !(1..=2).contains(&m_len) || c.get(m_len) != Some(&':') {
        return None;
    }
    let sec = &c[m_len + 1..];
    if digits_len(sec) < 2 {
        return None;
    }
    let ss: u32 = sec[..2].iter().collect::<String>().parse().ok()?;
    let mins: u32 = c[..m_len].iter().collect::<String>().parse().ok()?;
    if ss >= 60 {
        return None;
    }
    let mut end = m_len + 3;
    let mut frac = String::new();
    if c.get(end) == Some(&'.') && c.get(end + 1).is_some_and(char::is_ascii_digit) {
        let n = digits_len(&c[end + 1..]);
        frac = c[end + 1..end + 1 + n].iter().collect();
        end += 1 + n;
    }
    if c.get(end).is_some_and(|x| x.is_alphanumeric() || *x == ':') {
        return None;
    }
    // Round to tenths (on the first two decimals), carrying into seconds and minutes.
    let digit = |i: usize| frac.chars().nth(i).and_then(|d| d.to_digit(10)).unwrap_or(0);
    let total_tenths = (mins * 60 + ss) * 10 + digit(0) + u32::from(digit(1) >= 5);
    let (mins, ss, tenths) = (total_tenths / 600, total_tenths / 10 % 60, total_tenths % 10);
    let mut text = if mins == 0 {
        words(ss)
    } else {
        let secs = match ss {
            0 => "oh oh".to_string(),
            1..=9 => format!("oh {}", ONES[ss as usize]),
            _ => words(ss),
        };
        format!("{} {secs}", words(mins))
    };
    if !frac.is_empty() {
        text = format!("{text} point {}", ONES[tenths as usize]);
    }
    Some((text, end))
}

/// Parses `5` or `5a` after the leading letter of a track reference.
fn ref_part(c: &[char]) -> Option<(String, usize)> {
    let n = digits_len(c);
    if !(1..=2).contains(&n) {
        return None;
    }
    let mut label = words(c[..n].iter().collect::<String>().parse().ok()?);
    let mut used = n;
    match c.get(n) {
        Some(l) if l.is_ascii_lowercase() && *l != 's' => {
            label = format!("{label} {}", l.to_ascii_uppercase());
            used += 1;
        }
        _ => {}
    }
    if c.get(used).is_some_and(|x| x.is_alphanumeric()) {
        return None;
    }
    Some((label, used))
}

/// `T5`, `T10a`, `T5-T6`, `S2`, `S1-S3`.
fn track_ref(c: &[char]) -> Option<(String, usize)> {
    let (singular, plural) = match c.first()? {
        'T' => ("Turn", "Turns"),
        'S' => ("sector", "sectors"),
        _ => return None,
    };
    let (first, used) = ref_part(&c[1..])?;
    let used = used + 1;
    if matches!(c.get(used), Some('-' | '–')) {
        let mut r = &c[used + 1..];
        let mut extra = 1;
        if r.first() == Some(&c[0]) {
            r = &r[1..];
            extra += 1;
        }
        if let Some((second, u2)) = ref_part(r) {
            return Some((format!("{plural} {first} to {second}"), used + extra + u2));
        }
    }
    Some((format!("{singular} {first}"), used))
}

enum Unit {
    Seconds,
    Kph,
    Rpm,
    Metres,
    Degrees,
    Percent,
    G,
    Incidents,
}

/// Recognises a unit suffix right after a number; returns it and the chars it spans
/// (including one optional leading space).
fn match_unit(after: &[char]) -> Option<(Unit, usize)> {
    let spaced = usize::from(after.first() == Some(&' '));
    let rest = &after[spaced..];
    let head: String = rest.iter().take(4).collect::<String>().to_lowercase();
    let word_end = |n: usize| {
        rest.get(n)
            .is_none_or(|x| !x.is_alphanumeric() && *x != '/')
    };
    let found = if rest.starts_with(&['°', 'C']) {
        (Unit::Degrees, 2)
    } else if rest.first() == Some(&'°') {
        (Unit::Degrees, 1)
    } else if rest.first() == Some(&'%') {
        (Unit::Percent, 1)
    } else if head.starts_with("km/h") && word_end(4) {
        (Unit::Kph, 4)
    } else if head.starts_with("kph") && word_end(3) {
        (Unit::Kph, 3)
    } else if head.starts_with("rpm") && word_end(3) {
        (Unit::Rpm, 3)
    } else if rest.first() == Some(&'s') && word_end(1) {
        (Unit::Seconds, 1)
    } else if rest.first() == Some(&'m') && word_end(1) {
        (Unit::Metres, 1)
    } else if spaced == 0 && rest.first() == Some(&'C') && word_end(1) {
        (Unit::Degrees, 1)
    } else if spaced == 0 && rest.first() == Some(&'g') && word_end(1) {
        (Unit::G, 1)
    } else if spaced == 0 && rest.first() == Some(&'x') && word_end(1) {
        (Unit::Incidents, 1)
    } else {
        return None;
    };
    Some((found.0, spaced + found.1))
}

/// Numbers, optionally signed, with their unit: "65%", "+0.37s", "9196 rpm", "18 m", "29C".
/// `out` is the text produced so far (used to spot "Turns 10-11" style ranges).
fn quantity(c: &[char], out: &str) -> Option<(String, usize)> {
    let sign = match c.first()? {
        s @ ('+' | '-') if c.get(1).is_some_and(char::is_ascii_digit) => Some(*s),
        _ => None,
    };
    let start = usize::from(sign.is_some());
    if !c.get(start)?.is_ascii_digit() {
        return None;
    }
    let int_len = digits_len(&c[start..]);
    let mut end = start + int_len;
    if c.get(end) == Some(&'.') && c.get(end + 1).is_some_and(char::is_ascii_digit) {
        end += 1 + digits_len(&c[end + 1..]);
    }
    let num: String = c[start..end].iter().collect();
    let is_int = !num.contains('.');

    // "Turns 10-11" -> "Turns ten to eleven"
    let last_word = out.split_whitespace().next_back().unwrap_or_default().to_lowercase();
    if sign.is_none()
        && is_int
        && matches!(c.get(end), Some('-' | '–'))
        && ["turns", "sectors", "laps", "gears"]
            .iter()
            .any(|w| last_word.ends_with(w))
    {
        let n2 = digits_len(&c[end + 1..]);
        if n2 > 0 && !c.get(end + 1 + n2).is_some_and(|x| x.is_alphanumeric()) {
            let second: String = c[end + 1..end + 1 + n2].iter().collect();
            return Some((
                format!("{} to {}", plain(&num), plain(&second)),
                end + 1 + n2,
            ));
        }
    }

    let minus = if sign == Some('-') { "minus " } else { "" };
    match match_unit(&c[end..]) {
        Some((unit, used)) => {
            let text = match unit {
                // Deltas drop the sign: the surrounding words say slower or faster.
                Unit::Seconds => delta_words(&num),
                Unit::Kph => format!("{minus}{} K P H", plain(&num)),
                Unit::Rpm => format!("{minus}{} revs", rpm_words(&num)),
                Unit::Metres if num == "1" => format!("{minus}one metre"),
                Unit::Metres => format!("{minus}{} metres", plain(&num)),
                Unit::Degrees => format!("{minus}{} degrees", plain(&num)),
                Unit::Percent => format!("{minus}{} percent", plain(&num)),
                Unit::G => format!("{minus}{} G", plain(&num)),
                Unit::Incidents if num == "1" => "one incident".to_string(),
                Unit::Incidents if is_int => format!("{} incidents", plain(&num)),
                Unit::Incidents => format!("{} times", plain(&num)),
            };
            Some((text, end + used))
        }
        None => {
            // Digits glued to letters ("3D", "1st"): leave the whole token alone.
            let glued = c[end..].iter().take_while(|x| x.is_alphanumeric()).count();
            if glued > 0 {
                let raw: String = c[..end + glued].iter().collect();
                return Some((raw, end + glued));
            }
            Some((format!("{minus}{}", plain(&num)), end))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::for_speech as sp;

    #[test]
    fn lap_times() {
        assert_eq!(sp("2:04.690"), "two oh four point seven.");
        assert_eq!(sp("1:32.051"), "one thirty two point one.");
        assert_eq!(sp("1:59.96"), "two oh oh point zero.");
        assert_eq!(sp("1:15.2 lap"), "one fifteen point two lap.");
    }

    #[test]
    fn time_deltas() {
        assert_eq!(sp("+0.37s"), "four tenths.");
        assert_eq!(sp("0.37 s slower"), "four tenths slower.");
        assert_eq!(sp("-0.214s"), "two tenths.");
        assert_eq!(sp("0.05s"), "half a tenth.");
        assert_eq!(sp("0.02s"), "a few hundredths.");
        assert_eq!(sp("0.1s"), "a tenth.");
        assert_eq!(sp("1.4s"), "one point four seconds.");
        assert_eq!(sp("+1.2s"), "one point two seconds.");
        assert_eq!(sp("2s"), "two seconds.");
    }

    #[test]
    fn units() {
        assert_eq!(
            sp("minimum speed 6 kph lower"),
            "minimum speed six K P H lower."
        );
        assert_eq!(sp("120 km/h"), "one hundred twenty K P H.");
        assert_eq!(sp("at 9196 rpm"), "at ninety-one ninety-six revs.");
        assert_eq!(sp("8500 rpm"), "eighty-five hundred revs.");
        assert_eq!(sp("10200 rpm"), "ten thousand two hundred revs.");
        assert_eq!(sp("346 rpm"), "three hundred forty six revs.");
        assert_eq!(sp("braked 18 m earlier"), "braked eighteen metres earlier.");
        assert_eq!(sp("1 m"), "one metre.");
        assert_eq!(
            sp("from 29C to 24°C"),
            "from twenty nine degrees to twenty four degrees."
        );
        assert_eq!(sp("29 °C"), "twenty nine degrees.");
        assert_eq!(
            sp("+8.9° extra lock"),
            "eight point nine degrees extra lock."
        );
        assert_eq!(sp("65% of braking"), "sixty five percent of braking.");
        assert_eq!(sp("peak 1.8g"), "peak one point eight G.");
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(sp("65 and 8.9"), "sixty five and eight point nine.");
        assert_eq!(sp("lap 12"), "lap twelve.");
        assert_eq!(
            sp("99999 laps"),
            "ninety nine thousand nine hundred ninety nine laps."
        );
        assert_eq!(sp("123456 laps"), "123456 laps.");
    }

    #[test]
    fn track_references() {
        assert_eq!(sp("T5"), "Turn five.");
        assert_eq!(sp("T10a"), "Turn ten A.");
        assert_eq!(sp("T5–T6"), "Turns five to six.");
        assert_eq!(sp("T5-T6"), "Turns five to six.");
        assert_eq!(sp("S2 was quick"), "sector two was quick.");
        assert_eq!(sp("Focus on Sector 5"), "Focus on Sector five.");
        assert_eq!(sp("Turns 10-11"), "Turns ten to eleven.");
        assert_eq!(
            sp("Turn 12: 0.37s slower"),
            "Turn twelve, four tenths slower."
        );
    }

    #[test]
    fn arrows() {
        assert_eq!(sp("1→2"), "one to two.");
        assert_eq!(sp("Gear 1→2 upshift"), "Gear one to two upshift.");
    }

    #[test]
    fn symbols_and_markdown() {
        assert_eq!(sp("**Bold** point"), "Bold point.");
        assert_eq!(
            sp("# Heading\n- first\n• second\n1. third"),
            "Heading. first. second. third."
        );
        assert_eq!(sp("a · b — c – d"), "a, b, c, d.");
        assert_eq!(sp("slow (lap 12) corner"), "slow, lap twelve, corner.");
        assert_eq!(
            sp("65% vs 30%"),
            "sixty five percent versus thirty percent."
        );
        assert_eq!(sp("e.g. braking"), "for example braking.");
        assert_eq!(sp("brake & throttle"), "brake and throttle.");
        assert_eq!(sp("4x"), "four incidents.");
        assert_eq!(sp("1x"), "one incident.");
        assert_eq!(sp("  lots   of   space  "), "lots of space.");
        assert_eq!(sp("keep going!"), "keep going!");
        assert_eq!(sp("next:"), "next.");
    }

    #[test]
    fn leaves_words_alone() {
        assert_eq!(
            sp("Ts and St with ABS in an S-curve, 3D"),
            "Ts and St with ABS in an S-curve, 3D."
        );
        assert_eq!(sp("1st gear"), "1st gear.");
        assert_eq!(sp("Tower"), "Tower.");
    }

    #[test]
    fn full_sentences() {
        assert_eq!(
            sp("Turn 12: 0.37s slower on your fastest lap (lap 12): more understeer mid-corner (+8.9° extra steering lock), ABS active for 65% of braking vs 30%"),
            "Turn twelve, four tenths slower on your fastest lap, lap twelve, more understeer mid-corner, eight point nine degrees extra steering lock, ABS active for sixty five percent of braking versus thirty percent."
        );
        assert_eq!(
            sp("Gear 1→2 upshift: you usually change up at 9196 rpm, 346 rpm past the car's shift light (3 of 4 late)"),
            "Gear one to two upshift, you usually change up at ninety-one ninety-six revs, three hundred forty six revs past the car's shift light, three of four late."
        );
    }
}
