# Generalize the Content Pipeline

## Execution Protocol

- Use TDD for every implementation step: add a failing test, run it and confirm the expected failure, make the smallest implementation change, run the focused test, then run the relevant package checks.
- Stop after each step and wait for explicit approval before starting the next step.
- Keep each hand-authored step under 200 changed lines. Report the diff size before requesting approval.
- Do not add comments, README prose, or other explanatory text to implementation changes.
- Treat dependency-lockfile generation separately from hand-authored changes and report it explicitly in the step summary.

## Summary

Replace the Markdown-only content pass with a source-kind pipeline supporting primary Markdown content, explicitly templated content with any extension, and verbatim content copies. Templating is a shared capability: Markdown is not a non-templated category. The pipeline discovers all content through the Rust `ignore` crate, renders explicit templates after primary pages, and exposes site collections so feeds can be built without type-specific code.

The v6 feed remains green in this slice, but it is intentionally a placeholder feed rather than the complete eventual Atom implementation.

## Content Contracts

- Discover content under `content/` with the `ignore` crate so hidden paths and paths excluded by ignore files are omitted.
- Always exclude `_data.lx.yaml` from content handling.
- Treat Markdown as primary content and an implicit templated source, not as a non-templated alternative.
- Classify `.md` sources as primary Markdown content with built-in page output semantics.
- Classify a source with an exact `lx` extension anywhere in its extension chain as explicitly templated arbitrary content.
- Classify every remaining source as a raw file and copy it verbatim to the same path relative to the output root.
- Derive a templated output path by removing the discovered `lx` extension(s) and preserving every other extension.
- Support terminal `lx`: `foo.lx` emits `foo`.
- Support arbitrary extension chains: `foo.lx.fancy.extension.chain` emits `foo.fancy.extension.chain`.
- Remove every `lx` extension found.
- Reject templated sources whose output ends in `htm`, case-insensitively. `.html`, not `.htm`, is the supported HTML extension.
- Sort discovered sources deterministically by source path.

## Template Contracts

- A rendered source participates in the same metadata, layout, body-rendering, and optional-layout contract regardless of whether it is primary Markdown or an explicit template.
- An explicit template may begin with YAML front matter delimited by `---` lines.
- A source without an opening fence is entirely template body.
- An opening fence without a closing fence is a build error.
- Front matter must parse as a YAML mapping. Empty front matter is an empty mapping.
- `layout`, when present, must be a string naming an exact template, such as `feed.jinja`.
- Resolve `layout` from the source’s own metadata first, then inherit the nearest layout from the data cascade using the source’s parent directory.
- When a layout resolves, pass the rendered body to the layout as `content` and render the named layout.
- When no layout resolves, write the rendered body directly. A separate layout is never required.
- Render an explicit template body as Jinja before applying its resolved layout. Primary Markdown uses its existing Markdown body processor and Jinja text-node rewriting before applying its resolved layout.
- Expose `config`, `data`, `path`, `source`, `build.date`, `pages`, `posts`, and `archive` to every explicit template.
- Expose `content` additionally when rendering a layout.
- Represent `path` as a relative `RootedPath`, allowing normal path serialization and `url_for(path, config)`.
- Capture one UTC timestamp per build and expose it as RFC 3339 `build.date`.
- Order `pages` by output path. Order `posts` newest-first, with output path as the tie-breaker. Build `archive` from the completed Markdown items.

## Output Contracts

- Build an in-memory output registry for every intended destination before writing files.
- Include Markdown pages, explicit template outputs, raw content copies, `_static` copies, and generated CSS in the output registry.
- Reject duplicate normalized output paths and identify both source paths.
- Reject ancestor/descendant output paths, such as `foo` and `foo/bar.xml`, because one producer requires a file where another requires a directory.
- Preserve existing Markdown output behavior: `<path>/index.html` with Markdown layouts.
- Write templated outputs directly at the derived path without appending `index.html`.
- Inject live reload in Serve mode into Markdown output and any templated output whose final extension is exactly `html`, case-insensitively.
- Never inject live reload into raw copies, XML, JSON, text, or other non-HTML output.

## V6 Feed

- Convert `sites/v6/content/feed.lx.xml` to fenced YAML with `layout: feed.jinja`.
- Replace the broken feed template with a minimal valid Atom shell using only title, site URL, configured author data, and `build.date`.
- Do not implement feed entries, taxonomy collections, missing template imports, or the full RSS/Atom item graph in this slice.
- Keep the feed source discovered so it emits `public/feed.xml`.

## TDD Steps

Each step ends with a focused test run, relevant package checks, a diff summary, and an approval request.

1. Add the `ignore` dependency and generated lockfile changes; verify the existing test suite still passes.
2. Add tests for primary Markdown, extension-chain discovery, and explicit templated output derivation; implement classification and output-path logic without treating Markdown as non-templated.
3. Add tests for fenced, absent, empty, malformed, and unclosed front matter; implement source loading, metadata parsing, and layout extraction.
4. Add tests for a standalone body template with no layout; add source-aware string rendering and the shared template render helper.
5. Add tests for local and cascade-inherited layouts, fixed build time, collection ordering, and `url_for(path, config)`; implement the shared layout resolver, explicit-template context, and post-page rendering pass.
6. Add tests for ignored, hidden, `_data.lx.yaml`, and ordinary raw content; implement the `ignore` walk and verbatim-copy stage. (Note: binary files are intentionally treated as an error.)
7. Add tests for duplicate and ancestor/descendant output collisions across Markdown, templates, raw files, static files, and CSS; implement the output registry and validation before writes.
8. Add tests for HTML live reload in Serve mode, non-injection for non-HTML outputs, and the `.htm` hard error; implement output-type handling.
9. Migrate the v6 feed source and placeholder template; verify the complete v6 build and `public/feed.xml`.

## Acceptance

- `cargo fmt --check` passes.
- `cargo clippy -p lx` passes.
- `cargo test -p lx` passes.
- A v6 publish succeeds and emits `public/feed.xml`.
- A standalone `.lx` source can emit an extensionless file.
- A standalone `.lx.html` source receives live reload in Serve mode.
- A templated `.lx.xml` source does not receive live reload.
- No implementation step exceeds 200 hand-authored changed lines.

## Assumptions

- `ignore` means the crate named `ignore`, with its normal hidden-file and ignore-file behavior.
- Raw copies preserve bytes and relative paths, except for hidden, ignored, and `_data.lx.yaml` files. (Note: UTF-8 is required, so this still holds even when files are invalid, because they will intentionally fail the build.)
- Collision detection covers exact paths and ancestor/descendant path conflicts.
- Existing Markdown metadata and output behavior is unchanged apart from sharing the common layout resolver and rendering helper.
- No README or implementation comments are added in this work.

## Addendum

In future work, update `Archive` to follow the same model as `Collection`: it should refer to content by `Id` rather than depending on `Item`/`Post` directly. `Archive` does not need the item itself; it only needs the `Id` that points to the relevant content.
