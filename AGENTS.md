Don't add tests unless I request it.
Don't update README unless absolutely necessary.
Assume greenfield and don't write migration code unless stated otherwise.
Never write comments unless it can't be described by the code alone. Even then, you should make the code self-explnatory instead of relying on comments where possible.
Use nix develop .#android instead of gradlew.
Use nix fmt; don't run treefmt directly. Leaving unrelated formatting changes is fine.
Prefer a code that's *simply elegant*
