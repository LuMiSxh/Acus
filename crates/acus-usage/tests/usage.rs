use acus_usage::{UsageOpts, report};
use std::fs;

const CLAUDE: &str = r#"{"type":"user","cwd":"/x/Proj","message":{"role":"user","content":"do it"}}
{"type":"assistant","cwd":"/x/Proj","message":{"id":"m1","model":"opus","usage":{"input_tokens":10,"cache_read_input_tokens":1000,"cache_creation":{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0},"output_tokens":20},"content":[{"type":"tool_use","id":"t1","name":"Grep","input":{"pattern":"x"}}]}}
{"type":"assistant","cwd":"/x/Proj","message":{"id":"m1","model":"opus","usage":{"input_tokens":10,"cache_read_input_tokens":1000,"cache_creation":{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0},"output_tokens":20},"content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"a"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"0123456789"}]}}
{"type":"assistant","message":{"id":"m2","model":"opus","usage":{"input_tokens":0,"cache_read_input_tokens":2000,"output_tokens":5},"content":[{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"sed -n 1,5p a"}}]}}
{"type":"assistant","message":{"id":"m3","model":"opus","usage":{"input_tokens":0,"cache_read_input_tokens":3000,"output_tokens":5},"content":[{"type":"tool_use","id":"t4","name":"Edit","input":{}}]}}
{"type":"system","subtype":"compact_boundary"}
"#;

const CODEX: &str = r#"{"type":"session_meta","payload":{"cwd":"/y/Other"}}
{"type":"turn_context","payload":{"model":"gpt-x"}}
{"type":"event_msg","payload":{"type":"user_message","message":"hi"}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"rg foo\"}","call_id":"c1"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":110},"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"cache_write_input_tokens":0,"output_tokens":10}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":110},"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"cache_write_input_tokens":0,"output_tokens":10}}}}
{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"abc"}}
"#;

#[test]
fn aggregates_both_agents() {
    let d = tempfile::tempdir().unwrap();
    let claude = d.path().join("claude");
    let codex = d.path().join("codex");
    fs::create_dir_all(claude.join("-x-Proj/s1/subagents")).unwrap();
    fs::create_dir_all(codex.join("2026/10/03")).unwrap();
    fs::write(claude.join("-x-Proj/s1.jsonl"), CLAUDE).unwrap();
    fs::write(codex.join("2026/10/03/rollout-a.jsonl"), CODEX).unwrap();
    let r = report(&UsageOpts {
        claude_root: Some(claude),
        codex_root: Some(codex),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(r.prompts, 2);
    let s: Vec<_> = r
        .sessions
        .iter()
        .map(|s| (s.agent, s.project.as_str(), s.tokens.requests))
        .collect();
    assert_eq!(
        s,
        [("claude", "Proj", 3), ("codex", "Other", 1)],
        "dedup by message id / total"
    );
    let c = &r.sessions[0];
    assert_eq!(
        (c.tokens.cache_read, c.tokens.cache_write_1h, c.compactions),
        (6000, 100, 1)
    );
    assert_eq!(c.max_context, 3000);
    let x = &r.sessions[1].tokens;
    assert_eq!((x.input, x.cache_read, x.output), (20, 80, 10));

    let grep = r.tools.iter().find(|t| t.name == "Grep").unwrap();
    assert_eq!((grep.calls, grep.result_chars), (1, 12));
    // Grep, Read, sed form one chain of 3; Edit ends it.
    assert_eq!(
        (r.chains.count, r.chains.calls_in_chains, r.chains.longest),
        (1, 3, 3)
    );
    assert_eq!(r.chains.tool_calls, 5);
}

#[test]
fn project_filter() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("p")).unwrap();
    fs::write(d.path().join("p/s.jsonl"), CLAUDE).unwrap();
    let r = report(&UsageOpts {
        claude_root: Some(d.path().into()),
        codex_root: Some(d.path().join("none")),
        project: Some("nomatch".into()),
        ..Default::default()
    })
    .unwrap();
    assert!(r.sessions.is_empty());
}
