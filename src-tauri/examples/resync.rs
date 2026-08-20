//! Re-mirrors a wiki's titles, so a change to what gets indexed takes effect.
//!
//! `cargo run --example resync`

#[tokio::main]
async fn main() {
    let Some(data) = dirs::data_dir().map(|d| d.join("app.roundtable.launcher")) else {
        println!("no data directory");
        return;
    };
    let http = reqwest::Client::builder()
        .user_agent("Roundtable")
        .build()
        .expect("client");

    for id in ["eldenring", "convergence", "eldenring-ru"] {
        let Some(source) = roundtable_lib::wiki::source(id) else {
            continue;
        };
        let before = roundtable_lib::wiki::titles(&data, id).len();
        print!("{id}: {before} titles -> ");
        match roundtable_lib::wiki::sync_titles(&http, &data, source, |_| {}).await {
            Ok(count) => println!("{count}"),
            Err(error) => println!("failed: {error}"),
        }
    }

    // What each English article is called in Russian, so a name never has to be
    // translated by guesswork.
    for id in ["eldenring", "convergence"] {
        let Some(source) = roundtable_lib::wiki::source(id) else {
            continue;
        };
        print!("{id}: russian names -> ");
        match roundtable_lib::wiki::sync_langlinks(&http, &data, source, "ru", |_| {}).await {
            Ok(count) => println!("{count}"),
            Err(error) => println!("failed: {error}"),
        }
    }
}
