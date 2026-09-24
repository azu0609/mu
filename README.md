<samp>

## mu

*μ* sized agent harness for the rest of us, with near-*無* base tokens, RAM and binary footprint.

### $\color{Gray}{\textsf{---}}$

### Usage

#### With Nix

```bash
nix run # Starts the TUI
```

#### Environment Variables

| Key | Default |
| :--- | :--- |
| `MU_BASE_URL` | `http://127.0.0.1:8317/v1` |
| `MU_API_KEY` | *-* |
| `MU_MODEL` | `gpt-6-luna` |
| `MU_EFFORT` | `xhigh` |
| `MU_CONTEXT` | `272000` |

#### Data Store

All persisted data (e.g., sessions) are stored under `$XDG_STATE_HOME/mu`. Falls back to `$HOME/.local/state/mu` otherwise.

### FAQ

- Q: **It depends on `curl` and `ripgrep`, fuck you mean *μ* sized?**
  - A: Agents *love* using `curl` and `rg` on its own, so we've asked ourselves: "why not use it in the harness too?", and the answer was that.
- Q: **What does _"near-無 base tokens"_ mean?**
  - A: We aim for ~64 input tokens.
  - **Note:** Some providers (e.g., DeepSeek) may include an additional set of server-side templates and count it toward the reported `input_tokens` value. Feel free to try the command below to test the token floor yourself.
  ```bash
  curl -X POST 'https://api.deepseek.com/responses' \
    -H 'Content-Type: application/json' \
    -H 'Authorization: Bearer <TOKEN>' \
    --data-raw '{
      "model": "deepseek-flash",
      "input": "Hello",
      "reasoning": {
        "effort": "max"
      },
      "stream": false,
      "tools": [
        {
          "type": "function",
          "name": "tool"
        }
      ]
    }' | jq '.usage.input_tokens' # 257 as of writing
  ```
- Q: **How am I supposed to use my Codex/Claude/Gemini subs?**
  - A: To keep code focused on the *harness*, we only implement a small subset of the **stateless** OpenAI Responses API. We recommend using [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) and pointing `mu` to your CPA instance for such use cases.
- Q: **How do I use two providers without switching the env and re-launching consistently?**
  - A: CPA can become a nearly transparent OpenAI Compatible API gateway (`openai-compatibility` for Chat Completions, `codex-api-key` for Responses API). Use that.

### $\color{Gray}{\textsf{---}}$

### License

`mu` is licensed under the [MIT License](LICENSE).

</samp>
