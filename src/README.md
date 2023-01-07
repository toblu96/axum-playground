# Axum playground

This project acts as playground for different axum features.

## Dev (Powershell)

Run the project in watch mode.
Needs crates `cargo-watch`.

```bash
cargo install cargo-watch
```

Start project in watch mode, each file change results in a restart of the application.

```powershell
$Env:RUST_LOG="trace"; cargo watch -x 'run'
```
