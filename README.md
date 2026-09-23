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
| `MU_MODEL` | `gpt-5` |
| `MU_EFFORT` | *-* |
| `MU_CONTEXT` | `128000` |

#### Data Store

All persisted data (e.g., sessions) are stored under `$XDG_STATE_HOME/mu`. Falls back to `$HOME/.local/state/mu` otherwise.

### FAQ

- Q: **It depends on `curl` and `ripgrep`, fuck you mean *無*?**
  - A: Agents *love* using `curl` and `rg` on its own, so we've asked ourselves: "why not use it in the harness too?", and the answer was that.
- Q: **How am I supposed to use my Codex/Claude/Gemini subs?**
  - A: To keep code focused on the *harness*, we only implement a subset of the OpenAI Responses API. We recommend using [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) and pointing `mu` to your CPA instance for such use cases.
- Q: **How am I supposed to use two providers without switching the env consistently?**
  - A: CPA can become a nearly transparent OpenAI Compatible API provider. Use that.

### $\color{Gray}{\textsf{---}}$

### License

`mu` is licensed under the [MIT License](LICENSE).

</samp>
