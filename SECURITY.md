# Security policy

Havre holds sensitive data — your mail, calendars, contacts, passwords for your mail accounts and API keys — so
security reports are taken seriously and handled before anything else.

## Supported versions

Havre is in early development (0.x). Security fixes are made on the latest release and on `develop`; older versions
are not patched. Please update to the latest release before reporting.

| Version | Supported |
|---------|-----------|
| Latest release | ✅ |
| `develop` | ✅ |
| Older releases | ❌ |

## Reporting a vulnerability

**Please do not open a public issue, discussion or pull request for a security problem.**

Report it privately through GitHub: on the repository, go to **Security › Advisories › Report a vulnerability**
(GitHub's private vulnerability reporting). Include:

- what an attacker could do, and what they need first (an account on the Havre server, access to the network, an
  email sent to the victim…);
- the steps to reproduce, with the version or commit;
- logs or a proof of concept if you have one — with no real personal data in it.

What happens next:

- an acknowledgement within **7 days**;
- an assessment and a plan (or the reasons it isn't treated as a vulnerability) within **30 days**;
- a fix released as soon as possible, then a published advisory crediting you, unless you prefer to stay anonymous.

Please give a reasonable time to fix the problem before disclosing it publicly.

## Scope

In scope: the Havre server (`crates/`), the web UI (`web/`), Havre Desktop (`desktop/`), the Docker image and the
default configuration.

Examples of what matters most:

- reading or changing someone's mail, events, tasks, notes or files without being signed in;
- leaking stored secrets (mail passwords, OAuth refresh tokens, AI API keys, the recovery key) or session tokens;
- script injection through an email, an invitation, a note or a file name;
- the AI assistant acting (sending, deleting, creating) without the user's confirmation;
- the MCP endpoint answering without a valid token.

Out of scope: problems that need full control of the server or of the user's device, missing hardening that has no
practical impact, and vulnerabilities in third-party services (Microsoft, Google, mail servers) — report those to
their owners.

## Good practice for self-hosters

- Run Havre behind HTTPS (a reverse proxy such as Traefik or Caddy) and set `HAVRE_PUBLIC_URL` to the `https://`
  address so session cookies are marked `Secure`.
- Keep `.env` and the data folder readable by Havre only; back up the recovery key (`havre key`) somewhere safe.
- Update to new releases; watch the repository (*Releases* and *Security advisories*) to be told.
