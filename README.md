# Dev Control Center

A desktop control panel for your local development environment. Manage your
project workspace, a Docker service stack, listening ports, and live container
logs from one window — on Windows, Linux, and macOS.

> **Status: pre-alpha, under active construction.**
> Nothing here is installable yet. Follow
> [`docs/rfc/20260806-dev-control-center-tauri.md`](docs/rfc/20260806-dev-control-center-tauri.md)
> for the architecture and the batch-by-batch build plan.

---

## What it does

| Tab | Purpose |
| :--- | :--- |
| **Workspace Projects** | Scans your project roots, detects the stack (Laravel, CodeIgniter, Rust, Go, Python, Next.js, Vite, Node, Docker), reads framework and Git status, and launches a dev server or terminal with the right PHP / Node / Go version already on `PATH`. |
| **Databases & Services** | Starts and stops a curated Docker stack (MySQL, PostgreSQL, MongoDB, Redis, Elasticsearch, Mailpit, MinIO, RabbitMQ, Kafka, Portainer, Caddy) and imports/exports databases without leaving the app. |
| **Ports & Processes** | Lists everything listening, shows which process owns each port, and lets you kill the ones you own — with a protected-process guard so you cannot shoot the OS. |
| **Live Logs** | Streams container logs and project log files in real time. |

Service definitions live in a JSON registry, so adding your own service is a
config change, not a code change.

## Design goals

- **No hardcoded environment.** Every path, distro name, and toolchain location
  is detected on first run and editable in Settings.
- **Works with either Docker setup.** Docker Desktop (native) and Docker running
  inside a WSL distro are both first-class, detected automatically.
- **No credentials in the repository, ever.** Stack credentials are generated
  randomly on first run and written to a gitignored `.env`.
- **Testable without Docker.** All domain logic runs against a mock command
  runner, so CI is meaningful on a bare runner.

## Requirements

Docker is the only hard prerequisite, in either form:

- **Docker Desktop** (Windows / macOS / Linux), or
- **Docker Engine inside WSL2** (Windows), or natively (Linux)

Optional, detected if present: Git Bash, Windows Terminal, PHP installs,
nvm, Go SDK, and your editors.

## Building from source

Requires [Rust](https://rustup.rs) 1.90+ and [Node.js](https://nodejs.org) 20+.

```bash
git clone https://github.com/muhananaufal/dev-control-center
cd dev-control-center
npm install
npm run tauri dev
```

Full build and packaging instructions land in Batch 8.

## Contributing

The project is being built in ordered batches described in the RFC. Until
Batch 8 lands, the surface is still moving — issues are welcome, pull requests
are premature.

## License

[MIT](LICENSE) © 2026 muhananaufal
