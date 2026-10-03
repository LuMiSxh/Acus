//! Token and tool-call analytics over Claude Code (`~/.claude/projects`) and Codex
//! (`~/.codex/sessions`) JSONL transcripts.

mod claude;
mod codex;
mod tools;

pub use tools::group;

use acus_walk::{WalkOpts, walk};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

#[derive(Debug, Default, Clone)]
pub struct UsageOpts {
    /// Defaults to `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`.
    pub claude_root: Option<PathBuf>,
    /// Defaults to `$CODEX_HOME/sessions` or `~/.codex/sessions`.
    pub codex_root: Option<PathBuf>,
    /// Only transcripts modified within this many days.
    pub days: Option<u64>,
    /// Case-insensitive substring of the session's working directory.
    pub project: Option<String>,
}

/// Relative cost in input-token equivalents (Anthropic list-price ratios):
/// cache read 0.1x, 5-minute cache write 1.25x, 1-hour cache write 2x, output 5x.
pub const WEIGHTS: [(&str, f64); 5] = [
    ("input", 1.0),
    ("cache_read", 0.1),
    ("cache_write_5m", 1.25),
    ("cache_write_1h", 2.0),
    ("output", 5.0),
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub output: u64,
    pub requests: u64,
}

impl Tokens {
    pub fn classes(&self) -> [u64; 5] {
        [
            self.input,
            self.cache_read,
            self.cache_write_5m,
            self.cache_write_1h,
            self.output,
        ]
    }

    pub fn weighted(&self) -> f64 {
        self.classes()
            .iter()
            .zip(WEIGHTS)
            .map(|(n, (_, w))| *n as f64 * w)
            .sum()
    }

    /// Tokens the model had to look at for this request.
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write_5m += o.cache_write_5m;
        self.cache_write_1h += o.cache_write_1h;
        self.output += o.output;
        self.requests += o.requests;
    }
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub agent: &'static str,
    pub project: String,
    pub id: String,
    pub model: String,
    pub tokens: Tokens,
    pub weighted: f64,
    pub max_context: u64,
    pub compactions: u32,
    pub prompts: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct ToolStat {
    pub name: String,
    pub calls: u64,
    pub input_chars: u64,
    pub result_chars: u64,
}

/// Runs of ≥3 consecutive search/read tool calls.
#[derive(Debug, Default, Serialize)]
pub struct Chains {
    pub count: u64,
    pub calls_in_chains: u64,
    pub tool_calls: u64,
    pub longest: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub total: Tokens,
    pub weighted: f64,
    pub prompts: u64,
    /// Requests' context size percentiles: p50, p90, p99.
    pub context: [u64; 3],
    pub models: Vec<(String, Tokens)>,
    pub sessions: Vec<Session>,
    pub tools: Vec<ToolStat>,
    pub chains: Chains,
    /// Tool-call categories → calls per agent `[claude, codex]`.
    #[serde(skip)]
    pub categories: BTreeMap<&'static str, [u64; 2]>,
}

/// What one transcript file contributes.
#[derive(Default)]
pub(crate) struct FileStats {
    pub agent: &'static str,
    pub session: String,
    pub cwd: String,
    /// (dedup id, model, tokens of one request)
    pub requests: Vec<(Option<String>, String, Tokens)>,
    pub compactions: u32,
    pub prompts: u64,
    /// (tool name, input chars, is search/read)
    pub calls: Vec<(String, u64, bool)>,
    pub results: HashMap<String, u64>,
    /// Category hits per tool call, see `tools::classify`.
    pub categories: Vec<&'static str>,
}

/// Shell commands whose first real step only searches or reads (`cd x && rg …` counts).
pub(crate) fn is_explore_cmd(cmd: &str) -> bool {
    for part in cmd.split(['&', ';', '|', '\n']) {
        let first = part.split_whitespace().next().unwrap_or("");
        match first.rsplit('/').next().unwrap_or(first) {
            "" | "cd" | "pwd" | "echo" | "printf" | "set" => continue,
            "grep" | "rg" | "sed" | "cat" | "head" | "tail" | "find" | "fd" | "ls" | "wc"
            | "acus" | "tree" | "nl" => return true,
            _ => return false,
        }
    }
    false
}

fn default_root(env: &str, home_sub: &str, sub: &str) -> Option<PathBuf> {
    std::env::var_os(env)
        .map(|d| PathBuf::from(d).join(sub))
        .or_else(|| std::env::home_dir().map(|h| h.join(home_sub).join(sub)))
}

pub fn report(o: &UsageOpts) -> Result<Report> {
    let claude = o
        .claude_root
        .clone()
        .or_else(|| default_root("CLAUDE_CONFIG_DIR", ".claude", "projects"));
    let codex = o
        .codex_root
        .clone()
        .or_else(|| default_root("CODEX_HOME", ".codex", "sessions"));
    let cutoff = o
        .days
        .map(|d| SystemTime::now() - Duration::from_secs(d * 86_400));
    let files = Mutex::new(Vec::new());
    for (root, agent) in [(claude, "claude"), (codex, "codex")] {
        let Some(root) = root.filter(|r| r.is_dir()) else {
            continue;
        };
        let opts = WalkOpts {
            roots: vec![root.clone()],
            include: vec!["*.jsonl".into()],
            hidden: true,
            ..Default::default()
        };
        walk(&opts, |p| {
            if cutoff.is_some_and(|c| p.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < c))
            {
                return;
            }
            // ponytail: unreadable or foreign files are skipped; transcripts are best-effort data.
            let Ok(text) = std::fs::read_to_string(p) else {
                return;
            };
            let mut st = if agent == "claude" {
                claude::parse(&text)
            } else {
                codex::parse(&text)
            };
            st.agent = agent;
            st.session = session_id(&root, p, agent);
            files.lock().unwrap().push(st);
        })?;
    }
    let mut files = files.into_inner().unwrap();
    if let Some(q) = &o.project {
        let q = q.to_lowercase();
        files.retain(|f| f.cwd.to_lowercase().contains(&q));
    }
    files.sort_by(|a, b| a.session.cmp(&b.session));
    Ok(merge(files))
}

/// Claude: `<project>/<session>.jsonl` and `<project>/<session>/subagents/*.jsonl`
/// share one session; Codex: one rollout file per session.
fn session_id(root: &Path, p: &Path, agent: &str) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let part = if agent == "claude" {
        rel.iter().nth(1)
    } else {
        rel.file_name()
    };
    let s = part.map(|s| s.to_string_lossy()).unwrap_or_default();
    let s = s.trim_end_matches(".jsonl");
    // `rollout-<timestamp>-<uuid>`: keep the uuid.
    s.get(s.len().saturating_sub(36)..).unwrap_or(s).to_owned()
}

fn merge(files: Vec<FileStats>) -> Report {
    let mut r = Report::default();
    let mut seen = HashSet::new();
    let mut sessions: HashMap<(&'static str, String), Session> = HashMap::new();
    let mut models: HashMap<String, Tokens> = HashMap::new();
    let mut tools: HashMap<String, ToolStat> = HashMap::new();
    let mut contexts = Vec::new();
    for f in files {
        let project = Path::new(&f.cwd)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let s = sessions
            .entry((f.agent, f.session.clone()))
            .or_insert_with(|| Session {
                agent: f.agent,
                project: project.clone(),
                id: f.session.clone(),
                model: String::new(),
                tokens: Tokens::default(),
                weighted: 0.0,
                max_context: 0,
                compactions: 0,
                prompts: 0,
            });
        if s.project.is_empty() {
            s.project = project;
        }
        for c in &f.categories {
            r.categories.entry(c).or_default()[usize::from(f.agent == "codex")] += 1;
        }
        for (id, model, t) in f.requests {
            if id.is_some_and(|id| !seen.insert(id)) {
                continue;
            }
            s.tokens.add(&t);
            s.max_context = s.max_context.max(t.context());
            contexts.push(t.context());
            models.entry(model.clone()).or_default().add(&t);
            r.total.add(&t);
            if s.model.is_empty() || s.model == "<synthetic>" {
                s.model = model;
            }
        }
        s.compactions += f.compactions;
        s.prompts += f.prompts;
        r.prompts += f.prompts;
        let mut run = 0u64;
        for (name, input, explore) in &f.calls {
            let t = tools.entry(name.clone()).or_insert_with(|| ToolStat {
                name: name.clone(),
                ..Default::default()
            });
            t.calls += 1;
            t.input_chars += input;
            r.chains.tool_calls += 1;
            if *explore {
                run += 1;
            } else {
                close_chain(&mut r.chains, &mut run);
            }
        }
        close_chain(&mut r.chains, &mut run);
        for (name, chars) in f.results {
            tools
                .entry(name.clone())
                .or_insert_with(|| ToolStat {
                    name,
                    ..Default::default()
                })
                .result_chars += chars;
        }
    }
    r.weighted = r.total.weighted();
    contexts.sort_unstable();
    for (i, q) in [0.5, 0.9, 0.99].iter().enumerate() {
        r.context[i] = contexts
            .get((contexts.len() as f64 * q) as usize)
            .copied()
            .unwrap_or(0);
    }
    r.sessions = sessions
        .into_values()
        .filter(|s| s.tokens.requests > 0 || s.prompts > 0)
        .collect();
    for s in &mut r.sessions {
        s.weighted = s.tokens.weighted();
    }
    r.sessions
        .sort_by(|a, b| b.weighted.total_cmp(&a.weighted).then(a.id.cmp(&b.id)));
    r.models = models.into_iter().collect();
    r.models
        .sort_by(|a, b| b.1.weighted().total_cmp(&a.1.weighted()));
    r.tools = tools.into_values().collect();
    r.tools
        .sort_by(|a, b| b.calls.cmp(&a.calls).then(a.name.cmp(&b.name)));
    r
}

fn close_chain(c: &mut Chains, run: &mut u64) {
    if *run >= 3 {
        c.count += 1;
        c.calls_in_chains += *run;
        c.longest = c.longest.max(*run);
    }
    *run = 0;
}
