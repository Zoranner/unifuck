# unifuck

Rust and Slint desktop utility for local Unity Hub and Unity Editor patch flows.

## Project Structure

- `src/main.rs`: application startup and top-level window setup.
- `src/core/`: local Unity installation discovery, license file operations, binary patching, hosts updates, and shared licensing client operations.
- `src/hub/`: Unity Hub discovery, ASAR read/write support, and version-specific Hub patch strategies.
- `src/gui/`: Slint application state, event bindings, and background jobs used by the UI.
- `ui/`: Slint window, reusable UI components, visual effects, layouts, styles, and embedded assets.
- `tests/`: regression tests for ASAR handling and UI layout contracts.
- `.refs/`: local reference material only. It is ignored by Git and is not part of the maintained project source.

## Development

Use the Rust toolchain and Cargo. Do not use Unity Editor, Unity batchmode, or Unity-generated project files as verification for this repository.

```powershell
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

For read-only format checks:

```powershell
cargo fmt --all -- --check
```

## Release

Release packaging is defined in `.github/workflows/release.yml` and runs on version tags matching `v*`.

The release workflow currently checks formatting, clippy, and tests before building the Windows x64 executable archive.
