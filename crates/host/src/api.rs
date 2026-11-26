use crate::{
    Result,
    bridge::Bridge,
    config,
    items::block_from_item,
    process,
    session::{Entry, Usage},
    tools,
};
use mu_wire::{Block, Kind, LiveEvent, ModelInfo, ToolStatus};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub const CACHE_MISS: &str = "Cache miss · previously cached prefix was not read";

pub type Emit<'a> = &'a (dyn Fn(LiveEvent) + Sync);

pub struct Step {
    pub entries: Vec<Entry>,
    pub usage: Usage,
    pub again: bool,
}

pub struct Request {
    pub instructions: String,
    pub model: String,
    pub effort: Option<String>,
    pub input: Vec<Value>,
    pub cwd: PathBuf,
    pub bridge: Arc<Bridge>,
    pub session: String,
    pub previous_usage: Usage,
}

impl Request {
    pub fn body(&self) -> Value {
        let description = format!("Special bash commands: {}", self.bridge.commands().join(", "));
        let mut body = json!({
            "model": self.model, "instructions": self.instructions, "input": self.input,
            "stream": true, "store": false, "include": ["reasoning.encrypted_content"],
            "reasoning": {"summary": "auto"},
            "parallel_tool_calls": true,
            "tools": [{"type":"function", "name":"bash", "description":description,
                "parameters":{"type":"object", "properties":{"command":{"type":"string"}, "timeoutMs":{"type":"integer"}}, "required":["command"], "additionalProperties":false}, "strict":false}]
        });
        if let Some(effort) = &self.effort {
            body["reasoning"]["effort"] = effort.clone().into();
        }
        body
    }
}

struct Live<'a> {
    emit: Emit<'a>,
    slots: BTreeMap<usize, usize>,
}

impl Live<'_> {
    fn upsert(&mut self, index: usize, block: Block) {
        if let Some(&slot) = self.slots.get(&index) {
            (self.emit)(LiveEvent::Set { index: slot, block });
        } else {
            let slot = self.slots.len();
            self.slots.insert(index, slot);
            (self.emit)(LiveEvent::Push(block));
        }
    }

    fn delta(&mut self, index: usize, kind: Kind, text: &str) {
        if !self.slots.contains_key(&index) {
            self.upsert(index, Block::new(kind, ""));
        }
        (self.emit)(LiveEvent::Delta { index: self.slots[&index], text: text.into() });
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

fn curl(path: &str, max_time: u64) -> Result<Command> {
    let base = config::var("MU_BASE_URL")?
        .or(config::var("OPENAI_BASE_URL")?)
        .unwrap_or_else(|| "http://127.0.0.1:8317/v1".into());
    let mut curl = Command::new("curl");
    // Ignore ~/.curlrc: it must not silently alter the request or write model data to disk.
    curl.args([
        "--disable",
        "--silent",
        "--show-error",
        "--fail-with-body",
        "--connect-timeout",
        "15",
        "--max-time",
        &max_time.to_string(),
        "--url",
        &format!("{}{path}", base.trim_end_matches('/')),
    ]);
    if let Some(key) = config::var("MU_API_KEY")?.or(config::var("OPENAI_API_KEY")?) {
        if key.contains(['\r', '\n']) {
            return Err("API key contains a newline".into());
        }
        curl.env("MU_API_KEY", key).args([
            "--variable",
            "%MU_API_KEY",
            "--expand-header",
            "Authorization: Bearer {{MU_API_KEY}}",
        ]);
    }
    Ok(curl)
}

const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

// Asks for Anthropic's shape of the list even though mu speaks Responses: it is the only spec
// that says how much context a model takes.
pub fn models() -> Result<Vec<ModelInfo>> {
    let mut curl = curl("/models?limit=1000", 30)?;
    curl.args(["--header", "anthropic-version: 2026-10-21"]);
    let (mut body, mut errors) = (vec![], vec![]);
    let exit = process::run(&mut curl, None, Duration::from_secs(35), &Arc::default(), |stderr, bytes| {
        let buffer = if stderr { &mut errors } else { &mut body };
        if buffer.len() + bytes.len() > 16 * 1024 * 1024 {
            return Err("The model list exceeds 16 MiB".into());
        }
        buffer.extend_from_slice(bytes);
        Ok(())
    })?;
    if exit.timed_out || exit.code != 0 {
        return Err(format!(
            "Listing models failed ({}): {} {}",
            exit.code,
            String::from_utf8_lossy(&errors),
            String::from_utf8_lossy(&body[..body.len().min(2048)])
        )
        .into());
    }
    let list: Value = serde_json::from_slice(&body)?;
    let models = list["data"].as_array().ok_or("The endpoint's model list has no data")?;
    Ok(models
        .iter()
        .filter_map(|model| {
            Some(ModelInfo {
                name: model["id"].as_str()?.to_owned(),
                context: model["max_input_tokens"].as_u64().filter(|&n| n > 0),
                efforts: EFFORTS.map(Into::into).into(),
            })
        })
        .collect())
}

pub fn step(request: Request, cancel: Arc<AtomicBool>, emit: Emit) -> Result<Step> {
    let mut live = Live { emit, slots: BTreeMap::new() };
    let mut items = BTreeMap::new();
    let mut completed = None;
    let mut sse = Sse::default();
    let mut errors = vec![];
    let mut preview = vec![];
    let mut curl = curl("/responses", 600)?;
    curl.args([
        "--no-buffer",
        "--header",
        "Content-Type: application/json",
        "--header",
        "Accept: text/event-stream",
        "--data-binary",
        "@-",
    ]);
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
                        if let Some(block) = block_from_item(item, event["type"] == "response.output_item.added") {
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
                        if let Some(block) = block_from_item(item, true) {
                            live.upsert(index, block);
                        }
                    }
                    "response.function_call_arguments.done" => {
                        let item = items.entry(index).or_insert_with(|| json!({"type":"function_call"}));
                        item["arguments"] = event["arguments"].clone();
                        if let Some(block) = block_from_item(item, false) {
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
    let output: Vec<Value> = response["output"].as_array().cloned().unwrap_or_else(|| items.into_values().collect());
    for (index, item) in output.iter().enumerate() {
        if let Some(block) = block_from_item(item, false) {
            live.upsert(index, block);
        }
    }
    if usage.cache_miss(request.previous_usage) {
        emit(LiveEvent::Notice(Block::new(Kind::Warning, CACHE_MISS)));
    }
    let results = thread::scope(|scope| {
        let handles: Vec<_> = output
            .iter()
            .enumerate()
            .filter(|(_, call)| call["type"] == "function_call")
            .map(|(index, call)| {
                let slot = live.slots[&index];
                let mut block = block_from_item(call, false).unwrap();
                let cwd = &request.cwd;
                let cancel = &cancel;
                let run = request.bridge.open(Some(request.session.clone()));
                scope.spawn(move || {
                    block.tool.as_mut().unwrap().status = ToolStatus::Running;
                    emit(LiveEvent::Set { index: slot, block: block.clone() });
                    let tool = block.tool.as_mut().unwrap();
                    let result = if call["name"] != "bash" {
                        Err("Unknown tool (only bash is available)".into())
                    } else {
                        tools::bash(call["arguments"].as_str().unwrap_or(""), cwd, &run, cancel, |output| {
                            tool.output = output.clone();
                            emit(LiveEvent::Output { index: slot, output });
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
                    let entry = Entry::Tool(tool.clone());
                    emit(LiveEvent::Set { index: slot, block });
                    (entry, images)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().map_err(|_| "Tool worker panicked".into())).collect::<Result<Vec<_>>>()
    })?;
    let again = !results.is_empty() && !cancel.load(Ordering::Relaxed);
    let mut entries: Vec<_> = output.into_iter().map(|item| Entry::Item { item }).collect();
    let mut images = vec![];
    for (entry, attached) in results {
        entries.push(entry);
        images.extend(attached);
    }
    entries.extend(images.into_iter().map(|item| Entry::Item { item }));
    Ok(Step { entries, usage, again })
}
