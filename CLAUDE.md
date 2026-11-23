# CLAUDE.md

Read [AGENTS.md](AGENTS.md) first — it applies to you too, starting with the "Carrot fork: git workflow" section.

The process rule that outranks everything else: **never commit directly to `main`**. Daily work lands on `develop`. `main` is the release branch — every push to it automatically builds a Docker image and publishes it to `ghcr.io/paique/pumpkin`, so a broken commit there ships broken servers to every host running the image.
