# Installation and operations

## Requirements

- A server, NAS or computer that stays on (Havre is one small Rust process and a SQLite file).
- **Docker** with Compose — or a release download (Linux) — or, to build from source, **Rust** (stable, 1.98 or newer),
  **Node.js 22+** and **pnpm**.
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

### With the ready-made image

Each release also publishes an image, for PCs and servers (amd64) and the Raspberry Pi 4/5 (arm64):
`ghcr.io/maelbel/havre:<version>` (or `:latest`). No build on your machine: in `docker-compose.yml`, replace the
`build:` lines with `image: ghcr.io/maelbel/havre:0.5.0`, then `docker compose up -d`. To update, change the
version and run `docker compose pull && docker compose up -d`.

To check that an image was built by Havre's release workflow:
`gh attestation verify oci://ghcr.io/maelbel/havre:0.5.0 --repo maelbel/havre`.

## From a release (Linux)

On the [Releases](https://github.com/maelbel/havre/releases) page, download `havre-<version>-linux-x86_64.tar.gz`
(a PC or server) or `…-linux-arm64.tar.gz` (Raspberry Pi 4/5 with a 64-bit OS), and `SHA256SUMS`. They run on
Debian 12 / Raspberry Pi OS (bookworm), Ubuntu 22.04 and newer.

```sh
sha256sum -c SHA256SUMS --ignore-missing     # the file is the one published
tar xzf havre-0.5.0-linux-arm64.tar.gz && cd havre-0.5.0-linux-arm64
cp .env.example .env
./havre                                      # http://127.0.0.1:8080
```

The web UI is in `web/.output/public` next to `havre`, where `HAVRE_WEB_DIR` looks by default. `gh attestation verify
<file> --repo maelbel/havre` also proves a download was built by the release workflow from Havre's code.

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

**Phones and notifications** need HTTPS too: browsers only install web apps and allow push notifications on an
`https://` address (or `localhost`). Two more things:
- The server sends notifications to the phones' push services, so it needs to reach them over HTTPS (outgoing
  port 443): `fcm.googleapis.com` (Chrome, Android), `*.push.apple.com` (Safari), `updates.push.services.mozilla.com`
  (Firefox).
- Behind a sign-in proxy (Authentik, Authelia…), `/manifest.webmanifest`, `/sw.js` and the icons are fetched with
  your session cookie, like the rest of the page: nothing to open up. If you exempt paths from the proxy, leave
  these protected or public; both work.

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

**Off-site copy** (Settings › Backups › Off-site copy): after each backup,
Havre encrypts it with the recovery key and sends it to a **WebDAV folder**
(Nextcloud, a NAS) or an **S3 bucket** (Backblaze B2, MinIO, Scaleway, AWS),
keeping the last 14 there (older copies Havre made are deleted; other files
are never touched). A failed copy is tried again every 15 minutes, then every
hour; the status card turns orange after a day of failures.

- Nextcloud: the folder's address is
  `https://<your cloud>/remote.php/dav/files/<user>/Havre` and the password an
  *app password* (Settings › Security › Devices & sessions).
- S3: the endpoint (e.g. `https://s3.eu-central-003.backblazeb2.com`), the
  bucket, its region, and a key allowed only on that bucket. MinIO and B2 use
  *path-style* addresses (the default).
- Addresses must be `https://` (plain `http://` only to the same machine).
  The password or secret key is stored encrypted and never sent back to the
  browser.

The copies are `havre-<date>.havre` files: unreadable without the recovery
key — keep it somewhere other than the server.

**The time** is the server's local time. With Docker, set `TZ` (docker-compose.yml
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

**Disaster recovery** (the server is gone): install Havre on the new machine,
download the newest `.havre` file from your Nextcloud or bucket the usual
way, then restore it with the recovery key — no off-site password needed:

```sh
havre restore havre-2026-11-08-030000.havre --key <64 hex characters>
```

Havre checks the key's fingerprint, decrypts the copy (a changed or
incomplete copy is refused), checks the database and puts it in place.

**Exports:** each calendar as an `.ics` file (Settings › Backups › Export),
which any calendar app can import.
