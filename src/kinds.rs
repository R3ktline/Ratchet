//! What a notebook or section is about, guessed from its name: the icon it
//! shows in the sidebar, the color a subject's notebook gets and how a
//! section's pages are named. Names are matched in English, German,
//! French, Spanish, Italian, Portuguese and Romanian.

use crate::i18n::gettext_noop;

/// A kind of section: lectures, exercises, a lab…
pub struct SectionType {
    pub icon: &'static str,
    /// Page names in such a section: "Lecture 1", "Exercise 2"… (English;
    /// translated where used).
    pub page: &'static str,
    stems: &'static [&'static str],
}

/// A subject: math, medicine…
pub struct Subject {
    pub icon: &'static str,
    /// One of `library::PALETTE`.
    pub color: &'static str,
    stems: &'static [&'static str],
}

const EXERCISE_ICON: &str = "document-edit-symbolic";

// First match wins, so "Coursework" is an assignment before it's a course.
const SECTION_TYPES: &[SectionType] = &[
    SectionType {
        icon: "ratchet-papers-symbolic",
        page: gettext_noop("Assignment"),
        stems: &[
            "assign", "homework", "coursework", "sheet", "worksheet", "paper", "problem",
            "hausaufgabe", "aufgabe", "abgabe", "übungsbl", "uebungsbl", "blatt", "blätter",
            "devoir", "feuille", "tarea", "compit", "tarefa", "temă", "teme", "tema",
        ],
    },
    SectionType {
        icon: EXERCISE_ICON,
        page: gettext_noop("Exercise"),
        stems: &[
            "exerci", "practice", "recitation", "übung", "uebung", "ubung", "td", "ejercic",
            "esercit", "eserciz", "exercíc",
        ],
    },
    SectionType { icon: EXERCISE_ICON, page: gettext_noop("Tutorial"), stems: &["tutor"] },
    SectionType { icon: EXERCISE_ICON, page: gettext_noop("Seminar"), stems: &["semin", "sémin"] },
    SectionType {
        icon: "applications-science-symbolic",
        page: gettext_noop("Lab"),
        stems: &[
            "lab", "labs", "labo", "labor", "praktikum", "tp", "practic", "práctic", "pratic",
            "prátic",
        ],
    },
    SectionType {
        icon: "alarm-symbolic",
        page: gettext_noop("Exam"),
        stems: &[
            "exam", "midterm", "quiz", "klausur", "prüfung", "pruefung", "prufung", "partiel",
            "parcial", "esam", "colocviu",
        ],
    },
    SectionType {
        icon: "user-bookmarks-symbolic",
        page: gettext_noop("Summary"),
        stems: &[
            "summar", "cheat", "formula", "glossar", "zusammenfass", "spick", "formel",
            "résum", "resumen", "resumo", "riassunt", "rezumat",
        ],
    },
    SectionType {
        icon: "folder-projects-symbolic",
        page: gettext_noop("Project"),
        stems: &["project", "projekt", "projet", "proyect", "progett", "proiect"],
    },
    SectionType {
        icon: "accessories-dictionary-symbolic",
        page: gettext_noop("Lecture"),
        stems: &[
            "cours", "lectur", "vorlesung", "vl", "kurs", "curs", "corso", "corsi", "lezion",
            "lecci", "clase", "aula", "preleger",
        ],
    },
];

const SUBJECTS: &[Subject] = &[
    Subject {
        icon: "ratchet-stethoscope-symbolic",
        color: "red",
        stems: &[
            "medic", "médec", "mediz", "anatom", "physio", "fisiolog", "fiziolog", "patholog",
            "patolog", "pharma", "farmac", "chirurg", "surg", "cirug", "clinic", "klinik",
            "clínic", "cliniq", "nurs", "pflege", "enferm", "infermier", "histolog", "istolog",
            "embryolog", "embriolog", "immunolog", "imunolog", "inmunolog", "neurolog",
            "radiolog", "cardio", "dermat", "pediatr", "pädiatr", "psychiatr", "psiquiatr",
            "psichiatr", "psihiatr", "dental", "dentist", "zahn", "odonto", "veterin",
            "tiermed",
        ],
    },
    Subject {
        icon: "ratchet-sigma-symbolic",
        color: "blue",
        stems: &[
            "math", "matem", "analys", "análisis", "analisi", "analiz", "algebr", "álgebr",
            "geometr", "géométr", "stochast", "statist", "estadíst", "estadist", "probab",
            "wahrscheinlich", "calcul", "cálcul", "numerik", "numeric", "numéri", "topolog",
            "kombinatorik", "combinat",
        ],
    },
    Subject {
        icon: "applications-science-symbolic",
        color: "orange",
        stems: &["chem", "biochem", "chim", "quím", "quim"],
    },
    Subject {
        icon: "emoji-objects-symbolic",
        color: "purple",
        stems: &[
            "physik", "physics", "physique", "física", "fisica", "fizic", "optik", "optics",
            "optique", "thermodynam", "quantum", "quanten", "elektrodyn", "electrodyn", "astro",
        ],
    },
    Subject {
        icon: "emoji-nature-symbolic",
        color: "green",
        stems: &[
            "biolog", "botan", "zoolog", "genet", "génét", "ecolog", "ökolog", "oekolog",
            "écolog", "mikrobiolog", "microbiolog",
        ],
    },
    Subject {
        icon: "utilities-terminal-symbolic",
        color: "slate",
        stems: &[
            "informat", "comput", "program", "algorithm", "algorit", "software", "datenbank",
            "database", "datenstruk", "coding", "crypt", "krypt",
        ],
    },
    Subject {
        icon: "applications-engineering-symbolic",
        color: "brown",
        stems: &[
            "engineer", "ingenieur", "ingénie", "ingenier", "ingegner", "engenhar", "inginer",
            "mechan", "mecán", "mécan", "mecca", "mecân", "mecan", "elektro", "electr",
            "électr", "eléctr", "elétr", "elettr", "maschin", "statik", "werkstoff",
        ],
    },
    Subject {
        icon: "preferences-desktop-locale-symbolic",
        color: "yellow",
        stems: &[
            "language", "sprach", "langue", "lengua", "idioma", "lingua", "limba", "limbă",
            "linguist", "lingüíst", "linguíst", "literat", "littérat", "grammar", "grammati",
            "grammaire", "gramátic", "gramatic", "english", "englisch", "anglais", "inglés",
            "ingles", "inglês", "engleză", "engleza", "german", "deutsch", "allemand", "alemán",
            "aleman", "tedesco", "alemão", "germană", "french", "französisch", "franzosisch",
            "français", "francais", "francés", "frances", "francês", "franceză", "franceza",
            "spanish", "spanisch", "espagnol", "español", "espanol", "spagnolo", "espanhol",
            "spaniolă", "spaniola", "italian", "italien", "italiano", "italiană", "portugu",
            "portugi", "portogh", "portughez", "romanian", "rumänisch", "rumanisch", "roumain",
            "rumano", "rumeno", "romeno", "română", "latin", "latein", "latín", "greek",
            "griechisch", "grec", "griego", "greco", "grego", "greacă",
        ],
    },
    Subject {
        icon: "applications-graphics-symbolic",
        color: "pink",
        stems: &[
            "art", "arts", "arte", "artă", "kunst", "design", "zeichn", "dessin", "dibujo",
            "disegno", "desenho", "desen", "architect", "architekt", "arquitect", "architett",
            "arhitect", "maler", "painting", "peintur", "pintur", "pittur", "fotograf",
            "photogra",
        ],
    },
    Subject {
        icon: "audio-x-generic-symbolic",
        color: "purple",
        stems: &["music", "musik", "musique", "música", "musica", "muzic"],
    },
    Subject {
        icon: "x-office-spreadsheet-symbolic",
        color: "teal",
        stems: &[
            "econom", "ökonom", "oekonom", "économ", "wirtschaft", "business", "betriebs",
            "bwl", "vwl", "finan", "account", "buchhalt", "comptab", "contab", "marketing",
            "management",
        ],
    },
    Subject {
        icon: "emoji-people-symbolic",
        color: "pink",
        stems: &["psycholog", "psicolog", "psiholog"],
    },
    Subject { icon: "emoji-activities-symbolic", color: "green", stems: &["sport", "deport"] },
];

/// A stem matches a word that starts with it; stems shorter than four
/// letters ("lab", "art", "td") only match the whole word.
fn matches(name: &str, stems: &[&str]) -> bool {
    let name = name.to_lowercase();
    let mut words = name.split(|c: char| !c.is_alphanumeric());
    words.any(|w| stems.iter().any(|stem| if stem.chars().count() < 4 { w == *stem } else { w.starts_with(stem) }))
}

pub fn section_type(name: &str) -> Option<&'static SectionType> {
    SECTION_TYPES.iter().find(|t| matches(name, t.stems))
}

pub fn subject(name: &str) -> Option<&'static Subject> {
    SUBJECTS.iter().find(|s| matches(name, s.stems))
}

/// A section's icon: its type, else its subject ("Anatomy" in a medicine
/// notebook), else none (the plain colored tab).
pub fn section_icon(name: &str) -> Option<&'static str> {
    section_type(name).map(|t| t.icon).or_else(|| subject(name).map(|s| s.icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str) -> Option<&'static str> {
        section_type(name).map(|t| t.page)
    }

    #[test]
    fn section_types() {
        assert_eq!(page("Course"), Some("Lecture"));
        assert_eq!(page("Vorlesung"), Some("Lecture"));
        assert_eq!(page("Coursework"), Some("Assignment"));
        assert_eq!(page("Übungsblätter"), Some("Assignment"));
        assert_eq!(page("Übungen"), Some("Exercise"));
        assert_eq!(page("Tema"), Some("Assignment"));
        assert_eq!(page("Seminar"), Some("Seminar"));
        assert_eq!(page("Séminaire"), Some("Seminar"));
        assert_eq!(page("Laboratory"), Some("Lab"));
        assert_eq!(page("Labor"), Some("Lab"));
        assert_eq!(page("Klausurvorbereitung"), Some("Exam"));
        assert_eq!(page("Notes"), None);
        assert_eq!(page("Labyrinth"), None);
    }

    #[test]
    fn subjects() {
        let icon = |name: &str| subject(name).map(|s| s.icon);
        assert_eq!(icon("Analysis I"), Some("ratchet-sigma-symbolic"));
        assert_eq!(icon("Mathématiques"), Some("ratchet-sigma-symbolic"));
        assert_eq!(icon("Matematică"), Some("ratchet-sigma-symbolic"));
        assert_eq!(icon("Anatomie"), Some("ratchet-stethoscope-symbolic"));
        assert_eq!(icon("Physiologie"), Some("ratchet-stethoscope-symbolic"));
        assert_eq!(icon("Physik 2"), Some("emoji-objects-symbolic"));
        assert_eq!(icon("Elektrotechnik"), Some("applications-engineering-symbolic"));
        // "art" only as a whole word.
        assert_eq!(icon("Artificial Intelligence"), None);
        assert_eq!(icon("Art History"), Some("applications-graphics-symbolic"));
        assert_eq!(subject("Medizin").map(|s| s.color), Some("red"));
    }
}
