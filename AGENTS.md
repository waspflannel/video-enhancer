# Project guidance

## Read first

- `README.md` explains the project and current status.
- `components.md` defines the build order and links the learning resources.
- `NVIDIA_LOCAL_PIPELINE.md` defines the NVIDIA processing direction.
- `documents/coding-standards.md` defines naming, simplicity, validation, formatting, and verification conventions. Follow it for application changes.

## Working rules

- Keep the product focused: import one video, enhance locally, and save it with synchronized audio.
- Implement in `app/`. Keep supporting documents in `documents/` and implementation plans in `plans/`.
- Use the root Git repository; do not initialize another repository inside `app/`.
- Build the complete working pipeline before optimizing it or polishing the UI.
- Use NVIDIA VSR and VFG with their local pretrained models. Do not bring in the original application's model stack.
- Keep the implementation simple: Rust and Cargo, targeting Windows x64. See `documents/development.md`. Choose the desktop UI framework and NVIDIA/media bindings when implementing those components.
- Use Context7 for current library/SDK/API documentation. If the relevant documentation is unavailable there, consult official vendor documentation. Existing NVIDIA reading links are in `components.md`.
- Keep models, SDK downloads, video assets, exports, and credentials out of Git. Local copies live in this project's ignored `tools/`, `sdk/`, and `sample-videos/` directories. Do not depend on or modify the other project's shared assets.
- Preserve source videos. Check exported video and audio before reporting success.
- Treat this as a personal tool for normal local videos. Keep practical runtime errors; avoid speculative validation and repeated checks of library guarantees.
- Use descriptive names and small functions that make the processing flow obvious. Keep comments brief and function parameter lists on one line.
- Run checks appropriate to each change unless the user defers testing, and report what was verified. Use Cargo from `app/`. The user removed the automated tests; do not recreate them unless requested.
