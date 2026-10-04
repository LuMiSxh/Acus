use crate::Outcome;
use crate::config::Config;
use crate::fmt::Out;
use anyhow::Result;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// The question, e.g. "Did the build pass?".
    question: String,
    /// Text to judge; default: stdin.
    #[arg(long)]
    state: Option<String>,
    /// Read the text to judge from a file.
    #[arg(long, conflicts_with = "state")]
    state_file: Option<PathBuf>,
    /// Choice option as NAME=DESCRIPTION (repeatable); default is yes/no.
    #[arg(long = "choice", value_name = "NAME=DESC")]
    choices: Vec<String>,
    /// Score levels, lowest first, comma-separated.
    #[arg(long, value_delimiter = ',', conflicts_with = "choices")]
    score: Vec<String>,
    /// Image to judge (Clef only; repeatable, at most 4).
    #[arg(long = "image")]
    images: Vec<PathBuf>,
}

#[cfg(not(feature = "cmd-decide"))]
pub fn run(_: Args, _: &Out, _: &Config) -> Result<Outcome> {
    anyhow::bail!(
        "acus was built without `decide`\nhint: cargo install --path crates/acus-cli --features cmd-decide"
    )
}

#[cfg(feature = "cmd-decide")]
pub fn run(a: Args, out: &Out, cfg: &Config) -> Result<Outcome> {
    use crate::fmt::Format;
    use acus_decide::{Endpoint, Question, base64, decide};
    use anyhow::{Context, bail};
    use std::io::{IsTerminal, Read};

    let d = &cfg.decide;
    let path = cfg.path.display();
    if d.enabled == Some(false) {
        bail!(
            "decide is disabled\nhint: set [decide] enabled = true in {path} or ACUS_DECIDE_ENABLED=1"
        );
    }
    let Some(url) = d.url.clone() else {
        bail!("no decide endpoint configured\nhint: set ACUS_DECIDE_URL or [decide] url in {path}");
    };
    let key_env = d.api_key_env.as_deref().unwrap_or("TYPESAFE_API_KEY");
    let Some(api_key) = std::env::var(key_env).ok().filter(|k| !k.trim().is_empty()) else {
        // Exit 2, not 1: a missing key must not read as a "no" answer in scripts.
        eprintln!(
            "warning: no API key in ${key_env}, decide not used\nhint: export {key_env}, or set [decide] api_key_env in {path}"
        );
        return Ok(Outcome::Exit(2));
    };
    let ep = Endpoint {
        url,
        model: d.model.clone().unwrap_or_else(|| "jev-latest".into()),
        api_key: Some(api_key),
    };
    let state = match (a.state, a.state_file) {
        (Some(s), _) => s,
        (_, Some(f)) => {
            std::fs::read_to_string(&f).with_context(|| format!("cannot read {}", f.display()))?
        }
        _ if std::io::stdin().is_terminal() => {
            bail!("nothing to judge\nhint: pass --state TEXT, --state-file PATH or pipe text in")
        }
        _ => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
    };
    let q = if !a.choices.is_empty() {
        let opts = a.choices.iter().map(|c| {
            let (k, v) = c.split_once('=').unwrap_or((c, c));
            (k.trim().to_owned(), v.trim().to_owned())
        });
        Question::Choice(opts.collect())
    } else if !a.score.is_empty() {
        Question::Score(a.score)
    } else {
        Question::YesNo
    };
    if a.images.len() > 4 {
        bail!("at most 4 images");
    }
    let images = a
        .images
        .iter()
        .map(|p| {
            std::fs::read(p)
                .map(|b| base64(&b))
                .with_context(|| format!("cannot read {}", p.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let ans = decide(&ep, &state, &a.question, &q, &images)?;
    if out.format == Format::Json {
        println!(
            "{}",
            serde_json::json!({ "answer": ans.value, "confidence": ans.confidence, "yes": ans.yes })
        );
    } else {
        println!("{} {:.2}", ans.value, ans.confidence);
    }
    // Yes/no maps onto the exit code so scripts can branch on it.
    Ok(if ans.yes == Some(false) {
        Outcome::Empty
    } else {
        Outcome::Found
    })
}
