# Vendored tree-sitter grammars

These eight grammars are vendored as C and compiled by `crates/dae-qt/build.rs`
against the single `tree-sitter` runtime the crate already links. Five are
vendored because their published Rust bindings depend on an old `tree-sitter`
(0.20–0.22) that would link a second copy of the C runtime and collide with the
0.27 one (Kotlin's upstream main has migrated to `tree-sitter-language` but no
compatible crate is published yet). LaTeX is vendored because its published
crate (0.1.0) ships a `parser.c` that references an external scanner but omits
`scanner.c`, so the crate fails to link; the scanner here comes from upstream
master, whose twelve external tokens match the crate's parser exactly.
Gitignore has no published crate at all (its bindings target `tree-sitter
~0.20` and were never released). Groovy's published crate (amaanq's
`tree-sitter-groovy`) is an independent rewrite whose node names do not match
the upstream/nvim-treesitter query, so the grammar nvim-treesitter actually
uses (murtaza64) is vendored instead.

Each directory holds the generated `parser.c`, an optional external
`scanner.c`, the `tree_sitter/` headers the parser includes, and the
`highlights.scm` query consumed via `include_str!`. Only the files needed to
compile and highlight are kept; `grammar.json`/`node-types.json` are omitted.

| Directory    | Upstream                                            | License |
| ------------ | --------------------------------------------------- | ------- |
| `dockerfile` | camdencheek/tree-sitter-dockerfile (crate 0.2.0)    | MIT     |
| `gitignore`  | shunsambongi/tree-sitter-gitignore (commit f4685bf) | MIT     |
| `groovy`     | murtaza64/tree-sitter-groovy (commit 2a6ddd5, the nvim-treesitter pin) | MIT |
| `json5`      | tree-sitter/tree-sitter-json5 (crate 0.1.0)         | MIT     |
| `kotlin`     | fwcd/tree-sitter-kotlin (commit d6af96d)            | MIT     |
| `latex`      | latex-lsp/tree-sitter-latex (crate 0.1.0 parser, master scanner) | MIT |
| `less`       | jimliang/tree-sitter-less (crate 1.0.0)             | MIT     |
| `scss`       | tree-sitter-grammars/tree-sitter-scss (crate 1.0.0) | MIT     |

The parsers are ABI 13–15, within the 13–15 range the linked runtime accepts.
To refresh one, re-copy `src/parser.c`, `src/scanner.c`, `src/tree_sitter/*.h`
and `queries/highlights.scm` from the matching upstream release. Gitignore and
Groovy have no releases, and Kotlin's published crate lags its main branch;
refresh those from the pinned commit (or a newer nvim-treesitter pin for
Groovy) and re-apply the query adaptations below. Kotlin's `parser.c` is ~35 MB
and dominates the crate's build time; keep it at the default optimization
level rather than raising it.

## Query-only directories

`graphql/` and `proto/` hold only a `highlights.scm`. Their grammars come from
crates.io, but those crates either ship no highlight query (GraphQL) or comment
out the export (Proto), so the query is vendored here and read with
`include_str!`. `build.rs` does not compile these.

| Directory | Query source                                        | License    |
| --------- | --------------------------------------------------- | ---------- |
| `graphql` | nvim-treesitter `queries/graphql/highlights.scm`    | Apache-2.0 |
| `proto`   | tree-sitter-proto crate 0.6.0 `queries/highlights.scm` | MIT     |

## Adapting upstream queries

The GraphQL, LaTeX, Gitignore and Kotlin queries come from nvim-treesitter
(Apache-2.0; Kotlin's via fwcd, who already removed the `#lua-match?`
patterns); Proto's comes from its crate (MIT) and Groovy's from its own
repository (MIT). They need mechanical adaptation before the
tree-sitter-highlight engine renders them correctly, because the engine's
predicate and capture semantics differ from nvim's:

- `#lua-match?` predicates are ignored by the engine (unknown predicates are
  always true), which turns narrowed captures into catch-alls — every LaTeX
  command and every comment would paint as a keyword. They are rewritten to
  `#match?` with an equivalent Rust regex (the `\if…` conditionals and the
  `% !TeX` / `%&` directive comments).
- `@spell` / `@nospell` / `@none` captures are stripped. They mean nothing to
  the engine, and an unrecognized capture that lands last on a node erases
  that node's highlight entirely (the comment captures in Gitignore, Groovy,
  Less and SCSS, GroovyDoc tags, Dockerfile's `expansion`, and Kotlin's string
  interpolations).
- `@character.special` (Gitignore wildcards, Groovy's wildcard import) is
  renamed to `@punctuation.special`: the palette has no `character` entry, so
  the capture would otherwise land unrecognized and erase the node.
- Kotlin's query also speaks in capture names the palette lacks; they are
  renamed onto existing entries: `@namespace` → `@module`, `@include` →
  `@keyword`, `@parameter` → `@variable.parameter`, `@float` → `@number`,
  `@character` → `@string`, `@string.regex` → `@string.regexp`, and
  `@conditional` / `@repeat` / `@exception` → `@keyword.conditional` /
  `@keyword.repeat` / `@keyword.exception` (which fold onto `keyword`).
- Predicate-only captures like `@_name` are renamed to `@function` for the
  same reason; the predicates reference the renamed capture.
- `(#set! @_hyperlink url @markup.link.url)` was removed from the LaTeX
  `hyperlink` pattern: the engine rejects `set!` with a second capture name.

Re-apply these adaptations when refreshing a query from upstream. VB.NET has
no compatible highlight query and renders as plain text.