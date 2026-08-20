//! What the web search actually returns, for when the assistant says it found
//! nothing and the question is whether that is the search or the model.
//!
//!     cargo run --example web -- "convergence mod patch notes"

#[tokio::main]
async fn main() {
    let query: Vec<String> = std::env::args().skip(1).collect();
    let query = query.join(" ");
    if query.is_empty() {
        eprintln!("usage: cargo run --example web -- <query>");
        return;
    }

    let http = reqwest::Client::new();
    let found = roundtable_lib::web::search(&http, &query, 8).await;

    if found.is_empty() {
        println!("nothing came back — both engines refused or changed shape");
        return;
    }
    for one in &found {
        println!("{}\n  {}\n  {}\n", one.title, one.url, one.summary);
    }
}
