//! Typed judgements through the Jev "System One" API (`POST …/v1/systemone`), which
//! TypeSafe's Jev, Cloudflare's Clef (Workers AI) and Ollama's Clef serve.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};

pub enum Question {
    /// Jev `noul`: probability that the answer is yes.
    YesNo,
    /// Jev `choice`: option name → description.
    Choice(Vec<(String, String)>),
    /// Jev `score`: ordered levels, lowest first.
    Score(Vec<String>),
}

#[derive(Debug)]
pub struct Answer {
    pub value: String,
    /// Probability of `value`.
    pub confidence: f64,
    /// Set for yes/no questions.
    pub yes: Option<bool>,
}

pub struct Endpoint {
    pub url: String,
    pub model: String,
    pub api_key: Option<String>,
}

/// Request body; the single question is named `q`.
pub fn body(
    model: &str,
    state: &str,
    instructions: &str,
    q: &Question,
    images: &[String],
) -> Value {
    let mut question = Map::new();
    let (kind, criteria) = match q {
        Question::YesNo => ("noul", None),
        Question::Choice(opts) => (
            "choice",
            Some(json!(
                opts.iter()
                    .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
                    .collect::<Map<_, _>>()
            )),
        ),
        Question::Score(levels) => ("score", Some(json!(levels))),
    };
    question.insert("type".into(), kind.into());
    question.insert("instructions".into(), instructions.into());
    if let Some(c) = criteria {
        question.insert("criteria".into(), c);
    }
    let mut b = json!({ "model": model, "state": state, "questions": { "q": question } });
    if !images.is_empty() {
        // ponytail: Clef-only extension; base64 encoding assumed (Cloudflare docs do not say).
        b["images"] = json!(images);
    }
    b
}

pub fn parse_answer(resp: &Value, q: &Question) -> Result<Answer> {
    // Workers AI wraps the model output in `{ "result": …, "success": … }`.
    let resp = resp.get("result").filter(|r| r.is_object()).unwrap_or(resp);
    let Some(a) = resp.pointer("/answers/q") else {
        let msg = resp
            .get("error")
            .or_else(|| resp.get("errors"))
            .map_or_else(|| resp.to_string(), Value::to_string);
        bail!(
            "unexpected response: {}",
            msg.chars().take(300).collect::<String>()
        );
    };
    let num = |k: &str| a.get(k).and_then(Value::as_f64);
    Ok(match q {
        Question::YesNo => {
            let p = num("noul").ok_or_else(|| anyhow!("answer without `noul`: {a}"))?;
            let yes = p >= 0.5;
            Answer {
                value: if yes { "yes" } else { "no" }.into(),
                confidence: if yes { p } else { 1.0 - p },
                yes: Some(yes),
            }
        }
        Question::Choice(_) => Answer {
            value: a
                .get("choice")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("answer without `choice`: {a}"))?
                .into(),
            confidence: num("confidence").unwrap_or(f64::NAN),
            yes: None,
        },
        Question::Score(levels) => {
            let s = a
                .get("score")
                .ok_or_else(|| anyhow!("answer without `score`: {a}"))?;
            // Jev returns the expected level index as a float (`1.99`); a label is accepted too.
            let value = match s.as_f64() {
                Some(f) => levels
                    .get(f.round() as usize)
                    .cloned()
                    .unwrap_or_else(|| f.to_string()),
                None => s.as_str().map_or_else(|| s.to_string(), str::to_owned),
            };
            Answer {
                value,
                confidence: num("confidence").unwrap_or(f64::NAN),
                yes: None,
            }
        }
    })
}

pub fn decide(
    ep: &Endpoint,
    state: &str,
    instructions: &str,
    q: &Question,
    images: &[String],
) -> Result<Answer> {
    let mut req = ureq::post(&ep.url);
    if let Some(k) = &ep.api_key {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let resp: Value = match req.send_json(body(&ep.model, state, instructions, q, images)) {
        Ok(mut r) => r.body_mut().read_json().context("response is not JSON")?,
        Err(ureq::Error::StatusCode(code)) => {
            bail!(
                "HTTP {code} from {}\nhint: check the API key variable and the URL in the acus config",
                ep.url
            )
        }
        Err(e) => return Err(e).with_context(|| format!("request to {} failed", ep.url)),
    };
    parse_answer(&resp, q)
}

/// Standard base64 with padding (RFC 4648), for image payloads.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}
