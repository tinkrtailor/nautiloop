# Dashboard Setup

The nautiloop mobile dashboard is a server-rendered web interface served by the control plane at `/dashboard/*`. It provides loop monitoring, one-tap actions (approve/cancel/extend), fleet-level analytics, and a notification feed — all optimized for mobile.

## Security Model

### Primary defense: network-level isolation

The dashboard inherits its security posture from the deployment. **Do not expose `/dashboard` to the public internet without fronting it with an auth proxy.**

The recommended deployment uses Tailscale to restrict access to devices on your tailnet:

1. The control plane binds to a Tailscale IPv4 address (configured in `terraform/examples/hetzner`).
2. Only devices joined to your tailnet can reach the dashboard.
3. Add your phone to Tailscale, then bookmark `https://<nautiloop-ts-ipv4>/dashboard`.

This gives you a private surface with zero additional auth infrastructure.

### Secondary defense: API key cookie

The dashboard requires the same shared API key used by `nemo` CLI. On first visit, `/dashboard/login` prompts for the key and an engineer name. On success, an HttpOnly, Secure, SameSite=Strict cookie is set (7-day expiry).

- The cookie is **not readable by JavaScript** (HttpOnly) — XSS cannot exfiltrate the key.
- The `Secure` flag ensures the cookie is only sent over HTTPS (automatically relaxed for `localhost`/`127.0.0.1` binds during local development).
- `SameSite=Strict` prevents CSRF from external origins.

Programmatic access via `Authorization: Bearer <key>` header is also supported, matching existing CLI behavior.

### What this is NOT

- **Not a user database.** There are no per-engineer accounts, passwords, or SSO. The cluster has a single shared API key.
- **Not a permission boundary.** The `Mine` vs `Team` toggle is a view filter. Any authenticated user can see all loops. This matches the CLI's behavior and is appropriate for small, trusted teams.

## Alternative deployments

If you cannot use Tailscale, front the control plane with an auth proxy:

- **oauth2-proxy** — adds Google/GitHub/OIDC sign-in upstream of the dashboard.
- **Authelia** — self-hosted MFA gateway.
- **Cloudflare Access / AWS ALB with Cognito** — managed alternatives.

In all cases, the API key cookie remains as defense in depth.

## Pricing configuration

To display cost figures (instead of `—`) throughout the dashboard, add a `[pricing]` section to `nemo.toml`:

```toml
[pricing]
[pricing.models]
"claude-sonnet-4-20250514" = { input_per_million = 3.00, output_per_million = 15.00 }
"claude-opus-4-20250514"  = { input_per_million = 15.00, output_per_million = 75.00 }
```

Without this section, token counts are still displayed but all cost fields show `—`. The pricing config is loaded at startup; restart the control plane to pick up changes.

## Quick start

1. Ensure the control plane is running and reachable from your device.
2. Open `https://<host>/dashboard` in a mobile browser.
3. Enter the API key and your engineer name on the login page.
4. The card grid loads with your active loops. Tap any card for details.
