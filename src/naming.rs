//! Automatic page titles that learn from renames.
//!
//! A section remembers a [`Pattern`] such as `Lecture {n}` or
//! `{n}. {date}`. When the user renames a page to something containing a
//! number and/or a date, that title becomes the section's new pattern, and
//! the next page continues it (next number, today's date in the same format).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Piece {
    Text { text: String },
    /// Counter; `width` > 1 means zero padded ("03").
    Number { width: usize },
    Date { format: Vec<DatePart> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatePart {
    Text { text: String },
    Weekday { long: bool },
    Day { pad: bool },
    Month { pad: bool },
    MonthName { long: bool },
    Year { full: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub pieces: Vec<Piece>,
}

/// The calendar date used when generating a title.
#[derive(Clone, Copy, Debug)]
pub struct Today {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// 1 = Monday … 7 = Sunday
    pub weekday: u32,
}

impl Today {
    pub fn now() -> Self {
        let now = gtk::glib::DateTime::now_local().expect("local time");
        Today {
            year: now.year(),
            month: now.month() as u32,
            day: now.day_of_month() as u32,
            weekday: now.day_of_week() as u32,
        }
    }
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
const WEEKDAYS: [&str; 7] =
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

const MONTH_RE: &str = r"(?i:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sept?(?:ember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)";
const WEEKDAY_RE: &str = r"(?i:mon(?:day)?|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:r(?:s(?:day)?)?)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)";

static ISO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{4})([-./])(\d{1,2})([-./])(\d{1,2})\b").unwrap());
static NUMERIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})([-./])(\d{1,2})([-./])(\d{4}|\d{2})\b").unwrap());
static DAY_MONTH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\b(?:({WEEKDAY_RE})(,?\s+))?(\d{{1,2}})(\s+)({MONTH_RE})\b(?:(,?\s+)(\d{{4}})\b)?"
    ))
    .unwrap()
});
static MONTH_DAY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\b(?:({WEEKDAY_RE})(,?\s+))?({MONTH_RE})(\s+)(\d{{1,2}})\b(?:(,?\s+)(\d{{4}})\b)?"
    ))
    .unwrap()
});
static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());

fn text(s: &str) -> DatePart {
    DatePart::Text { text: s.to_string() }
}

fn is_long_name(s: &str) -> bool {
    match s.to_lowercase().as_str() {
        "june" | "july" => true,
        // Still abbreviations despite being longer than three letters.
        "sept" | "tues" | "thur" | "thurs" => false,
        _ => s.len() > 3,
    }
}

fn opt_weekday(parts: &mut Vec<DatePart>, c: &regex::Captures, wd: usize, sep: usize) {
    if let (Some(w), Some(s)) = (c.get(wd), c.get(sep)) {
        parts.push(DatePart::Weekday { long: is_long_name(w.as_str()) });
        parts.push(text(s.as_str()));
    }
}

fn opt_year(parts: &mut Vec<DatePart>, c: &regex::Captures, sep: usize, year: usize) {
    if let (Some(s), Some(_)) = (c.get(sep), c.get(year)) {
        parts.push(text(s.as_str()));
        parts.push(DatePart::Year { full: true });
    }
}

/// Find a date in `title`; returns its byte range and format.
fn find_date(title: &str) -> Option<(std::ops::Range<usize>, Vec<DatePart>)> {
    if let Some(c) = ISO_RE.captures(title) {
        let (m, d): (u32, u32) = (c[3].parse().ok()?, c[5].parse().ok()?);
        if (1..=12).contains(&m) && (1..=31).contains(&d) {
            let format = vec![
                DatePart::Year { full: true },
                text(&c[2]),
                DatePart::Month { pad: c[3].len() == 2 },
                text(&c[4]),
                DatePart::Day { pad: c[5].len() == 2 },
            ];
            return Some((c.get(0)?.range(), format));
        }
    }
    if let Some(c) = NUMERIC_RE.captures(title) {
        let (a, b): (u32, u32) = (c[1].parse().ok()?, c[3].parse().ok()?);
        // Day-first unless the first field can only be a month.
        let day_first = !(a <= 12 && b > 12);
        let (d, m) = if day_first { (a, b) } else { (b, a) };
        if (1..=12).contains(&m) && (1..=31).contains(&d) {
            let first = if day_first {
                DatePart::Day { pad: c[1].len() == 2 }
            } else {
                DatePart::Month { pad: c[1].len() == 2 }
            };
            let second = if day_first {
                DatePart::Month { pad: c[3].len() == 2 }
            } else {
                DatePart::Day { pad: c[3].len() == 2 }
            };
            let format = vec![
                first,
                text(&c[2]),
                second,
                text(&c[4]),
                DatePart::Year { full: c[5].len() == 4 },
            ];
            return Some((c.get(0)?.range(), format));
        }
    }
    if let Some(c) = DAY_MONTH_RE.captures(title) {
        let d: u32 = c[3].parse().ok()?;
        if (1..=31).contains(&d) {
            let mut format = Vec::new();
            opt_weekday(&mut format, &c, 1, 2);
            format.push(DatePart::Day { pad: c[3].len() == 2 });
            format.push(text(&c[4]));
            format.push(DatePart::MonthName { long: is_long_name(&c[5]) });
            opt_year(&mut format, &c, 6, 7);
            return Some((c.get(0)?.range(), format));
        }
    }
    if let Some(c) = MONTH_DAY_RE.captures(title) {
        let d: u32 = c[5].parse().ok()?;
        if (1..=31).contains(&d) {
            let mut format = Vec::new();
            opt_weekday(&mut format, &c, 1, 2);
            format.push(DatePart::MonthName { long: is_long_name(&c[3]) });
            format.push(text(&c[4]));
            format.push(DatePart::Day { pad: c[5].len() == 2 });
            opt_year(&mut format, &c, 6, 7);
            return Some((c.get(0)?.range(), format));
        }
    }
    None
}

/// Split plain text around its last number, if any.
fn push_with_number(pieces: &mut Vec<Piece>, s: &str, allow_number: bool) -> bool {
    let found = allow_number.then(|| NUMBER_RE.find_iter(s).last()).flatten();
    match found {
        Some(m) => {
            if m.start() > 0 {
                pieces.push(Piece::Text { text: s[..m.start()].to_string() });
            }
            let digits = m.as_str();
            let width = if digits.len() > 1 && digits.starts_with('0') { digits.len() } else { 1 };
            pieces.push(Piece::Number { width });
            if m.end() < s.len() {
                pieces.push(Piece::Text { text: s[m.end()..].to_string() });
            }
            true
        }
        None => {
            if !s.is_empty() {
                pieces.push(Piece::Text { text: s.to_string() });
            }
            false
        }
    }
}

impl Pattern {
    pub fn numbered(prefix: &str) -> Self {
        Pattern {
            pieces: vec![Piece::Text { text: format!("{prefix} ") }, Piece::Number { width: 1 }],
        }
    }

    /// Learn a pattern from a title. Returns `None` if the title has neither
    /// a number nor a date (the section keeps its previous pattern).
    pub fn learn(title: &str) -> Option<Self> {
        let mut pieces = Vec::new();
        match find_date(title) {
            Some((range, format)) => {
                let before = &title[..range.start];
                let after = &title[range.end..];
                // Prefer a number before the date ("Lecture 4 - 12 Oct"),
                // otherwise one after it ("12 Oct - Lecture 4").
                let numbered = push_with_number(&mut pieces, before, true);
                pieces.push(Piece::Date { format });
                push_with_number(&mut pieces, after, !numbered);
            }
            None => {
                if !push_with_number(&mut pieces, title, true) {
                    return None;
                }
            }
        }
        Some(Pattern { pieces })
    }

    fn matcher(&self) -> Regex {
        let mut re = String::from("^");
        for piece in &self.pieces {
            match piece {
                Piece::Text { text } => re.push_str(&regex::escape(text)),
                Piece::Number { .. } => re.push_str(r"(\d+)"),
                Piece::Date { .. } => re.push_str(r".+?"),
            }
        }
        re.push('$');
        Regex::new(&re).expect("escaped pattern")
    }

    /// Title for the next page, given the titles already in the section.
    pub fn next_title<'a>(&self, existing: impl IntoIterator<Item = &'a str>, today: Today) -> String {
        let matcher = self.matcher();
        let next = existing
            .into_iter()
            .filter_map(|t| matcher.captures(t)?.get(1)?.as_str().parse::<u64>().ok())
            .max()
            .map_or(1, |n| n + 1);
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text { text } => out.push_str(text),
                Piece::Number { width } => out.push_str(&format!("{next:0width$}")),
                Piece::Date { format } => out.push_str(&render_date(format, today)),
            }
        }
        out
    }
}

fn render_date(format: &[DatePart], t: Today) -> String {
    let mut out = String::new();
    for part in format {
        match part {
            DatePart::Text { text } => out.push_str(text),
            DatePart::Weekday { long } => {
                let name = WEEKDAYS[(t.weekday as usize).clamp(1, 7) - 1];
                out.push_str(if *long { name } else { &name[..3] });
            }
            DatePart::Day { pad: true } => out.push_str(&format!("{:02}", t.day)),
            DatePart::Day { pad: false } => out.push_str(&t.day.to_string()),
            DatePart::Month { pad: true } => out.push_str(&format!("{:02}", t.month)),
            DatePart::Month { pad: false } => out.push_str(&t.month.to_string()),
            DatePart::MonthName { long } => {
                let name = MONTHS[(t.month as usize).clamp(1, 12) - 1];
                out.push_str(if *long { name } else { &name[..3] });
            }
            DatePart::Year { full: true } => out.push_str(&t.year.to_string()),
            DatePart::Year { full: false } => out.push_str(&format!("{:02}", t.year % 100)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: Today = Today { year: 2026, month: 10, day: 12, weekday: 1 };

    fn next(learned_from: &str, existing: &[&str]) -> String {
        Pattern::learn(learned_from).unwrap().next_title(existing.iter().copied(), TODAY)
    }

    #[test]
    fn default_numbering() {
        let p = Pattern::numbered("Lecture");
        assert_eq!(p.next_title([], TODAY), "Lecture 1");
        assert_eq!(p.next_title(["Lecture 1", "Lecture 7", "Intro"], TODAY), "Lecture 8");
    }

    #[test]
    fn trailing_number() {
        assert_eq!(next("Week 4", &["Week 4"]), "Week 5");
        assert_eq!(next("Esercitazione 02", &["Esercitazione 02"]), "Esercitazione 03");
        assert_eq!(next("Ch. 3 notes", &["Ch. 3 notes"]), "Ch. 4 notes");
        assert!(Pattern::learn("Integrals intro").is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(next("2026-09-30", &[]), "2026-10-12");
        assert_eq!(next("30/9/2026", &[]), "12/10/2026");
        assert_eq!(next("Tue 6 Oct 2026", &[]), "Mon 12 Oct 2026");
        assert_eq!(next("Tuesday, September 30", &[]), "Monday, October 12");
        assert_eq!(next("Lab - 6 Oct", &[]), "Lab - 12 Oct");
    }

    #[test]
    fn number_and_date() {
        assert_eq!(next("Lecture 4 — 6 Oct", &["Lecture 4 — 6 Oct"]), "Lecture 5 — 12 Oct");
        assert_eq!(next("6 Oct - Lecture 4", &["6 Oct - Lecture 4"]), "12 Oct - Lecture 5");
    }
}
