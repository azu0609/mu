<samp>

## mu

*μ* sized agent harness for the rest of us, with *無* base tokens, RAM and binary footprint.
<br/>

### $\color{Gray}{\textsf{---}}$

### Usage

#### With Nix

```bash
nix run
```

#### Environment Variables

| Key | Default |
| :--- | :--- |
| `MU_BASE_URL` | `http://127.0.0.1:8317/v1` |
| `MU_API_KEY` | *-* |
| `MU_MODEL` | `gpt-5` |
| `MU_EFFORT` | `high` |
| `MU_CONTEXT` | `128000` |

#### Data Store

All persisted data (e.g., sessions) are stored under `$XDG_STATE_HOME/mu`. Falls back to `$HOME/.local/state/mu` otherwise.

### $\color{Gray}{\textsf{---}}$

### License

`mu` is licensed under the [MIT License](LICENSE).

</samp>
