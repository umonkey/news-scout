---
name: rust
description: Rules for writing and formatting Rust code. Use whenever creating or modifying Rust source code.
---

## Rust Coding Style

- Block operators and multi-line operators should be separated by empty lines.
- Sort methods alphabetically. Public methods should go first, then private.
- For error reporting, use anyhow.

### Block spacing

Statements that span multiple lines, should be surrounded by empty lines from any other statement. Bad:

```
let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
let request = serde_json::json!({
    "chat_id": self.chat_id,
    "text": args.text,
    "parse_mode": "HTML"
});
```

Good:

```
let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);

let request = serde_json::json!({
    "chat_id": self.chat_id,
    "text": args.text,
    "parse_mode": "HTML"
});
```
