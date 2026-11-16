// How the model's output items read in a transcript.
use mu_wire::{Block, Kind, Tool, ToolOutput, ToolStatus};
use serde_json::Value;

pub fn block_from_item(item: &Value, partial: bool) -> Option<Block> {
    if item["role"] == "user"
        && let Some(text) = item["content"].as_str()
    {
        return Some(Block::new(Kind::User, text));
    }
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
        "reasoning" => {
            let mut text = texts("summary", "summary_text");
            if text.is_empty() {
                text = texts("content", "reasoning_text");
            }
            Some(Block::new(Kind::Thought, text))
        }
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

// A partial JSON string is display-only: execution still uses the final, validated args.
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
