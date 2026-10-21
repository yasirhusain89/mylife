# Changelog

Notable user-facing changes to Fotoflow. The git history (Conventional
Commits) is the source of truth; this file summarizes per release.
`docs/ROADMAP.md` tracks the phase queue; the README owns phase definitions.

## [Unreleased]

### Fixed
- Highlight videos no longer render upside down: the engine bakes EXIF
  orientation into stills and applies the video track's
  `preferredTransform` (e.g. the 180° rotation current phone footage
  carries) before the canvas crop
  (`sidecar/engine/Sources/engine/main.swift`).

### Added
- This changelog.
- Working `pnpm index:semantic` / `pnpm semantic:q` scripts; the semantic
  code index now covers `sidecar/` Swift sources alongside `src/`,
  `src-tauri/`, `scripts/`, and `e2e/`.
- GitNexus structural index for this repo (refresh with
  `gitnexus analyze --skip-agents-md`); the three code-search tiers are
  documented in `AGENTS.md` ("Code search").
