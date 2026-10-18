# Installation and operations

## Requirements

- A server, NAS or computer that stays on (Havre is one small Rust process and a SQLite file).
- **Docker** with Compose — or, to build from source, **Rust** (stable), **Node.js 22+** and **pnpm**.
- Optional: [Ollama](https://ollama.com) for local AI, a reverse proxy (Caddy, Traefik…) for HTTPS.

## With Docker (recommended)

```sh
git clone https://github.com/maelbel/havre.git
cd havre
cp .env.example .env          # Windows PowerShell: Copy-Item .env.example .env
docker compose up -d --build
```

Open <http://localhost:9988>, choose your Havre password, and add your first account in **Settings › Mail
accounts**. Data lives in the `havre-data` volume; `TZ` in `docker-compose.yml` sets the time zone used for backups and the
morning brief.

To update: `git pull && docker compose up -d --build`.

## From source

```sh
cp .env.example .env
cd web && pnpm install && pnpm build && cd ..
cargo run --release -p havre-server      # http://127.0.0.1:8080
```

Want to look around first? Set `HAVRE_DEMO=1` in `.env` to get sample emails (no accounts in this mode).

## Configuration

Everything is set in `.env`; [`.env.example`](../.env.example) documents every variable. The main ones:

| Variable | Purpose |
|----------|---------|
| `HAVRE_BIND` | Address and port to listen on (default `127.0.0.1:8080`) |
| `HAVRE_DATA_DIR` | Database, encryption key and backups (default `data`) |
| `HAVRE_PUBLIC_URL` | Public address when behind a reverse proxy, e.g. `https://mail.example.com` |
| `HAVRE_MICROSOFT_CLIENT_ID`, `HAVRE_MICROSOFT_TENANT` | Microsoft 365 / Outlook sign-in — see [providers.md](providers.md) |
| `HAVRE_GOOGLE_CLIENT_ID`, `HAVRE_GOOGLE_CLIENT_SECRET` | Google sign-in — see [providers.md](providers.md) |
| `HAVRE_CACHE_MB` | Size of the cache of opened emails (default 512) |
| `HAVRE_RESET_PASSWORD` | `1` to choose a new Havre password at the next start |

## HTTPS

If Havre is reachable from outside your home network, put it behind HTTPS (a reverse proxy such as Caddy or
Traefik) and set `HAVRE_PUBLIC_URL` to the `https://` address: session cookies are then marked `Secure`, and the
OAuth2 redirect addresses are right.

## Backups and restore

Settings › Backups. Every night (03:00 by default) Havre copies its database
into a backups folder and keeps the last 7. The Apps page's status card shows
when the last one ran, and turns orange when one failed.

**What's in a backup:** accounts, labels, stars and snoozes, drafts, the Havre
calendar, AI and app settings. **What isn't:** your mail itself (it stays on
your mail servers; Havre downloads it again after a restore), the search
index and AI summaries (rebuilt), and sign-in sessions (you sign in again).

**Where:** `HAVRE_BACKUP_DIR`, by default `backups/` next to the database
(`/srv/havre/data/backups` in Docker). That's the same disk as Havre: copy the
folder somewhere else too, or point it at a NAS or another disk (with Docker,
mount that folder at `/srv/havre/data/backups`; it must be writable by the
container's `havre` user). Each backup can also be downloaded from the page.

**The time** is the server's local time. With Docker, set `TZ` (compose.yml
defaults to `Europe/Paris`).

**The recovery key.** Mail passwords, refresh tokens and API keys are
encrypted with a key kept in `secret.key`, next to the database — and *not*
in backups. A stolen backup alone gives nothing away; but restoring on a new
server needs that key. Print it once and keep it in your password manager:

```sh
docker compose exec havre havre key      # or: havre key
```

The page shows the key's fingerprint (the same one is written in each
backup), so you can tell which key a backup needs. Havre never sends the key
itself to the browser.

**Restore** (Havre stopped; the current database is kept as
`havre.db.before-restore-<time>`):

```sh
docker compose stop havre
docker compose run --rm havre havre restore /srv/havre/data/backups/havre-2026-10-15-030000.db
docker compose start havre

# Without Docker, from the data folder's machine:
havre restore backups/havre-2026-10-15-030000.db
# On a new server, add the recovery key:
havre restore <file> --key <64 hex characters>
# Key lost? Restore anyway and type the mail passwords again:
havre restore <file> --key-mismatch-ok
```

**Exports:** each calendar as an `.ics` file (Settings › Backups › Export),
which any calendar app can import.
