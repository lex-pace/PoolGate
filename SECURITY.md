# Security Policy

## Supported versions

PoolGate is currently in pre-release development. Until the first stable release, security fixes are applied to the default development branch on a best-effort basis.

| Version | Supported |
| --- | --- |
| `main` / `developer` | Yes, best effort |
| Older tags | No guarantee |

## Reporting a vulnerability

Please do **not** open a public GitHub issue for a suspected vulnerability. Report it privately through GitHub Security Advisories (preferred) or the security contact configured in the repository settings.

Include:

- affected version or commit;
- operating system and architecture;
- clear reproduction steps or a minimal proof of concept;
- impact and any required user permissions;
- logs or screenshots only after removing API keys, OAuth tokens, cookies, email addresses, and local paths.

We will acknowledge a report when possible, investigate it, and coordinate disclosure after a fix or mitigation is available. Please allow reasonable time for triage before public disclosure.

## Security boundaries

PoolGate is a local desktop gateway, not a hardened public internet service:

- the gateway defaults to `127.0.0.1`;
- LAN listening requires a gateway access key;
- API keys and OAuth credentials are sensitive user data;
- Token Monitor reads local tool usage files selected by the user;
- request and application logs may contain sensitive metadata even when credential redaction is enabled;
- custom provider URLs, proxy URLs, and headers should be treated as trusted configuration.

Do not expose a PoolGate port to the public internet without an independent access-control, TLS, firewall, and threat-model review.

## Before sharing diagnostics

Remove at least:

- `Authorization` and `Bearer` values;
- `x-api-key` and `x-goog-api-key` values;
- OAuth access and refresh tokens;
- provider custom headers and proxy credentials;
- database files, credential vaults, and local agent transcripts;
- private IP addresses and personally identifying account information.
