//! `xenolith note "<title>" [--body <text>] [--page <slug>]... [--space <id>] [--json]`: a note
//! from you to the curator. It goes through the running service with the owner key
//! (owner_client.rs), so it is stamped as yours (by: cli, source: user): the one kind of note,
//! with the window's, that may change a preference or ask to forget something without waiting.

use serde_json::json;

fn fail(msg: &str) -> ! {
    eprintln!("xenolith note: {msg}");
    std::process::exit(1)
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let opts = |name: &str| args.windows(2).filter(|w| w[0] == name).map(|w| w[1].clone()).collect::<Vec<_>>();
    let mut title = None;
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if matches!(a.as_str(), "--body" | "--page" | "--space") {
            skip = true;
        } else if !a.starts_with("--") && title.is_none() {
            title = Some(a.clone());
        }
    }
    let Some(title) = title.filter(|t| !t.trim().is_empty()) else {
        eprintln!("usage: xenolith note \"<title>\" [--body <text>] [--page <slug>]... [--space <id>] [--json]");
        std::process::exit(2)
    };
    let mut body = json!({ "title": title, "body": opts("--body").first().cloned().unwrap_or_default(), "pages": opts("--page") });
    if let Some(space) = opts("--space").first() {
        body["space"] = json!(space);
    }
    match crate::owner_client::post("/api/note", &body) {
        Ok((201, v)) => {
            if json_out {
                println!("{v}");
            } else if v["duplicate"] == true {
                println!("Already saved as note {}.", v["note"].as_str().unwrap_or(""));
            } else {
                println!("Saved note {} for the curator.", v["note"].as_str().unwrap_or(""));
            }
            std::process::exit(0)
        }
        Ok((_, v)) => fail(v["error"].as_str().unwrap_or("the service refused")),
        Err(e) => fail(&e),
    }
}
