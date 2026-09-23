# Project guidance

## Read first

- `README.md` explains the project and current status.
- `components.md` defines the build order and links the learning resources.
- `NVIDIA_LOCAL_PIPELINE.md` defines the NVIDIA processing direction.

## Working rules

- Keep the product focused: import one video, enhance locally, and save it with synchronized audio.
- Implement in `app/`. Keep supporting documents in `documents/` and implementation plans in `plans/`.
- Use the root Git repository; do not initialize another repository inside `app/`.
- Build the complete working pipeline before optimizing it or polishing the UI.
- Use NVIDIA VSR and VFG with their local pretrained models. Do not bring in the original application's model stack.
- Keep the implementation simple. Choose the language and media bindings when implementing; the scaffold does not prescribe a stack.
- Use Context7 for current library/SDK/API documentation. If the relevant documentation is unavailable there, consult official vendor documentation. Existing NVIDIA reading links are in `components.md`.
- Keep models, SDK downloads, video assets, exports, and credentials out of Git. Store shared assets in separate versioned locations without overwriting the original project's assets.
- Preserve source videos. Check exported video and audio before reporting success.
- Run checks appropriate to each change and report what was verified. No application build or test commands exist yet.
