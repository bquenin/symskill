# Releasing

To release:

```bash
just release
```

This will bump the version in Cargo.toml, tag, and push. The GitHub Actions release workflow then builds platform binaries, creates the GitHub release, and updates the Homebrew tap.
