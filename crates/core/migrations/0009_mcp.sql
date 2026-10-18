-- MCP: other AI apps (Claude Code, Claude Desktop…) can use Havre's tools.
-- One token per app; only its SHA-256 is stored (like sessions).
CREATE TABLE mcp_tokens (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    token_hash   BLOB NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
