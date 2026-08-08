//! What the assistant's wiki search returns for questions people actually ask.
//!
//! `cargo run --example search` — needs a mirrored wiki.
//!
//! Last measured against the ELDEN RING mirror of 4,868 titles: 19 of 25 first,
//! 22 of 25 in the top three. The three it misses are where searching titles
//! alone runs out — "bleed build" finds nothing because the game calls it blood
//! loss, and the flask questions land on `Flask` rather than `Golden Seed`,
//! which is the page that links to it. An agent that can read and follow gets
//! there; a search over titles cannot.

use roundtable_lib::ask;

/// A question, and every title that would answer it.
///
/// More than one, because more than one page often does. "Bleed" is a redirect
/// to "Blood Loss" and opening it lands on the same article, so a search that
/// returns either has done its job; insisting on the literal target scored a
/// correct answer as a miss. Where a hub page covers the question and links on
/// — "Flask" for how charges work — that counts too, since the assistant reads
/// what it opens.
const CASES: &[(&str, &[&str])] = &[
    ("Malenia Blade of Miquella", &["Malenia"]),
    ("Malenia boss fight how to beat", &["Malenia"]),
    ("Waterfowl Dance dodge", &["Waterfowl"]),
    ("how to get more flask charges", &["Golden Seed", "Flask"]),
    ("increase flask uses", &["Golden Seed", "Flask"]),
    ("sacred tear flask potency", &["Sacred Tear", "Sacred Flask"]),
    ("Radahn boss", &["Radahn"]),
    ("Starscourge Radahn location", &["Radahn"]),
    ("Reduvia dagger bleed", &["Reduvia"]),
    ("best arcane scaling weapon", &["Arcane", "Weapon Scaling"]),
    ("Rivers of Blood requirements", &["Rivers of Blood"]),
    ("where is Stormveil Castle", &["Stormveil"]),
    ("Roundtable Hold merchants", &["Roundtable Hold"]),
    ("Ranni questline steps", &["Ranni"]),
    ("Fia quest", &["Fia"]),
    ("summon spirit ashes upgrade", &["Ashes", "Spirit"]),
    ("Mimic Tear ash", &["Mimic Tear"]),
    ("smithing stone locations", &["Smithing Stone"]),
    ("what does arcane do", &["Arcane"]),
    ("rune farming", &["Rune"]),
    ("Godrick the Grafted", &["Godrick"]),
    ("bleed build", &["Blood Loss", "Bleed", "Hemorrhage"]),
    ("Blasphemous Blade", &["Blasphemous Blade"]),
    ("Great Rune activate", &["Great Rune"]),
    ("Margit the Fell Omen tips", &["Margit"]),
    // Asked in Russian. A name has to survive the alphabet it was typed in —
    // "Малению" transliterates to "maleniyu" against the wiki's "Malenia".
    //
    // Only a name can: "Кровослужитель" is a word, and no amount of
    // transliteration turns a Russian word into the English one the wiki uses.
    // That translation is the model's job, and the system prompt tells it to
    // search in English whatever language it was asked in — so what reaches
    // this index is already "Blood Servant".
    ("Как убить Малению", &["Malenia"]),
];

fn main() {
    // `--ask "some words"` runs one query, for looking at a single case.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--ask") {
        let data = dirs::data_dir().unwrap().join("app.roundtable.launcher");
        for query in &args[at + 1..] {
            for edition in [None, Some("convergence")] {
                let found: Vec<String> = ask::matching_titles(&data, edition, query, 6)
                    .into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                println!("{:<14} {query:?}\n     {}", format!("[{edition:?}]"), found.join(" | "));
                for (missing, close) in ask::unknown_words(&data, edition, query) {
                    println!("     no title has {missing:?}; nearest: {}", close.join(", "));
                }
            }
        }
        return;
    }

    if std::env::args().any(|a| a == "--weights") {
        let data = dirs::data_dir().unwrap().join("app.roundtable.launcher");
        for word in ["radahn", "boss", "bosses", "malenia", "flask", "tool", "beat"] {
            println!("{word:<10} {:.3}", ask::word_weight(&data, word));
        }
        return;
    }

    let data = match dirs::data_dir() {
        Some(dir) => dir.join("app.roundtable.launcher"),
        None => {
            println!("no data directory");
            return;
        }
    };

    let mut hit_first = 0;
    let mut hit_top3 = 0;
    let mut missed: Vec<&str> = Vec::new();

    for (query, wanted) in CASES {
        let found: Vec<String> = ask::matching_titles(&data, None, query, 6)
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        if found.is_empty() {
            println!("{query:<38} -> nothing");
            missed.push(query);
            continue;
        }

        let rank = found.iter().position(|title| {
            let lower = title.to_lowercase();
            wanted.iter().any(|want| lower.contains(&want.to_lowercase()))
        });
        match rank {
            Some(0) => hit_first += 1,
            Some(n) if n < 3 => hit_top3 += 1,
            _ => missed.push(query),
        }

        let mark = match rank {
            Some(0) => "  1st",
            Some(n) if n < 3 => "  top",
            Some(_) => "  deep",
            None => "  MISS",
        };
        println!("{mark}  {query:<36} {}", found.join(" | "));
    }

    let total = CASES.len();
    println!(
        "\nfirst: {hit_first}/{total}   top three: {}/{total}   missed: {}",
        hit_first + hit_top3,
        missed.len()
    );
    for query in missed {
        println!("   missed: {query}");
    }
}
