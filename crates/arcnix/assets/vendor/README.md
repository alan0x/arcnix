# Bundled desktop assets

These files are embedded into the desktop executable at build time; no CDN,
Node runtime or package manager is required at runtime or for normal Cargo builds.

| Package | Version | Files | License |
| --- | --- | --- | --- |
| `@xterm/xterm` | 6.0.0 | `xterm/xterm.js`, `xterm/xterm.css` | `xterm/LICENSE` (MIT) |
| `@xterm/addon-fit` | 0.11.0 | `addon-fit/addon-fit.js` | `addon-fit/LICENSE` (MIT) |
| `codemirror` | 5.65.20 | Text Editor standalone tokenizer and language modes in `codemirror/modes.js` | `codemirror/LICENSE` (MIT) |
| Workbench terminal renderer (`@xterm/xterm` and addons) | Pinned in the workbench lockfile | Included in the workbench | `xterm/LICENSE` (MIT) |
| `@codingame/monaco-vscode-api`, service overrides, built-in grammars and Chinese language pack | 39.0.5 | `workbench/workbench.gz` | `workbench/monaco-vscode-api-LICENSE` |
| VS Code sources and TextMate grammars | 1.145.2 | Included in the workbench | `workbench/vscode-LICENSE`, `workbench/vscode-ThirdPartyNotices.txt` |
| `markdown-it` | 15.0.3 | Included in the workbench | `workbench/markdown-it-LICENSE` |
| `prettier` | 3.10.1 | Code formatting worker in `workbench/workbench.gz` | `workbench/prettier-LICENSE` (MIT) |
| `@phosphor-icons/core` | 2.1.1 | Selected `assets/regular` and `assets/fill` SVGs in `phosphor/folder-icons.tsv` | `phosphor/LICENSE` (MIT) |

The full workbench dependency graph, npm tarballs and integrity hashes are pinned in
`tools/monaco/package-lock.json`. Run `npm ci` and `npm run build` in that directory
to regenerate the embedded bundle. The client serves its JavaScript, workers, fonts,
themes and translations from a local WebView protocol.

The Office command bar and Writer editing surface are supplied by the pinned syntra-office-ui dependency.
