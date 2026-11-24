# Handoff: error handling

## Where things stand

Start from `mu` (`lx: error on .htm extensions`). The working copy was empty when this note was written; `cargo test -p lx` passed, `cargo fmt --check` was clean, and `cargo run -- publish sites/v6` exited 0.

## What prompted this

Rejecting `.htm` laid bare a display problem that affects every page-level error. Running `cargo run -- publish` against a site with `content/404.md` carrying `permalink: 404.htm` prints:

```
Error: could not prepare 1 pages for rendering
/private/tmp/lx-htm4/content/404.md:
	output '404.htm' ends in '.htm'; write it as '.html' instead
output '404.htm' ends in '.htm'; write it as '.html' instead
```

The last line is a duplicate. `PageError`'s `Display` (`lx/src/build.rs`) writes `"{path}:\n\t{error}"` for each failure and then calls `error::write_to_fmt`, which writes the root error again before walking `source()`. The same duplication shows up for unrelated failures, for example a missing `base.jinja`, where the second and third lines are the error and its nested `minijinja::Error`.

## What is wanted

Improving error handling generally. Which problems to address, and what the output should look like instead, are open and to be decided in discussion — treat the code layout, the error types, and the printed format as things to agree on first rather than as work to infer from this note.
