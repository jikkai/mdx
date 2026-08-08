# Contributing to @amamo/mdx

This repository ships one ESM package, seven platform-specific native packages, and a bilingual
documentation site. A complete change may cross the TypeScript orchestration layer and the Rust
native binding, so use the full quality gate before handing work off.

## Prerequisites

- Node.js 20.19 or newer. CI currently runs Node.js 24.
- pnpm 11.20.0.
- Rust 1.97.1 with `rustfmt` and `clippy`.
- A supported local target from [the native target list](./apps/docs/docs/native-targets.mdx).

## Set up the repository

```sh
pnpm install
pnpm run build
pnpm run check
```

`pnpm run build` compiles the native binding for the current machine before compiling TypeScript.
The generated `.node`, `native.d.ts`, `dist/`, `target/`, `.amamo-mdx/`, and documentation build
directories are intentionally ignored.

## Repository map

| Path                  | Responsibility                                                            |
| --------------------- | ------------------------------------------------------------------------- |
| `src/config.ts`       | Public configuration types, validation, defaults, and path normalization. |
| `src/compiler.ts`     | Discovery, operation serialization, watch mode, and generated outputs.    |
| `src/shiki.ts`        | Shiki engine and language loading.                                        |
| `src/vite.ts`         | Vite plugin lifecycle and watcher integration.                            |
| `src/next.ts`         | Next config wrapper and startup compiler.                                 |
| `src/next-loader.cts` | Read-only Next loader for compiled cache records.                         |
| `native/src/`         | MDX parsing, schema validation, HAST transforms, cache, and manifests.    |
| `src/__tests__/`      | Vitest integration tests using the real native binding.                   |
| `fixtures/next/`      | Next Turbopack and Webpack build fixture.                                 |
| `apps/docs/docs/`     | English and `zh-CN` documentation source.                                 |
| `.tours/`             | Source-anchored CodeTour walkthroughs for new maintainers.                |

See [the architecture codemap](./docs/CODEMAPS/architecture.md) for the end-to-end compile path.

## Commands

The following table is derived from the root `package.json` scripts.

<!-- AUTO-GENERATED:START package-scripts -->

| Command                   | Purpose                                                                           |
| ------------------------- | --------------------------------------------------------------------------------- |
| `pnpm run build:native`   | Build the native binding for the current platform and generate `native.d.ts`.     |
| `pnpm run build:ts`       | Compile TypeScript into `dist/`.                                                  |
| `pnpm run build`          | Build the native binding, then TypeScript.                                        |
| `pnpm run docs:dev`       | Start the local Doctrine documentation server.                                    |
| `pnpm run docs:build`     | Build the static documentation site.                                              |
| `pnpm run format`         | Format supported files with Oxfmt and Rustfmt.                                    |
| `pnpm run format:check`   | Check Oxfmt and Rustfmt without changing files.                                   |
| `pnpm run lint`           | Run Oxlint.                                                                       |
| `pnpm run lint:fix`       | Apply safe Oxlint fixes.                                                          |
| `pnpm test`               | Rebuild the package and run the Vitest suite.                                     |
| `pnpm run test:rust`      | Run the Rust unit tests.                                                          |
| `pnpm run typecheck`      | Type-check TypeScript without emitting files.                                     |
| `pnpm run check:docs`     | Type-check and build the documentation site.                                      |
| `pnpm run check`          | Run formatting, lint, Clippy, Rust tests, type-checking, Vitest, and docs checks. |
| `pnpm run prepublishOnly` | Stage npm native packages with napi-rs before publication.                        |
| `pnpm run release`        | Run the maintainer-owned Verso release workflow.                                  |
| `pnpm run prepare`        | Install the repository's Git hooks after dependency installation.                 |

<!-- AUTO-GENERATED:END package-scripts -->

`DOCS_SITE_URL` optionally sets the canonical site URL during `docs:build`; local builds default to
`http://localhost/`.

## Tests and documentation

- Keep spec files in `src/__tests__/` with a one-to-one source filename; non-spec helpers such as
  `fixture.ts` may be shared there.
- Resolve the real native binding in integration tests. Mock only external process or filesystem
  boundaries that the test does not own.
- Update the English and `.zh-CN.mdx` page together. Keep frontmatter order and reader-visible facts
  aligned between locales.
- Run `pnpm run check:docs` after changing MDX, links, navigation, or Doctrine configuration.
- Do not hand-edit generated files under `dist/`, `.amamo-mdx/`, `.doctrine/`, or native build output.

## Pull request checklist

- The change has one clear responsibility.
- Public behavior and configuration defaults match the implementation.
- Regression tests protect non-trivial behavior changes.
- Both documentation locales are updated when public behavior changes.
- `pnpm run check` passes.
- No generated or temporary planning files are included.
