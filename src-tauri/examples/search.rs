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

/// A question, and a title that would answer it.
const CASES: &[(&str, &str)] = &[
    ("Malenia Blade of Miquella", "Malenia"),
    ("Malenia boss fight how to beat", "Malenia"),
    ("Waterfowl Dance dodge", "Waterfowl"),
    ("how to get more flask charges", "Golden Seed"),
    ("increase flask uses", "Golden Seed"),
    ("sacred tear flask potency", "Sacred Tear"),
    ("Radahn boss", "Radahn"),
    ("Starscourge Radahn location", "Radahn"),
    ("Reduvia dagger bleed", "Reduvia"),
    ("best arcane scaling weapon", "Arcane"),
    ("Rivers of Blood requirements", "Rivers of Blood"),
    ("where is Stormveil Castle", "Stormveil"),
    ("Roundtable Hold merchants", "Roundtable Hold"),
    ("Ranni questline steps", "Ranni"),
    ("Fia quest", "Fia"),
    ("summon spirit ashes upgrade", "Ashes"),
    ("Mimic Tear ash", "Mimic Tear"),
    ("smithing stone locations", "Smithing Stone"),
    ("what does arcane do", "Arcane"),
    ("rune farming", "Rune"),
    ("Godrick the Grafted", "Godrick"),
    ("bleed build", "Blood Loss"),
    ("Blasphemous Blade", "Blasphemous Blade"),
    ("Great Rune activate", "Great Rune"),
    ("Margit the Fell Omen tips", "Margit"),
];

fn main() {
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

        let rank = found
            .iter()
            .position(|title| title.to_lowercase().contains(&wanted.to_lowercase()));
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
