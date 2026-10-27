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
    Weekday {
        long: bool,
        /// Language code from [`LANGS`]; titles learned before there
        /// were several are English.
        #[serde(default = "english")]
        lang: String,
        /// Written with a capital ("Lundi" at the start of a title).
        #[serde(default)]
        cap: bool,
    },
    Day { pad: bool },
    Month { pad: bool },
    MonthName {
        long: bool,
        #[serde(default = "english")]
        lang: String,
        #[serde(default)]
        cap: bool,
    },
    Year { full: bool },
}

fn english() -> String {
    "en".into()
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

/// Month and weekday names, full and short, of one language.
struct Lang {
    code: &'static str,
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    /// Monday first.
    weekdays: [&'static str; 7],
    weekdays_short: [&'static str; 7],
}

const LANGS: [Lang; 7] = [
    Lang {
        code: "en",
        months: [
            "January", "February", "March", "April", "May", "June", "July", "August", "September",
            "October", "November", "December",
        ],
        months_short: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
        weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
        weekdays_short: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    },
    Lang {
        code: "de",
        months: [
            "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September",
            "Oktober", "November", "Dezember",
        ],
        months_short: ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"],
        weekdays: ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"],
        weekdays_short: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    },
    Lang {
        code: "fr",
        months: [
            "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre",
            "octobre", "novembre", "décembre",
        ],
        months_short: [
            "janv", "févr", "mars", "avr", "mai", "juin", "juil", "août", "sept", "oct", "nov", "déc",
        ],
        weekdays: ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
        weekdays_short: ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"],
    },
    Lang {
        code: "es",
        months: [
            "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre",
            "octubre", "noviembre", "diciembre",
        ],
        months_short: ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"],
        weekdays: ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
        weekdays_short: ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
    },
    Lang {
        code: "it",
        months: [
            "gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto",
            "settembre", "ottobre", "novembre", "dicembre",
        ],
        months_short: ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"],
        weekdays: ["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"],
        weekdays_short: ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
    },
    Lang {
        code: "pt",
        months: [
            "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro",
            "outubro", "novembro", "dezembro",
        ],
        months_short: ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"],
        weekdays: [
            "segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado",
            "domingo",
        ],
        weekdays_short: ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"],
    },
    Lang {
        code: "ro",
        months: [
            "ianuarie", "februarie", "martie", "aprilie", "mai", "iunie", "iulie", "august",
            "septembrie", "octombrie", "noiembrie", "decembrie",
        ],
        months_short: ["ian", "feb", "mar", "apr", "mai", "iun", "iul", "aug", "sep", "oct", "noi", "dec"],
        weekdays: ["luni", "marți", "miercuri", "joi", "vineri", "sâmbătă", "duminică"],
        weekdays_short: ["lun", "mar", "mie", "joi", "vin", "sâm", "dum"],
    },
];

/// Short English names beyond the table: (name, is a month, index).
const EN_EXTRA: [(&str, bool, usize); 4] =
    [("sept", true, 8), ("tues", false, 1), ("thur", false, 3), ("thurs", false, 3)];

/// The languages, the user's own first: it decides names several share
/// ("mar" is March in most of them).
fn langs() -> impl Iterator<Item = &'static Lang> {
    static FIRST: LazyLock<usize> = LazyLock::new(|| {
        gtk::glib::language_names()
            .iter()
            .find_map(|name| LANGS.iter().position(|l| name.split(['_', '.', '@']).next() == Some(l.code)))
            .unwrap_or(0)
    });
    std::iter::once(&LANGS[*FIRST]).chain(LANGS.iter().enumerate().filter(|(i, _)| *i != *FIRST).map(|(_, l)| l))
}

fn names(lang: &Lang, month: bool) -> (&[&'static str], &[&'static str]) {
    if month { (&lang.months, &lang.months_short) } else { (&lang.weekdays, &lang.weekdays_short) }
}

/// What a matched month or weekday name is: a `MonthName` / `Weekday`.
fn name_part(s: &str, month: bool, english: bool) -> Option<DatePart> {
    let lower = s.to_lowercase();
    let found = langs()
        .filter(|lang| !english || lang.code == "en")
        .find_map(|lang| {
            let (long, short) = names(lang, month);
            let at = |list: &[&str]| list.iter().position(|n| n.to_lowercase() == lower);
            at(long).map(|_| (lang.code, true)).or_else(|| at(short).map(|_| (lang.code, false)))
        })
        .or_else(|| EN_EXTRA.iter().any(|(n, m, _)| *n == lower && *m == month).then_some(("en", false)))?;
    let (lang, long) = (found.0.to_string(), found.1);
    let cap = s.chars().next().is_some_and(char::is_uppercase);
    Some(if month { DatePart::MonthName { long, lang, cap } } else { DatePart::Weekday { long, lang, cap } })
}

/// A regex alternation of every month (or weekday) name, longest first.
fn alternation(langs: &[Lang], month: bool) -> String {
    let mut all: Vec<String> = langs
        .iter()
        .flat_map(|l| {
            let (long, short) = names(l, month);
            long.iter().chain(short)
        })
        .map(|n| n.to_lowercase())
        .chain(EN_EXTRA.iter().filter(|e| e.1 == month).map(|e| e.0.to_string()))
        .collect();
    all.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
    all.dedup();
    let escaped: Vec<String> = all.iter().map(|n| regex::escape(n)).collect();
    format!("(?i:{})", escaped.join("|"))
}

static ISO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{4})([-./])(\d{1,2})([-./])(\d{1,2})\b").unwrap());
static NUMERIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})([-./])(\d{1,2})([-./])(\d{4}|\d{2})\b").unwrap());
// "3. Oktober", "12 de octubre de 2026", "Tue 6 Oct 2026".
static DAY_MONTH_RE: LazyLock<Regex> = LazyLock::new(|| {
    let (month, weekday) = (alternation(&LANGS, true), alternation(&LANGS, false));
    Regex::new(&format!(
        r"\b(?:({weekday})(\.?,?\s+))?(\d{{1,2}})(\.?\s+(?:de\s+)?)({month})\b(?:(\.?,?\s+(?:de\s+)?)(\d{{4}})\b)?"
    ))
    .unwrap()
});
// Month first is English only ("Oct 6"); other languages would read
// "Problem set 2" as 2 September.
static MONTH_DAY_RE: LazyLock<Regex> = LazyLock::new(|| {
    let (month, weekday) = (alternation(&LANGS[..1], true), alternation(&LANGS[..1], false));
    Regex::new(&format!(
        r"\b(?:({weekday})(\.?,?\s+))?({month})(\.?\s+)(\d{{1,2}})\b(?:(,?\s+)(\d{{4}})\b)?"
    ))
    .unwrap()
});
static NUMBER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());

fn text(s: &str) -> DatePart {
    DatePart::Text { text: s.to_string() }
}

fn opt_weekday(parts: &mut Vec<DatePart>, c: &regex::Captures, wd: usize, sep: usize, english: bool) {
    if let (Some(w), Some(s)) = (c.get(wd), c.get(sep))
        && let Some(part) = name_part(w.as_str(), false, english)
    {
        parts.push(part);
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
            opt_weekday(&mut format, &c, 1, 2, false);
            format.push(DatePart::Day { pad: c[3].len() == 2 });
            format.push(text(&c[4]));
            format.push(name_part(&c[5], true, false)?);
            opt_year(&mut format, &c, 6, 7);
            return Some((c.get(0)?.range(), format));
        }
    }
    if let Some(c) = MONTH_DAY_RE.captures(title) {
        let d: u32 = c[5].parse().ok()?;
        if (1..=31).contains(&d) {
            let mut format = Vec::new();
            opt_weekday(&mut format, &c, 1, 2, true);
            format.push(name_part(&c[3], true, true)?);
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
            DatePart::Weekday { long, lang, cap } => {
                out.push_str(&render_name(lang, false, t.weekday as usize, *long, *cap))
            }
            DatePart::Day { pad: true } => out.push_str(&format!("{:02}", t.day)),
            DatePart::Day { pad: false } => out.push_str(&t.day.to_string()),
            DatePart::Month { pad: true } => out.push_str(&format!("{:02}", t.month)),
            DatePart::Month { pad: false } => out.push_str(&t.month.to_string()),
            DatePart::MonthName { long, lang, cap } => {
                out.push_str(&render_name(lang, true, t.month as usize, *long, *cap))
            }
            DatePart::Year { full: true } => out.push_str(&t.year.to_string()),
            DatePart::Year { full: false } => out.push_str(&format!("{:02}", t.year % 100)),
        }
    }
    out
}

/// Month (1–12) or weekday (1 = Monday) `n` in `lang`.
fn render_name(lang: &str, month: bool, n: usize, long: bool, cap: bool) -> String {
    let lang = LANGS.iter().find(|l| l.code == lang).unwrap_or(&LANGS[0]);
    let (full, short) = names(lang, month);
    let name = if long { full } else { short }[n.clamp(1, full.len()) - 1];
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if cap => first.to_uppercase().chain(chars).collect(),
        _ => name.to_string(),
    }
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
    fn dates_in_other_languages() {
        // Names several languages share ("September") go by the system
        // language, so these use ones that aren't.
        assert_eq!(next("Vorlesung 3. März", &["Vorlesung 3. März"]), "Vorlesung 12. Oktober");
        assert_eq!(next("Mo, 5. Okt", &[]), "Mo, 12. Okt");
        assert_eq!(next("Lundi 5 octobre", &[]), "Lundi 12 octobre");
        assert_eq!(next("Clase 4 - 5 de octubre de 2026", &["Clase 4 - 5 de octubre de 2026"]), "Clase 5 - 12 de octubre de 2026");
        assert_eq!(next("lunedì 5 ottobre", &[]), "lunedì 12 ottobre");
        assert_eq!(next("segunda-feira, 5 de outubro", &[]), "segunda-feira, 12 de outubro");
        assert_eq!(next("Curs 3, 5 octombrie", &["Curs 3, 5 octombrie"]), "Curs 4, 12 octombrie");
        // Not a date: month-first is English only.
        assert_eq!(next("Problem set 2", &["Problem set 2"]), "Problem set 3");
    }

    #[test]
    fn old_patterns_stay_english() {
        let json = r#"{"kind":"month_name","long":false}"#;
        let part: DatePart = serde_json::from_str(json).unwrap();
        assert_eq!(render_date(&[part], TODAY), "Oct");
    }

    #[test]
    fn number_and_date() {
        assert_eq!(next("Lecture 4 — 6 Oct", &["Lecture 4 — 6 Oct"]), "Lecture 5 — 12 Oct");
        assert_eq!(next("6 Oct - Lecture 4", &["6 Oct - Lecture 4"]), "12 Oct - Lecture 5");
    }
}
