# Connecting Microsoft 365 and Google

Most mail providers only need your email address and password (Havre fills in the servers for Gmail, Outlook,
iCloud, Yahoo, Orange, Free…). Microsoft 365 / Outlook.com, and Gmail if you prefer not to use an app password,
sign in with **OAuth2**, which needs a one-time registration of your Havre with Microsoft or Google. This page walks
through it.

## Microsoft 365 and Outlook

Microsoft mailboxes — Outlook.com, Hotmail, and company addresses on
Microsoft 365 such as `you@company.com` — **no longer accept passwords** over
IMAP. Like Thunderbird, Havre uses **OAuth2**: a window opens on Microsoft's
sign-in page, you sign in and accept, the window closes, and Havre receives a
token. Only a refresh token is stored (encrypted); revoke it any time from your
Microsoft account (*My Apps* / *Security*).

If the window can't be used (pop-ups blocked, or Havre opened at an address
Microsoft won't accept, like `http://192.168.1.20`), choose **Use a code
instead**: type a short code at microsoft.com/devicelogin — it works from
anywhere.

In **Settings › Add account**, set **Authentication** to **OAuth2** (like
in Thunderbird). Havre picks it for you when it recognises a Microsoft address,
company domains included, and fills in the server (`outlook.office365.com`,
port 993) — still editable under *Server settings*. Click *Connect*.

Why does Thunderbird work without the setup below? Mozilla registered
Thunderbird with Microsoft once, for every user. Havre is self-hosted, so each
installation registers itself — it takes five minutes.

It needs a one-time **app registration** in Microsoft Entra — for a company
mailbox, ask your IT admin if you don't have the rights:

1. Open <https://entra.microsoft.com> → *Identity › Applications › App
   registrations* → **New registration**.
   - Name: `Havre`.
   - Supported account types: *Accounts in this organizational directory
     only* (just your company), or *any organizational directory and personal
     Microsoft accounts* (also Outlook.com).
   - Redirect URI: leave empty (next step).
2. **Authentication** → **Add a platform** → **Mobile and desktop
   applications** → *Custom redirect URIs*: the address you open Havre at,
   followed by `/api/oauth/microsoft/callback`. With Docker on your machine:

   ```text
   http://localhost:9988/api/oauth/microsoft/callback
   ```

   (Havre shows the exact address in the sign-in panel. `localhost` and
   `127.0.0.1` count as different addresses. Microsoft only accepts `http://`
   for `localhost`; anything else needs `https://`. Several addresses can be
   added.) Then, under *Advanced settings*, set **Allow public client flows:
   Yes** (for *Use a code instead*) → Save. There is no client secret: Havre
   uses PKCE, like Thunderbird.
3. **API permissions** → *Add a permission* → *APIs my organization uses* →
   search **Office 365 Exchange Online** → *Delegated permissions* →
   **IMAP.AccessAsUser.All** and **SMTP.Send** (to send mail) → Add. Then
   *Add a permission* → **Microsoft Graph** → *Delegated permissions* →
   **Calendars.ReadWrite** (for the Agenda) and **Tasks.ReadWrite** (for
   Tasks, Microsoft To Do) → Add. If your organisation
   requires it, click **Grant admin consent**.
4. **Overview** → copy the **Application (client) ID** into `.env`:

   ```sh
   HAVRE_MICROSOFT_CLIENT_ID=00000000-0000-0000-0000-000000000000
   # Single-tenant registration (step 1, first choice): your domain or tenant id
   HAVRE_MICROSOFT_TENANT=company.com
   ```

5. Restart Havre (`docker compose up -d` re-reads `.env`).

Behind a reverse proxy, if the redirect URI Havre shows is wrong, set
`HAVRE_PUBLIC_URL` (e.g. `https://mail.example.com`).

IMAP must also be enabled for the mailbox (Microsoft 365 admin center ›
*Users › Active users* › the user › *Mail* › *Manage email apps* › **IMAP**),
and **Authenticated SMTP** on the same page to send mail. IMAP is on by
default; Authenticated SMTP is often off, so ask your admin.

Accounts connected before Havre could send show *Reconnect to allow
sending* in Settings: reconnect once, and Microsoft asks for the new
permission.

When something goes wrong, Havre shows Microsoft's reason in plain words
(admin consent needed, redirect URI missing, wrong tenant…). If the
sign-in expires or is revoked, the account shows **Reconnect** in Settings.

## Gmail and Google Workspace

Gmail works two ways — pick it with **Authentication** when adding the account:

- **OAuth2** (like Thunderbird, the default for Gmail addresses): a window opens
  on Google's sign-in page, you accept, done. Needs the one-time setup below.
- **Normal password** with an *app password* (Google Account › Security ›
  2-Step Verification › App passwords). No setup, but Google is phasing these
  out, and Workspace admins often disable them.

Havre detects Google Workspace company addresses (from the domain's mail
servers) and picks OAuth2 with `imap.gmail.com`.

One-time setup in Google Cloud:

1. Open <https://console.cloud.google.com> → create a project named `Havre`.
   *APIs & Services › Library* → enable the **Gmail API**, the **Google
   Calendar API** (for the Agenda) and the **Google Tasks API** (for Tasks).
2. **Google Auth Platform** (the OAuth consent screen):
   - *Branding*: app name `Havre`, your email.
   - *Audience*: **Internal** if it's for a Google Workspace organisation
     only (simplest), otherwise **External**. With *External*, add yourself
     under *Test users*, then **Publish app** (see the note below).
   - *Data access* → *Add or remove scopes* → add
     `https://mail.google.com/`, `https://www.googleapis.com/auth/calendar`
     and `https://www.googleapis.com/auth/tasks` (manually if they aren't
     listed).
3. **Clients** → *Create client* → *Web application* → *Authorized redirect
   URIs*: the address you open Havre at, followed by
   `/api/oauth/google/callback`. With Docker on your machine:

   ```text
   http://localhost:9988/api/oauth/google/callback
   ```

4. Copy the **Client ID** and **Client secret** into `.env`, then restart
   Havre (`docker compose up -d`):

   ```sh
   HAVRE_GOOGLE_CLIENT_ID=1234567890-abc.apps.googleusercontent.com
   HAVRE_GOOGLE_CLIENT_SECRET=GOCSPX-...
   ```

> **Testing vs published.** While an *External* app is in "Testing", Google
> ends its sign-ins after **7 days** (the account then shows *Reconnect*).
> Publishing it removes that limit. Google hasn't reviewed your Havre, so the
> sign-in page warns *"Google hasn't verified this app"*: click *Advanced ›
> Go to Havre* — it's your own app. (Up to 100 people can use an unverified
> app; that's plenty for a self-hosted Havre.)

Unlike Microsoft, Google has no "type a code" fallback for mail: the window
is the only way, so the redirect URI must match the address you use.
