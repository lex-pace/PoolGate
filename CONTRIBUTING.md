# Contributing to PoolGate

Thanks for helping improve PoolGate. Please read the security policy before working with credentials, OAuth flows, gateway authentication, or Token Monitor collectors.

## Development environment

- Node.js 20 or newer
- pnpm 11
- Rust stable (edition 2021)
- Tauri 2 system prerequisites for the target platform
- Linux contributors may also need WebKitGTK, AppIndicator, DBus, and related Tauri build packages

Install dependencies with the repository's canonical package manager:

```bash
pnpm install
```

Run the desktop app during development:

```bash
pnpm run tauri dev
```

## Required checks

Before opening a pull request, run the checks relevant to your change:

```bash
pnpm run typecheck
pnpm run build
pnpm run check:rust
pnpm run test:rust
pnpm run fmt:check
pnpm run lint:rust
```

If a check cannot run on your platform, explain that in the pull request. Do not include real credentials, user databases, logs, OAuth tokens, or private agent transcripts in commits or fixtures.

## Change guidelines

- Keep changes focused and preserve existing behavior unless the change explicitly requires otherwise.
- Add or update Rust tests for routing, authentication, migrations, credential handling, imports, and collectors when applicable.
- Add frontend tests or a manual verification plan for UI and Tauri IPC changes.
- Use incremental database migrations; do not edit an already released migration in place.
- Redact secrets in diagnostics and test data.
- Update README, CHANGELOG, or security documentation when user-visible behavior or data handling changes.
- Avoid adding dependencies without explaining why an existing dependency cannot be used.

## Pull requests

A pull request should include:

- a concise description of the problem and solution;
- affected platforms and manual verification steps;
- tests and checks that were run;
- migration, privacy, security, or compatibility impact;
- screenshots for substantial UI changes.

Keep unrelated generated files, local previews, and build output out of the pull request.

## Commit and release scope

Do not commit user data or credentials. Release versions are created from reviewed tags and must include the corresponding changelog entry and reproducible build information.
