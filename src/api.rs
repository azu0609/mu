use crate::{
    Result, process,
    session::{Block, Kind, Usage},
    tools,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::Duration,
};

pub enum Event {
    Push(Block),
    Delta(usize, String),
    Set(usize, Block),
    Done(Result<Step>),
}
pub struct Step {
    pub items: Vec<Value>,
    pub blocks: Vec<Block>,
    pub usage: Usage,
    pub again: bool,
}
pub struct Request {
    pub instructions: String,
    pub model: String,
    pub effort: Option<String>,
    pub input: Vec<Value>,
    pub cwd: PathBuf,
}
impl Request {
    pub fn body(&self) -> Value {
        let mut body = json!({
            "model": self.model, "instructions": self.instructions, "input": self.input,
            "stream": true, "store": false, "include": ["reasoning.encrypted_content"],
            "tools": [{"type":"function", "name":"bash", "description":"Special bash commands: view_image",
                "parameters":{"type":"object", "properties":{"command":{"type":"string"}, "timeoutMs":{"type":"integer"}}, "required":["command"], "additionalProperties":false}, "strict":false}]
        });
        if let Some(effort) = &self.effort {
            body["reasoning"] = json!({"effort":effort, "summary":"auto"});
        }
        body
    }
}

struct Live<'a> {
    tx: &'a Sender<Event>,
    blocks: Vec<Block>,
}
impl Live<'_> {
    fn push(&mut self, kind: Kind, text: impl Into<String>) -> usize {
        let b = Block::new(kind, text);
        let i = self.blocks.len();
        self.blocks.push(b.clone());
        let _ = self.tx.send(Event::Push(b));
        i
    }
    fn delta(&mut self, i: usize, text: &str) {
        self.blocks[i].text.push_str(text);
        let _ = self.tx.send(Event::Delta(i, text.into()));
    }
    fn set(&mut self, i: usize, kind: Kind, text: String) {
        let b = Block::new(kind, text);
        self.blocks[i] = b.clone();
        let _ = self.tx.send(Event::Set(i, b));
    }
}

#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: String,
}
impl Sse {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() + self.data.len() > 32 * 1024 * 1024 {
            return Err("SSE event exceeds 32 MiB".into());
        }
        let mut events = vec![];
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8(self.pending.drain(..=end).collect())?;
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() && self.data.trim() != "[DONE]" {
                    events.push(serde_json::from_str(&self.data)?);
                }
                self.data.clear();
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
            }
        }
        Ok(events)
    }
}

fn display(item: &Value) -> Option<(Kind, String)> {
    let texts = |field: &str, typ: &str| {
        item[field]
            .as_array()
            .map(|a| {
                a.iter().filter(|v| v["type"] == typ).filter_map(|v| v["text"].as_str()).collect::<Vec<_>>().join("\n")
            })
            .unwrap_or_default()
    };
    match item["type"].as_str()? {
        "message" => {
            let mut text = texts("content", "output_text");
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if let Some(refusal) = part["refusal"].as_str() {
                        text.push_str(refusal);
                    }
                }
            }
            Some((Kind::Agent, text))
        }
        "reasoning" => Some((Kind::Thought, texts("summary", "summary_text"))),
        "function_call" => {
            let raw = item["arguments"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
            let command = args["command"].as_str().unwrap_or(raw);
            Some((Kind::Call, format!("{}: {}", item["name"].as_str().unwrap_or("tool"), command)))
        }
        _ => None,
    }
}

pub fn step(request: Request, cancel: Arc<AtomicBool>, tx: &Sender<Event>) -> Result<Step> {
    let mut live = Live { tx, blocks: vec![] };
    let mut slots = BTreeMap::new();
    let mut items = BTreeMap::new();
    let mut completed = None;
    let mut sse = Sse::default();
    let mut errors = vec![];
    let mut preview = vec![];
    let base = env::var("MU_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8317/v1".into());
    let url = format!("{}/responses", base.trim_end_matches('/'));
    let mut curl = Command::new("curl");
    // Ignore ~/.curlrc: it must not silently alter the request or write model data to disk.
    curl.args([
        "--disable",
        "--silent",
        "--show-error",
        "--no-buffer",
        "--fail-with-body",
        "--connect-timeout",
        "15",
        "--max-time",
        "600",
        "--header",
        "Content-Type: application/json",
        "--header",
        "Accept: text/event-stream",
        "--data-binary",
        "@-",
        "--url",
        &url,
    ]);
    if let Ok(key) = env::var("MU_API_KEY").or_else(|_| env::var("OPENAI_API_KEY")) {
        if key.contains(['\r', '\n']) {
            return Err("API key contains a newline".into());
        }
        curl.args(["--header", &format!("Authorization: Bearer {key}")]);
    }
    let exit = process::run(
        &mut curl,
        Some(serde_json::to_vec(&request.body())?),
        Duration::from_secs(610),
        &cancel,
        |stderr, bytes| {
            if stderr {
                errors.extend_from_slice(&bytes[..bytes.len().min(8192 - errors.len())]);
                return Ok(());
            }
            preview.extend_from_slice(&bytes[..bytes.len().min(2048 - preview.len())]);
            for event in sse.feed(bytes)? {
                let index = event["output_index"].as_u64().unwrap_or(0) as usize;
                match event["type"].as_str().unwrap_or("") {
                    "response.output_item.added" | "response.output_item.done" => {
                        let item = &event["item"];
                        if let Some((kind, text)) = display(item) {
                            if let Some(&slot) = slots.get(&index) {
                                // Some proxies omit summaries in item.done; don't erase received thoughts.
                                if !text.is_empty() {
                                    live.set(slot, kind, text);
                                }
                            } else {
                                slots.insert(index, live.push(kind, text));
                            }
                        }
                        items.insert(index, item.clone());
                    }
                    "response.output_text.delta"
                    | "response.refusal.delta"
                    | "response.reasoning_summary_text.delta"
                    | "response.reasoning_text.delta"
                    | "response.function_call_arguments.delta" => {
                        let typ = event["type"].as_str().unwrap();
                        let kind = if typ.contains("reasoning") {
                            Kind::Thought
                        } else if typ.contains("arguments") {
                            Kind::Call
                        } else {
                            Kind::Agent
                        };
                        let slot = *slots.entry(index).or_insert_with(|| live.push(kind, ""));
                        live.delta(slot, event["delta"].as_str().unwrap_or(""));
                    }
                    "response.completed" => {
                        completed = Some(event["response"].clone());
                    }
                    "response.failed" | "response.incomplete" | "error" => {
                        return Err(format!("Responses API: {}", event).into());
                    }
                    _ => (),
                }
            }
            Ok(())
        },
    )?;
    if exit.cancelled {
        return Err("Cancelled (partial response not saved)".into());
    }
    if exit.timed_out || exit.code != 0 {
        return Err(format!(
            "HTTP request failed ({}): {} {}",
            exit.code,
            String::from_utf8_lossy(&errors),
            String::from_utf8_lossy(&preview)
        )
        .into());
    }
    let response = completed
        .ok_or_else(|| format!("Stream ended without response.completed: {}", String::from_utf8_lossy(&preview)))?;
    let usage = Usage::from_json(&response["usage"]);
    let mut output: Vec<Value> =
        response["output"].as_array().cloned().unwrap_or_else(|| items.into_values().collect());
    // Final items are authoritative; preserve ids and opaque reasoning for replay.
    for (index, item) in output.iter().enumerate() {
        if let Some((kind, text)) = display(item) {
            if let Some(&slot) = slots.get(&index) {
                if !text.is_empty() {
                    live.set(slot, kind, text);
                }
            } else {
                live.push(kind, text);
            }
        }
    }
    let calls: Vec<_> = output.iter().filter(|v| v["type"] == "function_call").cloned().collect();
    let mut images = vec![];
    for call in &calls {
        let slot = live.push(Kind::Output, "");
        let result = if call["name"] != "bash" {
            Err("Unknown tool (only bash is available)".into())
        } else {
            tools::bash(call["arguments"].as_str().unwrap_or(""), &request.cwd, &cancel, |s| live.delta(slot, s))
        };
        let text = match result {
            Ok((text, attached)) => {
                images.extend(attached);
                text
            }
            Err(e) => format!("bash error: {e}"),
        };
        live.set(slot, Kind::Output, text.clone());
        output.push(json!({"type":"function_call_output", "call_id":call["call_id"], "output":text}));
    }
    output.extend(images);
    Ok(Step { items: output, blocks: live.blocks, usage, again: !calls.is_empty() && !cancel.load(Ordering::Relaxed) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_arbitrary_boundaries_and_crlf() {
        let mut sse = Sse::default();
        let mut events = vec![];
        for b in
            ": keepalive\r\nevent: x\r\ndata: {\"type\":\"µ\",\r\ndata: \"n\":1}\r\n\r\ndata: [DONE]\n\n".as_bytes()
        {
            events.extend(sse.feed(&[*b]).unwrap());
        }
        assert_eq!(events, vec![json!({"type":"µ", "n":1})]);
    }
    #[test]
    fn exact_minimal_tool_and_prompt() {
        let r = Request {
            instructions: "You are coding agent".into(),
            model: "m".into(),
            effort: None,
            input: vec![],
            cwd: PathBuf::new(),
        };
        let b = r.body();
        assert_eq!(b["instructions"], "You are coding agent");
        assert_eq!(b["tools"].as_array().unwrap().len(), 1);
        assert_eq!(b["tools"][0]["description"], "Special bash commands: view_image");
        assert!(b.get("reasoning").is_none());
    }
}
