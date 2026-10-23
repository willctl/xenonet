# Security

Xenolith is for a trusted local machine. The HTTP service binds to loopback and rejects other Hosts and unrelated browser origins. It does not authenticate OS users.

## Who Xenolith trusts

- **Local is any process on this machine.** A request without a token is local when it reaches the loopback port directly, names a loopback Host and carries no `Forwarded`, `X-Forwarded-*` or `Via` header. Local callers get what `access.json` says for `"local"`, admin by default. That includes your agents, other apps, and other OS accounts on the machine.
- **Tokens scope remote access and other apps' access.** They do not separate your own agents from each other: an agent that runs as you can read anything you can.
- **Sync always needs a token**, even from this machine. `/status`, `/ui` and `/api` take local requests only and never a token. `/health` is open and says nothing private.
- **A plain tunnel looks local.** A forwarder that adds none of those headers (`ssh -L`, socat, a TCP tunnel) is indistinguishable from a local process. If anyone reaches the port that way, run `xenolith access local none` and give each client its own token ([README](README.md#who-is-local)).
- The window's `X-Xenolith` header stops cross-site browser requests. It is not a password.

Do not use this service to isolate mutually untrusted users.

## Data and model access

The wiki, history, inbox, request records and vector cache are local files. Protect their permissions and backups. The secret detector is a best-effort guard, not a guarantee that every credential or personal detail will be recognized.

The curator and Ask send selected content to the configured model through its CLI. Optional embeddings send wiki sections and queries to OpenRouter. Enable external processing only when that is appropriate for the data. Provider retention settings are requests to the provider, not a local confidentiality boundary.

Notes and saved change records are untrusted input. Paths from those records must remain inside the wiki. Links and Windows reparse points below the configured wiki root are refused by the audited storage paths. The configured root itself may be a link. These checks do not defend against a process with write access racing filesystem changes, or against hard links created by the owner.

Automatic approval applies flagged curator changes. Choose manual approval when changes should wait for review. Source labels come from the caller and are not authenticated proof of authorship.

## Development and publication

Use synthetic data and an isolated server for verification. Never test against an installed service or a private wiki. Keep generated artifacts, machine reports, credentials and local evaluation questions out of Git.

The public Git identity is `willctl <will@willctl.com>`. Configure it locally, and run `node scripts/check-public.mjs` in this folder before publishing. Local private blocked terms may be stored in `.git/privacy-blocked.json`; never commit that file. The checks reduce accidental disclosure but do not prove anonymity.

See [the audit](docs/security-audit.md) for verified fixes, dependency warnings and remaining work. Report a suspected vulnerability through GitHub private vulnerability reporting when available; do not put secrets in a public issue.
