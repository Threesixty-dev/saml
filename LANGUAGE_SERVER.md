# SAML language server (prototype)

## Get and install the server

The [SAML releases](https://github.com/Threesixty-dev/saml/releases) are the
distribution point for versioned language tools. When a `saml-language-vX.Y.Z`
release is available, download the archive for your OS and CPU architecture.
It contains `saml-lsp`, `saml-check`, and `saml-map` (with `.exe` suffixes on
Windows). Extract them into a directory on your `PATH`, or keep them in a
dedicated directory and use the absolute path to `saml-lsp` in your editor.
The release also includes `SHA256SUMS.txt` for verifying downloads.
Compare the downloaded file's SHA-256 digest with its line in that file before
extracting it.

For example, on an Apple Silicon Mac, after downloading the matching archive:

```sh
mkdir -p "$HOME/.local/bin"
tar -xzf saml-language-v0.3.8-macos-aarch64.tar.gz -C "$HOME/.local/bin"
```

Choose the archive matching your system and release version. Windows releases
use `.zip`; extract the three `.exe` files into a directory of your choice.

If a language release is not yet available, the `saml-lsp` source is in the
public [Threesixty-dev/saml repository](https://github.com/Threesixty-dev/saml).
Install Rust and Cargo, then build and install the binary from a checkout:

```sh
git clone https://github.com/Threesixty-dev/saml.git
cd saml
cargo install --locked --path . --bin saml-lsp
```

Cargo installs the binary in its bin directory (normally `~/.cargo/bin` on
macOS and Linux). Run `cargo install --list` to confirm that `saml-lsp` is
installed. The server speaks LSP over standard input and output; it is started
by an editor or other LSP client, rather than run interactively in a terminal.

If you are developing this repository, `cargo build --locked --bin saml-lsp`
instead creates `target/debug/saml-lsp`. To test the whole project, run
`cargo test --locked`.

## Use it in VS Code

Download `saml-vX.Y.Z.vsix` from the same language release, then install it
with `code --install-extension /path/to/saml-vX.Y.Z.vsix` or VS Code's
**Install from VSIX...** command. The VSIX and binaries are separate assets.

If you are building from source, the VS Code extension is in
[`vscode-saml/`](vscode-saml/). With Node.js, npm, and the VS Code `code`
command available, package and install it locally:

```sh
cd vscode-saml
npx @vscode/vsce@4.1.1 package --out saml.vsix
code --install-extension saml.vsix
```

Open the repository containing your `.spec` and `.saml` files in VS Code. Set
`saml.serverPath` in that workspace's settings to the **absolute path** of the
installed `saml-lsp` binary if VS Code cannot find it on `PATH`, for example:

```json
{
  "saml.serverPath": "/home/you/.cargo/bin/saml-lsp"
}
```

On macOS, the path is usually `/Users/you/.cargo/bin/saml-lsp`; on Windows,
use the absolute path to `saml-lsp.exe`. You can locate the binary with
`command -v saml-lsp` on macOS/Linux or `where saml-lsp` on Windows. Without
`saml.serverPath`, the extension first looks for `target/debug/saml-lsp` under
the first open workspace folder and then searches VS Code's `PATH`. The
extension activates for `*.saml` and `*.spec` files. Server output appears in
VS Code's **SAML Language Server**
output channel; missing-binary and initialization errors appear as VS Code
notifications.

For another LSP-capable editor, configure `saml-lsp` as a stdio server for
`.saml` and `.spec` files and open the directory containing the model as the
workspace root. The server reads vocabulary from that workspace's `.spec`
files and `spec.saml`.

## Language features

Only `boundary`, `concept`, and `rule` are built-in keywords. A `.spec` file can
introduce vocabulary with declarations such as `concept Entity {}` and
`boundary Scope {}`. These become `entity` and `scope` in model files. The
former `boundry` spelling produces a rename diagnostic in spec files.
Vocabulary is workspace-wide: every `.spec` in the open workspace contributes
words to all model files in that workspace, regardless of directory. For the
existing example, `spec.saml` is also read as a manifest. VS Code gives built-in
keywords, spec-defined vocabulary, and property names distinct semantic token categories.
The static grammar colors comments, strings, names, and punctuation; it does not
maintain a list of user-defined words. The `samlVocabulary` token falls back to
the theme's `support.function` color, keeping it visually separate from model
names that use `entity.name.type`. `Public` and `Private` are highlighted as
keywords when used as access modifiers before a materialization kind.

`concept Entity` and `boundary Entity` give the same `entity` word two compatible
roles: concept and boundary. Hover shows both roles, and go to definition
includes both declarations.

Property names in a concept have their own identity. Hovering `environment` in
`concept ApplicationType` describes that property; go to definition on an
`environment:` assignment returns the property declaration. Hovering or
navigating from its `type: Environment` value still resolves the `Environment`
concept. Typed properties are required on materializations, directly or through
resolved `extends` bases. Find All References and Rename Symbol follow the
property across `.spec` declarations and model assignments without changing
same-named properties on other concepts.

`type: [Data]` in a property or materialization input schema requires a list
of `Data` references. A reference with a block, such as
`capability: RestApi { expose: [Article, Author] }`, binds inputs declared on
the `RestApi` materialization for that use. The server checks argument names,
value shapes, and resolved target kinds. The semantic map records bindings and
their target links.

Model materializations in a boundary are private by default. Put `Public`
immediately before the materialization kind to allow references from other
boundaries, as in `scope Identity Public entity User {}`. `Private` explicitly
states the default. An inaccessible typed reference produces an
`inaccessible-reference` diagnostic. The semantic map records visibility on
each occurrence and marks cross-boundary links.

The server supports full-document synchronization, diagnostics, document
symbols, go to definition, hover, completion, Find All References, and
workspace-wide Rename Symbol. Press Shift+F12 to find uses of `Interface` from
either its concept declaration or an `interface` model declaration. Press F2 to
rename both forms; typed property references and boundary qualifiers update too.

Materialization names are separate symbols. Go to Definition, hover, references,
and rename follow the resolved materialization identity, including typed property
values, binding arguments, `extends`, and explicit boundary qualifiers. Repeated
declarations of one identity are included together; same-named materializations
in other scopes are excluded. An unresolved, ambiguous, or inaccessible name
does not navigate to an arbitrary same-named declaration. Rename uses open
unsaved buffers and rejects invalid names, workspace collisions of the same
kind, and names with unresolved or untyped uses. Implicit boundary
materializations and `state` materializations cannot yet be renamed safely.
Fragments of one materialization merge distinct properties and compatible input
schemas. Duplicate declared property assignments across fragments, conflicting
input schemas, and mixed visibility produce semantic diagnostics.
Property renames reject conflicting rules, collisions and untracked uses in
their owning concept. A `state:` property key cannot yet be renamed because it
also participates in the current `state` declaration syntax.

The reusable Rust semantic analyzer is in `src/semantic.rs`. It composes spec
roles, maps workspace materializations, resolves typed references and `extends`
links, and returns semantic findings to the LSP. Run
`cargo run --bin saml-map -- project/example-1` to inspect its JSON output.
The custom LSP request `saml/semanticMap` returns the same map for open buffers.
Named model elements appear under its `materializations` key.
See `SEMANTIC_MODEL.md` for the current model and resolution rules.

Diagnostics cover unmatched `{}`, `[]`, `()`, unterminated quoted strings, and
the semantic findings described above. Other constructs whose semantics are not
yet specified are accepted. Declaration recognition uses the workspace
vocabulary, including `state: Name` lines when `State` is introduced by a spec.
A chain such as `layer Infrastructure scope Article {}` is represented as a `scope Article`
symbol when `Scope` is defined. The preceding `layer` is a qualifier.

## Rules still needed

The example does not determine a complete grammar or type system. In particular:

- Which properties may repeat within one declaration? `state:` and `stage:`
  occur more than once in the example, while the current spec types do not
  express that multiplicity.
- Should boundary kinds beyond `scope` and `layer` participate in lexical lookup
  differently from their current containment behavior?
- Which attributes and relations are legal for each declaration, and which are
  mandatory? The example uses both `from: Initial` and `from Initial`.
- Are semicolons optional everywhere, and are multi-line declarations always
  allowed?
- Are names case-sensitive? `UserPortalFrontEnd` in `infrastructure.saml` differs
  from `UserPortalFrontend` in `applications.saml`.

The semantic analyzer checks references whose target kind is declared in a
spec property rule or an `extends` clause. It does not yet validate unrestricted
prose relations or repeated assignments within one fragment. Renaming stops if
an untracked identifier with the same spelling could be left behind.

Run the tests with `cargo test`.

## Publishing a language release

The [language release workflow](.github/workflows/language-release.yml) tests
and packages the language tools for macOS (Apple Silicon and Intel), Linux
(ARM64 and x86-64), and Windows (x86-64), plus one VSIX. It checks that
`Cargo.toml` and `vscode-saml/package.json` have the same version. After the
changes pass CI and reach the intended release commit, tag that commit
`saml-language-vX.Y.Z` and push the tag. The workflow publishes a GitHub Release
only after every platform build and the VSIX package succeed. Pull requests
and manual workflow runs build artifacts without publishing a release.
