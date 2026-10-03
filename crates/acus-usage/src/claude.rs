use crate::{FileStats, Tokens, is_explore_cmd};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Rec<'a> {
    #[serde(rename = "type")]
    kind: Option<&'a str>,
    subtype: Option<&'a str>,
    cwd: Option<String>,
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
    #[serde(borrow)]
    message: Option<Msg<'a>>,
}

#[derive(Deserialize)]
struct Msg<'a> {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
    #[serde(borrow)]
    content: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    cache_creation: Option<CacheCreation>,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize)]
struct Block<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    id: Option<&'a str>,
    name: Option<&'a str>,
    tool_use_id: Option<&'a str>,
    #[serde(borrow)]
    input: Option<&'a RawValue>,
    #[serde(borrow)]
    content: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct BashInput<'a> {
    command: Option<&'a str>,
}

pub(crate) fn parse(text: &str) -> FileStats {
    let mut st = FileStats::default();
    let mut tool_of: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let Ok(r) = serde_json::from_str::<Rec>(line) else {
            continue;
        };
        if st.cwd.is_empty()
            && let Some(c) = r.cwd
        {
            st.cwd = c;
        }
        if r.kind == Some("system") && r.subtype == Some("compact_boundary") {
            st.compactions += 1;
        }
        let Some(m) = r.message else { continue };
        let content = m.content.map(RawValue::get).unwrap_or("");
        match r.kind {
            Some("assistant") => {
                if let Some(u) = m.usage {
                    let (w5, w1h) = match u.cache_creation {
                        Some(c) => (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens),
                        None => (u.cache_creation_input_tokens, 0),
                    };
                    let t = Tokens {
                        input: u.input_tokens,
                        cache_read: u.cache_read_input_tokens,
                        cache_write_5m: w5,
                        cache_write_1h: w1h,
                        output: u.output_tokens,
                        requests: 1,
                    };
                    st.requests.push((m.id, m.model.unwrap_or_default(), t));
                }
                for b in blocks(content).into_iter().filter(|b| b.kind == "tool_use") {
                    let name = b.name.unwrap_or("?").to_owned();
                    let input = b.input.map(RawValue::get).unwrap_or("");
                    let explore = match name.as_str() {
                        "Read" | "Grep" | "Glob" | "LS" => true,
                        "Bash" => serde_json::from_str::<BashInput>(input)
                            .ok()
                            .and_then(|i| i.command)
                            .is_some_and(is_explore_cmd),
                        _ => false,
                    };
                    if let Some(id) = b.id {
                        tool_of.insert(id.to_owned(), name.clone());
                    }
                    st.calls.push((name, input.len() as u64, explore));
                }
            }
            Some("user") if r.is_meta != Some(true) => {
                if content.starts_with('"') {
                    st.prompts += 1;
                    continue;
                }
                let bs = blocks(content);
                if bs.iter().any(|b| b.kind == "text")
                    && !bs.iter().any(|b| b.kind == "tool_result")
                {
                    st.prompts += 1;
                }
                for b in bs.iter().filter(|b| b.kind == "tool_result") {
                    let name = b
                        .tool_use_id
                        .and_then(|id| tool_of.get(id))
                        .cloned()
                        .unwrap_or_else(|| "?".into());
                    *st.results.entry(name).or_default() +=
                        b.content.map_or(0, |c| c.get().len() as u64);
                }
            }
            _ => {}
        }
    }
    st
}

fn blocks(content: &str) -> Vec<Block<'_>> {
    if content.starts_with('[') {
        serde_json::from_str(content).unwrap_or_default()
    } else {
        vec![]
    }
}
