# Cloudflare Tunnel + Access setup for the 3 new hostnames

Hostnames are flat (`infinite-scroll-*.virtualdino.com`): the free wildcard
certificate covers only one subdomain level, so nested names such as
`*.infinite-scroll.art.virtualdino.com` do not work. `infinite-scroll-library`
is the name the `printer` service's CORS configuration expects. The web UI
is same-origin (it is served by `printer`, which proxies uploads to
`uploader`), so a browser needs only that one hostname; `upload` and
`printer` are for machine callers such as the MCP layer.

Three new local ports need Tunnel ingress rules and Access Applications,
matching the existing `infinite-scroll-api.virtualdino.com` / MediaWatch
pattern (human session + service token on one Access Application per
hostname).

This document is a manual runbook, not something this session could execute:
it has no SSH path to `pve2` (where the Tunnel connector runs) and no
Cloudflare API scope beyond `zone:read`. Do not attempt any Cloudflare API
calls for this even if a broader token turns up somewhere — surface that
discovery and ask before using it, per this project's standing "never
enable/change account-level config without an explicit decision" rule.

## 1. Tunnel ingress (on pve2, wherever `cloudflared`'s config.yml lives
   for the tunnel that currently routes to infinite-scroll-api)

Add three ingress rules pointing at the Pi's LAN address (same one the
current `infinite-scroll-api` rule already targets), before the final
catch-all 404 rule:

```yaml
- hostname: infinite-scroll-upload.virtualdino.com
  service: http://<pi-lan-ip>:8081
- hostname: infinite-scroll-printer.virtualdino.com
  service: http://<pi-lan-ip>:8082
- hostname: infinite-scroll-library.virtualdino.com
  service: http://<pi-lan-ip>:8082
```

`printer.*` and `library.*` both point at the same port (8082): the
`printer` service now serves the static library frontend directly (embedded
via `include_str!` at compile time — see `crates/printer/src/main.rs`)
rather than through a separate webserver, so both public hostnames terminate
at the same local `printer` process. This is a standard, supported Cloudflare
Tunnel pattern (multiple public hostnames -> one origin service).

Reload the tunnel connector after editing (`cloudflared` picks up
config.yml changes on restart; check whether this specific connector is
managed by systemd and if so `systemctl restart cloudflared` on pve2, or
via the dashboard if it's a remotely-managed tunnel instead of a local
config file — check which mode this tunnel uses before assuming a local
file edit is even the right mechanism).

## 2. DNS

Each hostname needs a CNAME to the tunnel (`<tunnel-id>.cfargotunnel.com`),
same as the existing `infinite-scroll-api` record. If the tunnel is in
dashboard-managed "Public Hostname" mode, adding the hostname there
creates the DNS record automatically — check this before manually adding
CNAMEs, to avoid a conflicting duplicate record.

## 3. Access Applications (one per hostname, three total)

For each of `infinite-scroll-upload`, `infinite-scroll-printer`, `infinite-scroll-library`:

- Create a self-hosted Access Application for that exact hostname.
- Add two policies (both "Allow"):
  1. **Human session** — same identity provider/rule already used for
     other self-hosted apps on this account (e.g. "Emails ending in
     @<your domain>" or whatever the existing MediaWatch policy uses —
     copy it exactly rather than reinventing the rule).
  2. **Service token** — create one new Service Token per hostname (or
     reuse one across all three if that matches how `ART_ACCESS_CLIENT_ID`/
     `ART_ACCESS_CLIENT_SECRET` are already scoped — check the existing
     Access Application for `infinite-scroll-api.virtualdino.com` for the
     current convention before deciding).
- `library.*` technically only needs the human-session policy (nothing
  calls it as a service), but including the service-token policy too
  costs nothing and keeps all three Applications configured identically.

Record the resulting Client ID/Secret pairs in the secrets service (per
this project's standing policy — never leave them typed into a config
file in plaintext), named to match the existing `ART_ACCESS_CLIENT_ID`/
`ART_ACCESS_CLIENT_SECRET` convention, e.g. `UPLOAD_ACCESS_CLIENT_ID` /
`UPLOAD_ACCESS_CLIENT_SECRET`, and one pair each for `printer.*` and
`library.*` if using per-hostname tokens.

Also record the two application-level bearer tokens generated during Task
10's deployment (`UPLOADER_TOKEN` and `PRINTER_TOKEN` — live today in
`/etc/infinite-scroll/{uploader,printer}.env` on the Pi, `chmod 600`,
root-owned) in the secrets service. They were generated with
`openssl rand -hex 32` and are not recorded anywhere outside the Pi's own
env files yet.

## 4. After this is done

- Confirm `curl -I https://infinite-scroll-upload.virtualdino.com/health`
  (with no credentials) returns Cloudflare Access's login challenge, not
  a raw 200 — proves the Tunnel + Access wiring is live before anything
  tries to use it for real.
- Update the MCP-layer secrets (Task 12) with the real hostnames and
  service-token credentials once they exist.
