//! Provider abstraction (§4). One interface; Anthropic, OpenAI, Ollama, and
//! Claude Code behind it. Streaming callbacks; usage returned for the budget
//! meter. `base_url` is injectable so gate tests run the full path against a
//! local mock server.
use std::io::{BufRead, BufReader};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Msg {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<Msg>,
    pub max_tokens: u32,
    pub web_search: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug)]
pub struct ChatOutcome {
    pub text: String,
    pub usage: Usage,
    /// Only Claude Code reports its own cost figure.
    pub reported_cost_usd: Option<f64>,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    /// Where the request goes — the egress log destination.
    fn destination(&self) -> String;
    fn model(&self) -> String;
    fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, String>;
}

fn host_of(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Web search tool type by model generation (dynamic-filtering variant needs
/// Opus 4.6+/Sonnet 4.6+; older models get the basic variant).
pub fn anthropic_web_search_type(model: &str) -> &'static str {
    const NEW: [&str; 7] = [
        "opus-4-8", "opus-4-7", "opus-4-6", "sonnet-5", "sonnet-4-6", "fable-5", "mythos-5",
    ];
    if NEW.iter().any(|m| model.contains(m)) {
        "web_search_20260209"
    } else {
        "web_search_20250305"
    }
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String, // default https://api.anthropic.com
}

impl Provider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }
    fn destination(&self) -> String {
        host_of(&self.base_url)
    }
    fn model(&self) -> String {
        self.model.clone()
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, String> {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": req.messages,
            "stream": true,
        });
        if req.web_search {
            body["tools"] = serde_json::json!([{
                "type": anthropic_web_search_type(&self.model),
                "name": "web_search",
                "max_uses": 3,
            }]);
        }
        let resp = reqwest::blocking::Client::new()
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("anthropic request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("anthropic HTTP {status}: {text}"));
        }

        let mut text = String::new();
        let mut usage = Usage::default();
        for line in BufReader::new(resp).lines() {
            let line = line.map_err(|e| format!("stream read error: {e}"))?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let Ok(event): Result<serde_json::Value, _> = serde_json::from_str(data) else {
                continue;
            };
            match event["type"].as_str() {
                Some("message_start") => {
                    usage.input_tokens =
                        event["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
                }
                Some("content_block_delta") => {
                    if let Some(t) = event["delta"]["text"].as_str() {
                        text.push_str(t);
                        on_delta(t);
                    }
                }
                Some("message_delta") => {
                    if let Some(out) = event["usage"]["output_tokens"].as_u64() {
                        usage.output_tokens = out;
                    }
                }
                _ => {}
            }
        }
        Ok(ChatOutcome {
            text,
            usage,
            reported_cost_usd: None,
        })
    }
}

// ---------------------------------------------------------------------------
// OpenAI
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String, // default https://api.openai.com
}

impl Provider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }
    fn destination(&self) -> String {
        host_of(&self.base_url)
    }
    fn model(&self) -> String {
        self.model.clone()
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, String> {
        let mut messages = vec![serde_json::json!({"role": "system", "content": req.system})];
        messages.extend(
            req.messages
                .iter()
                .map(|m| serde_json::json!({"role": m.role, "content": m.content})),
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "max_completion_tokens": req.max_tokens,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        let resp = reqwest::blocking::Client::new()
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("openai request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("openai HTTP {status}: {text}"));
        }

        let mut text = String::new();
        let mut usage = Usage::default();
        for line in BufReader::new(resp).lines() {
            let line = line.map_err(|e| format!("stream read error: {e}"))?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data.trim() == "[DONE]" {
                break;
            }
            let Ok(event): Result<serde_json::Value, _> = serde_json::from_str(data) else {
                continue;
            };
            if let Some(t) = event["choices"][0]["delta"]["content"].as_str() {
                text.push_str(t);
                on_delta(t);
            }
            if event["usage"].is_object() {
                usage.input_tokens = event["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
                usage.output_tokens = event["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            }
        }
        Ok(ChatOutcome {
            text,
            usage,
            reported_cost_usd: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Ollama
// ---------------------------------------------------------------------------

pub struct OllamaProvider {
    pub model: String,
    pub base_url: String, // default http://localhost:11434
}

impl Provider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }
    fn destination(&self) -> String {
        host_of(&self.base_url)
    }
    fn model(&self) -> String {
        self.model.clone()
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, String> {
        let mut messages = vec![serde_json::json!({"role": "system", "content": req.system})];
        messages.extend(
            req.messages
                .iter()
                .map(|m| serde_json::json!({"role": m.role, "content": m.content})),
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });
        let resp = reqwest::blocking::Client::new()
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .map_err(|e| format!("ollama request failed (is Ollama running?): {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("ollama HTTP {status}: {text}"));
        }

        let mut text = String::new();
        let mut usage = Usage::default();
        for line in BufReader::new(resp).lines() {
            let line = line.map_err(|e| format!("stream read error: {e}"))?;
            let Ok(event): Result<serde_json::Value, _> = serde_json::from_str(&line) else {
                continue;
            };
            if let Some(t) = event["message"]["content"].as_str() {
                if !t.is_empty() {
                    text.push_str(t);
                    on_delta(t);
                }
            }
            if event["done"].as_bool() == Some(true) {
                usage.input_tokens = event["prompt_eval_count"].as_u64().unwrap_or(0);
                usage.output_tokens = event["eval_count"].as_u64().unwrap_or(0);
            }
        }
        Ok(ChatOutcome {
            text,
            usage,
            reported_cost_usd: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Claude Code (shell-out; opt-in — §4.1 path 2)
// ---------------------------------------------------------------------------

/// Everything Claude Code could touch gets disallowed. It is an agent, not a
/// completion endpoint; unconstrained it will attempt filesystem and shell
/// operations. Asserted in the M2 gate.
pub const CLAUDE_DISALLOWED_TOOLS: &str = "Bash,Edit,Write,Read,Glob,Grep,WebFetch,WebSearch,NotebookEdit,Task,TodoWrite,BashOutput,KillShell,SlashCommand,Skill,EnterPlanMode,ExitPlanMode";

/// Pure so the gate can assert lockdown without spawning anything.
pub fn build_claude_args(prompt: &str, system: &str) -> Vec<String> {
    vec![
        "-p".into(),
        prompt.into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--tools".into(),
        "".into(), // disable the entire built-in tool set
        "--disallowedTools".into(),
        CLAUDE_DISALLOWED_TOOLS.into(),
        "--max-turns".into(),
        "1".into(),
        "--no-session-persistence".into(),
        "--system-prompt".into(),
        system.into(),
    ]
}

/// Locate the user's claude binary. Never bundled, never authenticated by us.
pub fn detect_claude_code() -> Option<String> {
    let out = std::process::Command::new("where.exe")
        .arg("claude")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub struct ClaudeCodeProvider {
    pub exe: String,
}

impl ClaudeCodeProvider {
    /// History flattens into the single -p prompt: the CLI is one-shot.
    fn flatten_prompt(req: &ChatRequest) -> String {
        if req.messages.len() == 1 {
            return req.messages[0].content.clone();
        }
        let mut s = String::from("Conversation so far:\n");
        for m in &req.messages[..req.messages.len() - 1] {
            s.push_str(&format!("{}: {}\n", m.role, m.content));
        }
        s.push_str(&format!(
            "\nRespond to this message: {}",
            req.messages.last().map(|m| m.content.as_str()).unwrap_or("")
        ));
        s
    }
}

impl Provider for ClaudeCodeProvider {
    fn name(&self) -> &'static str {
        "claude_code"
    }
    fn destination(&self) -> String {
        // The CLI talks to Anthropic with the user's own auth.
        "api.anthropic.com (via claude CLI)".into()
    }
    fn model(&self) -> String {
        "claude-code-default".into()
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatOutcome, String> {
        use std::process::{Command, Stdio};
        let args = build_claude_args(&Self::flatten_prompt(req), &req.system);
        let mut cmd = if self.exe.to_lowercase().ends_with(".cmd")
            || self.exe.to_lowercase().ends_with(".bat")
        {
            let mut c = Command::new("cmd");
            c.arg("/c").arg(&self.exe).args(&args);
            c
        } else {
            let mut c = Command::new(&self.exe);
            c.args(&args);
            c
        };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start claude CLI: {e}"))?;

        let stdout = child.stdout.take().ok_or("no stdout from claude CLI")?;
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut cost = None;
        let mut result_err: Option<String> = None;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(event): Result<serde_json::Value, _> = serde_json::from_str(&line) else {
                continue;
            };
            match event["type"].as_str() {
                Some("stream_event") => {
                    let inner = &event["event"];
                    if inner["type"] == "content_block_delta" {
                        if let Some(t) = inner["delta"]["text"].as_str() {
                            text.push_str(t);
                            on_delta(t);
                        }
                    }
                }
                Some("result") => {
                    usage.input_tokens = event["usage"]["input_tokens"].as_u64().unwrap_or(0);
                    usage.output_tokens = event["usage"]["output_tokens"].as_u64().unwrap_or(0);
                    cost = event["total_cost_usd"].as_f64();
                    if event["is_error"].as_bool() == Some(true) {
                        result_err = Some(
                            event["result"].as_str().unwrap_or("claude CLI error").to_string(),
                        );
                    }
                    // Fallback: some result payloads carry the final text only here.
                    if text.is_empty() {
                        if let Some(t) = event["result"].as_str() {
                            text.push_str(t);
                            on_delta(t);
                        }
                    }
                }
                _ => {}
            }
        }
        let status = child.wait().map_err(|e| format!("claude CLI wait: {e}"))?;
        if let Some(e) = result_err {
            return Err(format!("claude CLI: {e}"));
        }
        if !status.success() && text.is_empty() {
            return Err(format!("claude CLI exited with {status}"));
        }
        Ok(ChatOutcome {
            text,
            usage,
            reported_cost_usd: cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use std::net::TcpListener;

    /// One-shot mock HTTP server: accepts a single connection, returns a
    /// canned body, hands back the request it saw.
    fn mock_server(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 65536];
            let mut request = Vec::new();
            // Read until end of headers + declared content-length body.
            loop {
                let n = std::io::Read::read(&mut stream, &mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length = text
                        .lines()
                        .find(|l| l.to_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.split(':').nth(1))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (format!("http://{addr}"), handle)
    }

    fn req() -> ChatRequest {
        ChatRequest {
            system: "You are a test.".into(),
            messages: vec![Msg {
                role: "user".into(),
                content: "hello".into(),
            }],
            max_tokens: 100,
            web_search: false,
        }
    }

    const ANTHROPIC_SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n";

    const OPENAI_SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\ndata: [DONE]\n";

    const OLLAMA_LINES: &str = "{\"message\":{\"content\":\"Hello \"},\"done\":false}\n{\"message\":{\"content\":\"world\"},\"done\":false}\n{\"message\":{\"content\":\"\"},\"done\":true,\"prompt_eval_count\":12,\"eval_count\":5}\n";

    /// The gate's "same prompt runs on all three through one interface".
    #[test]
    fn same_request_streams_through_all_three_http_providers() {
        let cases: Vec<(Box<dyn Fn(String) -> Box<dyn Provider>>, &str)> = vec![
            (
                Box::new(|url| {
                    Box::new(AnthropicProvider {
                        api_key: "k".into(),
                        model: "claude-opus-4-8".into(),
                        base_url: url,
                    })
                }),
                ANTHROPIC_SSE,
            ),
            (
                Box::new(|url| {
                    Box::new(OpenAiProvider {
                        api_key: "k".into(),
                        model: "gpt-5.4".into(),
                        base_url: url,
                    })
                }),
                OPENAI_SSE,
            ),
            (
                Box::new(|url| {
                    Box::new(OllamaProvider {
                        model: "llama3.2".into(),
                        base_url: url,
                    })
                }),
                OLLAMA_LINES,
            ),
        ];
        for (make, body) in cases {
            let (url, server) = mock_server(body);
            let provider = make(url);
            let mut deltas = Vec::new();
            let outcome = provider
                .stream(&req(), &mut |d| deltas.push(d.to_string()))
                .unwrap_or_else(|e| panic!("{} failed: {e}", provider.name()));
            assert_eq!(outcome.text, "Hello world", "{}", provider.name());
            assert_eq!(deltas.len(), 2, "{} streamed deltas", provider.name());
            assert_eq!(outcome.usage.input_tokens, 12, "{}", provider.name());
            assert_eq!(outcome.usage.output_tokens, 5, "{}", provider.name());
            let request_seen = server.join().unwrap();
            assert!(
                request_seen.contains("hello"),
                "{} sent the prompt",
                provider.name()
            );
        }
    }

    #[test]
    fn claude_args_lock_down_tools() {
        let args = build_claude_args("hi", "sys");
        let joined = args.join(" ");
        assert!(joined.contains("--disallowedTools"));
        for tool in ["Bash", "Edit", "Write", "Read", "WebFetch", "Task"] {
            assert!(
                CLAUDE_DISALLOWED_TOOLS.contains(tool),
                "{tool} must be disallowed"
            );
        }
        // Built-in tool set disabled outright.
        let tools_idx = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[tools_idx + 1], "");
        assert!(joined.contains("--max-turns 1"));
        assert!(joined.contains("--no-session-persistence"));
    }

    #[test]
    fn web_search_tool_type_tracks_model_generation() {
        assert_eq!(
            anthropic_web_search_type("claude-opus-4-8"),
            "web_search_20260209"
        );
        assert_eq!(
            anthropic_web_search_type("claude-haiku-4-5"),
            "web_search_20250305"
        );
    }

    /// Real end-to-end through the user's authenticated claude CLI.
    /// Run explicitly by the M2 gate: cargo test -- --ignored real_claude
    #[test]
    #[ignore]
    fn real_claude_code_roundtrip() {
        let exe = detect_claude_code().expect("claude not on PATH");
        let provider = ClaudeCodeProvider { exe };
        let mut req = req();
        req.messages[0].content = "Reply with exactly the word: pong".into();
        let outcome = provider.stream(&req, &mut |_| {}).expect("claude call failed");
        assert!(
            outcome.text.to_lowercase().contains("pong"),
            "unexpected reply: {}",
            outcome.text
        );
        assert!(outcome.usage.output_tokens > 0);
    }
}
