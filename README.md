# one2md

`one2md` converts Microsoft OneNote files into a folder of Markdown pages. It follows the logical object model in `[MS-ONE]` and uses the companion `[MS-ONESTORE]` and `[MS-FSSHTTPB]` encodings needed to read the physical file.

Supported inputs:

- `.one` section files from desktop OneNote or OneDrive
- `.onetoc2` notebook table-of-contents files
- `.onepkg` exported notebook packages

Each section becomes a subfolder and all of that section's pages are written directly inside it as `.md` files named from their page titles. A section-level `_index.md` links the pages in their original order and nests OneNote subpages according to their page levels. Duplicate names receive a numeric suffix. Every image and attachment from the conversion is stored in the single `<output>/_assets` directory.

The converter preserves rich-text emphasis, web hyperlinks, numbered and bulleted lists, task checkboxes, tables, images, attachments, stored handwriting-recognition text, and OfficeMath equations. OneNote math objects are emitted as `$...$` LaTeX, including fractions, roots, scripts, accents, limits, n-ary operators, delimiters, equation arrays, matrices, and mathematical Unicode symbols. Multi-row equations use the LaTeX `aligned` environment and align rows at their equals signs. Content without a Markdown equivalent is omitted with a visible HTML comment where appropriate.

## Build and use

```sh
cargo build --release
./target/release/one2md Notes.one
./target/release/one2md Notebook.onetoc2 -o notebook
./target/release/one2md Export.onepkg --output export
```

For the supplied example:

```sh
./target/release/one2md gm.one -o gm
```

This produces paths such as `gm/gm/Bayesian inference.md`, where the first `gm` is the requested output directory and the second is the OneNote section name.

To convert a directory of OneNote backups while selecting only the latest filesystem-modified copy of each section:

```sh
cargo build --release
python3 scripts/convert_onenote_backups.py /path/to/Backup -o backup-markdown
```

The script scans recursively, treats the directory and section name together as the section identity, and passes all selected `.one` files to one batch conversion. Selected backups are temporarily staged under their logical section names so backup date suffixes do not leak into output folder names. Their notebook and section-group folders are preserved relative to the backup root, while all assets remain in a single output-root `_assets` folder and every converted section receives a hierarchy-aware `_index.md`.

Use a Markdown viewer with KaTeX or MathJax enabled to render the emitted equations.

Run the test suite with:

```sh
cargo test
```

The parser is read-only. Asset names from OneNote are sanitized before they are written, and an output path that resolves to the input file is rejected.

## Format references

- `[MS-ONE]`: OneNote File Format (the supplied PDF)
- `[MS-ONESTORE]`: OneNote Revision Store File Format
- `[MS-FSSHTTPB]`: Binary Requests for File Synchronization via SOAP Protocol
