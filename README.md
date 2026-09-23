# video-enhancer

Enhance videos locally using NVIDIA's SDK and pretrained models on an RTX 5070.

Import a video, choose its output resolution and frame rate, optionally generate intermediate frames with AI, and save the enhanced video with synchronized audio.

## Status

Planning and project scaffold only. The application has not been implemented, and NVIDIA packages have not been installed or benchmarked in this project.

Build one complete working pipeline first, then optimize its speed and polish the UI.

## Start here

- [Components and build order](components.md): what we build, with beginner reading links and SDK/model downloads.
- [NVIDIA pipeline](NVIDIA_LOCAL_PIPELINE.md): the technical direction.

## Project layout

```text
app/                       Application implementation goes here
documents/                 Supporting project documents
plans/                     Implementation plans
components.md              Component roadmap and learning resources
NVIDIA_LOCAL_PIPELINE.md   NVIDIA processing pipeline design
```

The project uses one Git repository at the root. Keep downloaded models, SDK archives, sample videos, and exports outside Git. There are no build or test commands yet; add them when the first component is implemented.
