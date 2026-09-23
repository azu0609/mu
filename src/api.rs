use crate::{
    Result, process,
    session::{Block, Kind, Tool, ToolOutput, ToolStatus, Usage},
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
    thread,
    time::Duration,
};

pub enum Event {
    Push(Block),
    Delta(usize, String),
    Set(usize, Block),
    ToolOutput(usize, ToolOutput),
    Done(Result<Step>),
}
pub struct Step {
    pub items: Vec<Value>,
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
            "parallel_tool_calls": true,
            "tools": [{"type":"function", "name":"bash", "description":"Special bash commands: view_image",
                "parameters":{"type":"object", "properties":{"command":{"type":"string"}, "timeoutMs":{"type":"integer"}}, "required":["command"], "additionalProperties":false}, "strict":false}]
        });
        if let Some(effort) = &self.effort {
            body["reasoning"] = json!({"effort":effort, "summary":"auto"});
        }
        body
    }
}

// App owns the rendered blocks; API output indices map to stable UI slots.
struct Live<'a> {
    tx: &'a Sender<Event>,
    slots: BTreeMap<usize, usize>,
}
impl Live<'_> {
    fn upsert(&mut self, index: usize, block: Block) {
        if let Some(&slot) = self.slots.get(&index) {
            let _ = self.tx.send(Event::Set(slot, block));
        } else {
            let slot = self.slots.len();
            self.slots.insert(index, slot);
            let _ = self.tx.send(Event::Push(block));
        }
    }
    fn delta(&mut self, index: usize, kind: Kind, text: &str) {
        if !self.slots.contains_key(&index) {
            self.upsert(index, Block::new(kind, ""));
        }
        let _ = self.tx.send(Event::Delta(self.slots[&index], text.into()));
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

// Read only a top-level command, skipping complete fields before it. A partial
// JSON string is display-only: execution still uses the final, validated args.
fn command_prefix(raw: &str) -> Option<String> {
    let mut rest = raw.trim_start().strip_prefix('{')?;
    loop {
        let mut key = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let name = key.next()?.ok()?;
        rest = rest[key.byte_offset()..].trim_start().strip_prefix(':')?.trim_start();
        if name == "command" {
            return string_prefix(rest);
        }
        let mut value = serde_json::Deserializer::from_str(rest).into_iter::<serde::de::IgnoredAny>();
        value.next()?.ok()?;
        rest = rest[value.byte_offset()..].trim_start().strip_prefix(',')?;
    }
}

fn string_prefix(raw: &str) -> Option<String> {
    match serde_json::Deserializer::from_str(raw).into_iter::<String>().next()? {
        Ok(text) => return Some(text),
        Err(e) if e.is_eof() => (),
        Err(_) => return None,
    }
    // Supply the missing quote and let serde decode. On an unfinished escape,
    // withhold it from the preview (and its high surrogate, if paired).
    let mut prefix = raw.to_owned();
    loop {
        prefix.push('"');
        if let Ok(text) = serde_json::from_str(&prefix) {
            return Some(text);
        }
        prefix.truncate(prefix.rfind('\\')?);
    }
}

fn display(item: &Value, partial: bool) -> Option<Block> {
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
            Some(Block::new(Kind::Agent, text))
        }
        "reasoning" => Some(Block::new(Kind::Thought, texts("summary", "summary_text"))),
        "function_call" => {
            let name = item["name"].as_str().unwrap_or("tool");
            let raw = item["arguments"].as_str().unwrap_or("");
            let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
            let text = if name == "bash" {
                args["command"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| partial.then(|| command_prefix(raw)).flatten())
                    .unwrap_or_else(|| if partial || raw.is_empty() { "bash …" } else { raw }.to_owned())
            } else {
                format!("{name} {raw}")
            };
            let mut block = Block::new(Kind::Call, text);
            block.tool = Some(Tool {
                call_id: item["call_id"].as_str().unwrap_or("").into(),
                status: ToolStatus::Pending,
                output: ToolOutput::default(),
            });
            Some(block)
        }
        _ => None,
    }
}

pub fn step(request: Request, cancel: Arc<AtomicBool>, tx: &Sender<Event>) -> Result<Step> {
    let mut live = Live { tx, slots: BTreeMap::new() };
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
                        if let Some(block) = display(item, event["type"] == "response.output_item.added") {
                            live.upsert(index, block);
                        }
                        items.insert(index, item.clone());
                    }
                    "response.output_text.delta"
                    | "response.refusal.delta"
                    | "response.reasoning_summary_text.delta"
                    | "response.reasoning_text.delta" => {
                        let typ = event["type"].as_str().unwrap();
                        let kind = if typ.contains("reasoning") { Kind::Thought } else { Kind::Agent };
                        live.delta(index, kind, event["delta"].as_str().unwrap_or(""));
                    }
                    "response.function_call_arguments.delta" => {
                        let item = items.entry(index).or_insert_with(|| json!({"type":"function_call"}));
                        let mut raw = item["arguments"].as_str().unwrap_or("").to_owned();
                        raw.push_str(event["delta"].as_str().unwrap_or(""));
                        item["arguments"] = raw.into();
                        if let Some(block) = display(item, true) {
                            live.upsert(index, block);
                        }
                    }
                    "response.function_call_arguments.done" => {
                        let item = items.entry(index).or_insert_with(|| json!({"type":"function_call"}));
                        item["arguments"] = event["arguments"].clone();
                        if let Some(block) = display(item, false) {
                            live.upsert(index, block);
                        }
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
        if let Some(block) = display(item, false) {
            live.upsert(index, block);
        }
    }
    let results = thread::scope(|scope| {
        // Every task owns its call block and streams to that stable slot. A slow
        // first command cannot delay another command's execution or UI updates.
        let handles: Vec<_> = output
            .iter()
            .enumerate()
            .filter(|(_, call)| call["type"] == "function_call")
            .map(|(index, call)| {
                let slot = live.slots[&index];
                let mut block = display(call, false).unwrap();
                let cwd = &request.cwd;
                let cancel = &cancel;
                scope.spawn(move || {
                    block.tool.as_mut().unwrap().status = ToolStatus::Running;
                    let _ = tx.send(Event::Set(slot, block.clone()));
                    let tool = block.tool.as_mut().unwrap();
                    let result = if call["name"] != "bash" {
                        Err("Unknown tool (only bash is available)".into())
                    } else {
                        tools::bash(call["arguments"].as_str().unwrap_or(""), cwd, cancel, |output| {
                            tool.output = output.clone();
                            let _ = tx.send(Event::ToolOutput(slot, output));
                        })
                    };
                    let mut images = vec![];
                    match result {
                        Ok(result) => {
                            tool.output = result.output;
                            tool.status = result.status;
                            images = result.images;
                        }
                        Err(e) => {
                            tool.output.text.push_str(&format!("\n[tool error: {e}]"));
                            tool.status = ToolStatus::Error;
                        }
                    }
                    let mut text = tool.output.text.clone();
                    if let Some(summary) = tool.status.summary() {
                        text.push_str(&format!("\n[{summary}]"));
                    }
                    if tool.output.truncated
                        && let Some(log) = &tool.output.log
                    {
                        text.push_str(&format!(
                            "\n[Output log (temporary): {} — read needed ranges with sed, tail, or grep]",
                            log.display()
                        ));
                    }
                    let item = json!({"type":"function_call_output", "call_id":tool.call_id, "output":text});
                    let _ = tx.send(Event::Set(slot, block));
                    (item, images)
                })
            })
            .collect();
        // Keep replay/results in model call order, regardless of completion order.
        handles.into_iter().map(|h| h.join().map_err(|_| "Tool worker panicked".into())).collect::<Result<Vec<_>>>()
    })?;
    let again = !results.is_empty() && !cancel.load(Ordering::Relaxed);
    let mut images = vec![];
    for (item, attached) in results {
        output.push(item);
        images.extend(attached);
    }
    output.extend(images);
    Ok(Step { items: output, usage, again })
}
