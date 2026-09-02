use onenote_parser::Parser;
use onenote_parser::contents::{
    Content, EmbeddedFile, FileDataStatus, Image, MathInlineObject, MathObjectType, Outline,
    OutlineElement, OutlineItem, ParagraphStyling, RichText, Table,
};
use onenote_parser::notebook::Notebook;
use onenote_parser::page::{Page, PageContent};
use onenote_parser::property::common::ColorRef;
use onenote_parser::section::{Section, SectionEntry};
use onenote_parser::warn::Report;
use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use typed_path::TypedPath;

/// Information about a completed conversion.
#[derive(Debug)]
pub struct ConversionSummary {
    /// Number of pages written.
    pub pages: usize,
    /// Number of image and attachment files written.
    pub assets: usize,
    /// Non-fatal warnings raised while parsing the input.
    pub warnings: Vec<String>,
    /// Markdown output path.
    pub output: PathBuf,
}

/// Convert a OneNote section, notebook, or package into a folder of Markdown pages.
pub fn convert_file(
    input: &Path,
    output: &Path,
) -> Result<ConversionSummary, Box<dyn Error + Send + Sync>> {
    convert_files([input], output)
}

/// Convert multiple OneNote inputs into one folder of Markdown sections.
pub fn convert_files<I, P>(
    inputs: I,
    output: &Path,
) -> Result<ConversionSummary, Box<dyn Error + Send + Sync>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    convert_files_internal(inputs, None, output)
}

/// Convert multiple inputs while preserving their parent paths relative to `source_root`.
pub fn convert_files_with_hierarchy<I, P>(
    inputs: I,
    source_root: &Path,
    output: &Path,
) -> Result<ConversionSummary, Box<dyn Error + Send + Sync>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    convert_files_internal(inputs, Some(source_root), output)
}

fn convert_files_internal<I, P>(
    inputs: I,
    source_root: Option<&Path>,
    output: &Path,
) -> Result<ConversionSummary, Box<dyn Error + Send + Sync>>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    if inputs.is_empty() {
        return Err("at least one OneNote input is required".into());
    }
    let skip_malformed_inputs = inputs.len() > 1;

    let parser = Parser::new();
    let mut renderer = FolderRenderer::new(output)?;
    let mut warnings = Vec::new();

    for input in inputs {
        let input = input.as_ref();
        protect_input_from_overwrite(input, output)?;
        let relative_parent = match source_root {
            Some(source_root) => {
                let parent = input.parent().unwrap_or_else(|| Path::new(""));
                let relative = parent.strip_prefix(source_root).map_err(|_| {
                    format!(
                        "input '{}' is outside source root '{}'",
                        input.display(),
                        source_root.display()
                    )
                })?;
                safe_relative_hierarchy(relative)
            }
            None => PathBuf::new(),
        };
        let extension = input
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        match extension.as_str() {
            "one" => {
                let section = match parser.parse_section(as_typed_path(input)?) {
                    Ok(section) => section,
                    Err(error) if skip_malformed_inputs => {
                        warnings.push(format!(
                            "{}: skipped malformed backup: {error}",
                            input.display()
                        ));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                collect_report(section.report(), &mut warnings);
                renderer.render_section_at(&section, &relative_parent)?;
            }
            "onetoc2" => {
                let notebook = match parser.parse_notebook(as_typed_path(input)?) {
                    Ok(notebook) => notebook,
                    Err(error) if skip_malformed_inputs => {
                        warnings.push(format!(
                            "{}: skipped malformed backup: {error}",
                            input.display()
                        ));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                collect_notebook_warnings(&notebook, &mut warnings);
                renderer.render_notebook_at(&notebook, &relative_parent)?;
            }
            "onepkg" => {
                let notebook = match parser.parse_package(as_typed_path(input)?) {
                    Ok(notebook) => notebook,
                    Err(error) if skip_malformed_inputs => {
                        warnings.push(format!(
                            "{}: skipped malformed backup: {error}",
                            input.display()
                        ));
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                collect_notebook_warnings(&notebook, &mut warnings);
                renderer.render_notebook_at(&notebook, &relative_parent)?;
            }
            _ => {
                return Err(format!(
                    "unsupported input type '{}'; expected .one, .onetoc2, or .onepkg",
                    input.display()
                )
                .into());
            }
        }
    }

    Ok(ConversionSummary {
        pages: renderer.pages,
        assets: renderer.assets.count,
        warnings,
        output: output.to_owned(),
    })
}

fn protect_input_from_overwrite(input: &Path, output: &Path) -> io::Result<()> {
    let input_path = fs::canonicalize(input)?;
    let same = match fs::canonicalize(output) {
        Ok(output_path) => output_path == input_path,
        // A path that does not exist cannot currently resolve to the existing input.
        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
        Err(err) => return Err(err),
    };

    if same {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output path must not overwrite the OneNote input",
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn as_typed_path(path: &Path) -> Result<TypedPath<'_>, io::Error> {
    use std::os::unix::ffi::OsStrExt;
    Ok(TypedPath::unix(path.as_os_str().as_bytes()))
}

#[cfg(not(unix))]
fn as_typed_path(path: &Path) -> Result<TypedPath<'_>, io::Error> {
    let value = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "input path is not valid Unicode",
        )
    })?;
    Ok(TypedPath::derive(value.as_bytes()))
}

fn collect_notebook_warnings(notebook: &Notebook, warnings: &mut Vec<String>) {
    collect_report(notebook.report(), warnings);
    collect_entry_warnings(notebook.entries(), warnings);
}

fn collect_entry_warnings(entries: &[SectionEntry], warnings: &mut Vec<String>) {
    for entry in entries {
        match entry {
            SectionEntry::Section(section) => collect_report(section.report(), warnings),
            SectionEntry::SectionGroup(group) => collect_entry_warnings(group.entries(), warnings),
        }
    }
}

fn collect_report(report: &Report, warnings: &mut Vec<String>) {
    warnings.extend(report.warnings().iter().map(|warning| {
        warning.page().map_or_else(
            || warning.message().to_owned(),
            |(_, title)| format!("{title}: {}", warning.message()),
        )
    }));
}

struct FolderRenderer {
    root: PathBuf,
    sections: HashSet<String>,
    pages: usize,
    assets: AssetWriter,
}

struct PageIndexEntry {
    title: String,
    filename: String,
    level: i32,
}

impl FolderRenderer {
    fn new(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.to_owned(),
            sections: HashSet::new(),
            pages: 0,
            assets: AssetWriter::new(root.join("_assets"), "../_assets".to_owned()),
        })
    }

    fn render_notebook_at(&mut self, notebook: &Notebook, parent: &Path) -> io::Result<()> {
        self.render_entries_at(notebook.entries(), parent)
    }

    fn render_entries_at(&mut self, entries: &[SectionEntry], parent: &Path) -> io::Result<()> {
        for entry in entries {
            match entry {
                SectionEntry::Section(section) => self.render_section_at(section, parent)?,
                SectionEntry::SectionGroup(group) => {
                    self.render_entries_at(group.entries(), parent)?
                }
            }
        }
        Ok(())
    }

    fn render_section_at(&mut self, section: &Section, parent: &Path) -> io::Result<()> {
        let preferred = safe_title_component(section.display_name());
        let preferred = if preferred.is_empty() {
            "Untitled section".to_owned()
        } else {
            preferred
        };
        let section_name = unique_section_name(preferred, parent, &mut self.sections);
        let section_dir = self.root.join(parent).join(section_name);
        fs::create_dir_all(&section_dir)?;
        self.assets.relative_directory = format!(
            "{}_assets",
            "../".repeat(parent.components().count().saturating_add(1))
        );
        let mut page_names = HashSet::from(["_index.md".to_owned()]);
        let mut index_entries = Vec::new();

        for series in section.page_series() {
            for page in series.pages() {
                self.pages += 1;
                let title = page_title(page, self.pages);
                let preferred = markdown_filename(&title);
                let page_name = unique_generated_name(preferred, &mut page_names);
                let page_path = section_dir.join(&page_name);
                let mut renderer = Renderer::new(&mut self.assets);
                renderer.render_page(page, &title)?;
                fs::write(&page_path, renderer.markdown.as_bytes())?;
                index_entries.push(PageIndexEntry {
                    title,
                    filename: page_name,
                    level: page.level(),
                });
            }
        }

        let index = render_section_index(section.display_name(), &index_entries);
        fs::write(section_dir.join("_index.md"), index.as_bytes())?;

        Ok(())
    }
}

fn render_section_index(section_title: &str, pages: &[PageIndexEntry]) -> String {
    let base_level = pages.iter().map(|page| page.level).min().unwrap_or(0);
    let mut markdown = format!("# {}\n\n", escape_markdown(section_title.trim(), false));

    for page in pages {
        let depth = page.level.saturating_sub(base_level).max(0) as usize;
        markdown.push_str(&"  ".repeat(depth));
        markdown.push_str("- [");
        markdown.push_str(&escape_markdown(&page.title, false));
        markdown.push_str("](<");
        markdown.push_str(&escape_link_target(&page.filename));
        markdown.push_str(">)\n");
    }

    markdown
}

fn safe_relative_hierarchy(path: &Path) -> PathBuf {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => {
                let name = safe_title_component(&value.to_string_lossy());
                Some(if name.is_empty() {
                    "Untitled folder".to_owned()
                } else {
                    name
                })
            }
            _ => None,
        })
        .collect()
}

fn unique_section_name(preferred: String, parent: &Path, used: &mut HashSet<String>) -> String {
    let mut candidate = preferred.clone();
    let mut suffix = 2;
    while !used.insert(parent.join(&candidate).to_string_lossy().to_lowercase()) {
        candidate = format!("{preferred}-{suffix}");
        suffix += 1;
    }
    candidate
}

fn page_title(page: &Page, number: usize) -> String {
    page.title_text()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Untitled page {number}"))
}

fn markdown_filename(title: &str) -> String {
    let name = safe_title_component(&normalize_title_for_filename(title)).replace('_', "-");
    let name = if name.is_empty() {
        "Untitled page".to_owned()
    } else {
        name
    };
    if name.to_ascii_lowercase().ends_with(".md") {
        name
    } else {
        format!("{name}.md")
    }
}

fn normalize_title_for_filename(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '(' => {
                normalized.truncate(normalized.trim_end().len());
                if !normalized.is_empty() {
                    normalized.push_str(" - ");
                }
            }
            ')' | '\'' | '’' => {}
            _ => normalized.push(ch),
        }
    }

    normalized
}

fn unique_generated_name(preferred: String, used: &mut HashSet<String>) -> String {
    let path = Path::new(&preferred);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled")
        .to_owned();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_owned);
    let mut candidate = preferred;
    let mut suffix = 2;
    while !used.insert(candidate.to_ascii_lowercase()) {
        candidate = extension.as_deref().map_or_else(
            || format!("{stem}-{suffix}"),
            |extension| format!("{stem}-{suffix}.{extension}"),
        );
        suffix += 1;
    }
    candidate
}

struct Renderer<'a> {
    markdown: String,
    assets: &'a mut AssetWriter,
    pending_math_rows: Vec<String>,
    pending_math_compact_before: bool,
    previous_outline_row_nonempty: bool,
    in_references: bool,
}

const TAB_INDENT: &str = "\u{2003}\u{2003}";
const REFERENCE_IMAGE_INDENT: &str = "\u{2003}";

impl<'a> Renderer<'a> {
    fn new(assets: &'a mut AssetWriter) -> Self {
        Self {
            markdown: String::new(),
            assets,
            pending_math_rows: Vec::new(),
            pending_math_compact_before: false,
            previous_outline_row_nonempty: false,
            in_references: false,
        }
    }

    fn render_page(&mut self, page: &Page, title: &str) -> io::Result<()> {
        self.heading(1, title);

        for content in page.contents() {
            self.render_page_content(content)?;
        }

        if let Some(recognition) = page.ink_recognition() {
            let text = recognition.text();
            if !text.trim().is_empty() {
                self.markdown.push_str("**Recognized handwriting**\n\n");
                for line in text.lines() {
                    self.markdown.push_str("> ");
                    self.markdown.push_str(&escape_markdown(line, false));
                    self.markdown.push('\n');
                }
                self.markdown.push('\n');
            }
        }

        if self.in_references && self.markdown.ends_with("\n\n") {
            self.markdown.pop();
        }

        Ok(())
    }

    fn render_page_content(&mut self, content: &PageContent) -> io::Result<()> {
        match content {
            PageContent::Outline(outline) => self.render_outline(outline),
            PageContent::Image(image) => {
                let block = self.render_image(image)?;
                self.push_block(&block);
                Ok(())
            }
            PageContent::EmbeddedFile(file) => {
                let block = self.render_embedded_file(file)?;
                self.push_block(&block);
                Ok(())
            }
            PageContent::Ink(_) => {
                self.push_block(
                    "<!-- OneNote ink drawing omitted; recognized text follows when available. -->",
                );
                Ok(())
            }
            PageContent::Unknown => {
                self.push_block("<!-- Unsupported OneNote page content omitted. -->");
                Ok(())
            }
        }
    }

    fn render_outline(&mut self, outline: &Outline) -> io::Result<()> {
        self.previous_outline_row_nonempty = false;
        self.render_outline_items(outline.items(), 0, false)?;
        self.flush_pending_math();
        self.previous_outline_row_nonempty = false;
        Ok(())
    }

    fn render_outline_items(
        &mut self,
        items: &[OutlineItem],
        depth: usize,
        indent_after_where: bool,
    ) -> io::Result<()> {
        for item in items {
            match item {
                OutlineItem::Group(group) => {
                    let group_depth = group.child_level().saturating_sub(1) as usize;
                    self.render_outline_items(
                        group.outlines(),
                        depth.max(group_depth),
                        indent_after_where,
                    )?;
                }
                OutlineItem::Element(element) => {
                    let block = self.render_outline_element(element)?;
                    let compact_before = self.previous_outline_row_nonempty;
                    let section_heading = outline_section_heading(element);
                    let image_only = element
                        .contents()
                        .iter()
                        .any(|content| matches!(content, Content::Image(_)))
                        && element.contents().iter().all(|content| match content {
                            Content::Image(_) => true,
                            Content::RichText(text) => text.text().trim().is_empty(),
                            _ => false,
                        });
                    let suppress_reference_image_bullet = self.in_references && image_only;
                    let force_reference_text_bullet = self.in_references && !image_only;
                    let task = task_state(element);
                    let list = element.list_contents().first();
                    let is_list = section_heading.is_none()
                        && (force_reference_text_bullet
                            || (!suppress_reference_image_bullet
                                && (task.is_some() || list.is_some())));

                    if let Some(heading) = section_heading {
                        self.flush_pending_math();
                        self.push_outline_heading(heading, compact_before);
                    } else if suppress_reference_image_bullet {
                        self.flush_pending_math();
                        self.prepare_reference_block(&block);
                        self.push_indented_block(&block, REFERENCE_IMAGE_INDENT);
                    } else if force_reference_text_bullet {
                        self.flush_pending_math();
                        self.prepare_reference_block(&block);
                        self.push_list_block(&block, depth, "- ");
                    } else if let Some(completed) = task {
                        self.flush_pending_math();
                        self.prepare_outline_block(&block, compact_before);
                        self.push_list_block(
                            &block,
                            depth,
                            if completed { "- [x] " } else { "- [ ] " },
                        );
                    } else if let Some(list) = list {
                        self.flush_pending_math();
                        self.prepare_outline_block(&block, compact_before);
                        let marker = if list.list_format().contains(&'\u{fffd}') {
                            match list.list_restart() {
                                Some(start) if start > 1 => format!("{start}. "),
                                _ => "1. ".to_owned(),
                            }
                        } else {
                            "- ".to_owned()
                        };
                        self.push_list_block(
                            &block,
                            depth + usize::from(indent_after_where),
                            &marker,
                        );
                    } else {
                        self.push_plain_block(&block, indent_after_where, compact_before);
                    }

                    self.previous_outline_row_nonempty = !block.trim().is_empty();

                    if let Some(heading) = section_heading {
                        self.in_references = heading == "References";
                    }

                    self.render_outline_items(
                        element.children(),
                        if is_list { depth + 1 } else { depth },
                        indent_after_where || block.trim().eq_ignore_ascii_case("where"),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn render_outline_element(&mut self, element: &OutlineElement) -> io::Result<String> {
        let mut blocks = Vec::new();
        for content in element.contents() {
            let block = match content {
                Content::RichText(text) => render_rich_text(text, false, self.in_references),
                Content::Table(table) => self.render_table(table)?,
                Content::Image(_) if self.in_references => String::new(),
                Content::Image(image) => self.render_image(image)?,
                Content::EmbeddedFile(file) => self.render_embedded_file(file)?,
                Content::Ink(_) => "<!-- OneNote ink drawing omitted. -->".to_owned(),
                Content::Unknown => "<!-- Unsupported OneNote content omitted. -->".to_owned(),
            };
            if !block.trim().is_empty() {
                blocks.push(block);
            }
        }
        Ok(blocks.join("\n\n"))
    }

    fn render_table(&mut self, table: &Table) -> io::Result<String> {
        let cols = table
            .contents()
            .iter()
            .map(|row| row.contents().len())
            .max()
            .unwrap_or(table.cols() as usize)
            .max(1);
        let mut rows = Vec::new();
        for row in table.contents() {
            let mut values = Vec::with_capacity(cols);
            for index in 0..cols {
                let value = if let Some(cell) = row.contents().get(index) {
                    let mut contents = Vec::new();
                    for element in cell.contents() {
                        let value = self.render_outline_element(element)?;
                        if !value.trim().is_empty() {
                            contents.push(value);
                        }
                    }
                    table_cell(contents.join("\n"))
                } else {
                    String::new()
                };
                values.push(value);
            }
            rows.push(values);
        }

        let Some((first, remaining)) = rows.split_first() else {
            return Ok(String::new());
        };
        let mut output = markdown_table_row(first);
        output.push('\n');
        output.push('|');
        for _ in 0..cols {
            output.push_str(" --- |");
        }
        for row in remaining {
            output.push('\n');
            output.push_str(&markdown_table_row(row));
        }
        Ok(output)
    }

    fn render_image(&mut self, image: &Image) -> io::Result<String> {
        let Some(reader) = image.read() else {
            return Ok("*[OneNote image data is unavailable]*".to_owned());
        };
        if image.data_status() != FileDataStatus::Available {
            return Ok("*[OneNote image data is invalid]*".to_owned());
        }

        let extension = safe_extension(image.extension().unwrap_or("bin"));
        let fallback = format!("image-{:04}.{extension}", self.assets.next_number());
        let preferred = image.image_filename().unwrap_or(&fallback);
        let path = self
            .assets
            .write_reader(preferred, Some(&extension), reader)?;
        let image_markdown = format!("![]({path})");

        Ok(match image.hyperlink_url() {
            Some(target) if !is_onenote_link(target) => {
                format!("[{image_markdown}](<{}>)", escape_link_target(target))
            }
            _ => image_markdown,
        })
    }

    fn render_embedded_file(&mut self, file: &EmbeddedFile) -> io::Result<String> {
        let display_name = if file.filename().trim().is_empty() {
            "OneNote attachment"
        } else {
            file.filename()
        };
        if file.data_status() != FileDataStatus::Available {
            return Ok(format!(
                "*Attachment unavailable: {}*",
                escape_markdown(display_name, false)
            ));
        }

        let fallback = format!("attachment-{:04}.bin", self.assets.next_number());
        let preferred = if file.filename().trim().is_empty() {
            fallback.as_str()
        } else {
            file.filename()
        };
        let path = self.assets.write_reader(preferred, None, file.read())?;
        Ok(format!(
            "[Attachment: {}]({path})",
            escape_markdown(display_name, false)
        ))
    }

    fn heading(&mut self, level: usize, title: &str) {
        let title = title.replace(['\r', '\n'], " ");
        self.markdown.push_str(&"#".repeat(level.clamp(1, 6)));
        self.markdown.push(' ');
        self.markdown
            .push_str(&escape_markdown(title.trim(), false));
        self.markdown.push_str("\n\n");
    }

    fn push_outline_heading(&mut self, title: &str, compact_before: bool) {
        self.compact_outline_gap(compact_before);
        self.markdown.push_str("## ");
        self.markdown.push_str(title);
        self.markdown.push('\n');
    }

    fn push_block(&mut self, block: &str) {
        let block = block.trim();
        if !block.is_empty() {
            self.markdown.push_str(block);
            self.markdown.push_str("\n\n");
        }
    }

    fn push_indented_block(&mut self, block: &str, indent: &str) {
        let block = block.trim();
        if !block.is_empty() {
            self.markdown.push_str(indent);
            self.markdown.push_str(block);
            self.markdown.push_str("\n\n");
        }
    }

    fn push_plain_block(&mut self, block: &str, indent: bool, compact_before: bool) {
        if let Some(block) = format_multiline_equations(block) {
            self.flush_pending_math();
            self.prepare_outline_block(&block, compact_before);
            if indent {
                self.push_indented_block(&block, TAB_INDENT);
            } else {
                self.push_block(&block);
            }
            return;
        }

        let Some(latex) = standalone_math_latex(block) else {
            self.flush_pending_math();
            self.prepare_outline_block(block, compact_before);
            if indent {
                self.push_indented_block(block, TAB_INDENT);
            } else {
                self.push_block(block);
            }
            return;
        };

        if !self.pending_math_rows.is_empty() && !starts_with_math_relation(latex) {
            self.flush_pending_math();
        }
        if self.pending_math_rows.is_empty() {
            self.pending_math_compact_before = compact_before;
        }
        self.pending_math_rows.push(latex.to_owned());
    }

    fn flush_pending_math(&mut self) {
        let rows = std::mem::take(&mut self.pending_math_rows);
        let compact_before = std::mem::take(&mut self.pending_math_compact_before);
        match rows.as_slice() {
            [] => {}
            _ => {
                self.compact_outline_gap(compact_before);
                self.push_block(&format!(
                    "$\\displaystyle\\qquad{}$",
                    format_equation_array(&rows)
                ));
            }
        }
    }

    fn prepare_outline_block(&mut self, block: &str, compact_before: bool) {
        if !block.trim().is_empty() {
            // A Markdown table must start a new block. Turning the preceding
            // blank line into a hard line break makes Obsidian display the
            // generated pipes literally, especially when table cells contain
            // equations.
            self.compact_outline_gap(compact_before && !starts_with_markdown_table(block));
        }
    }

    fn prepare_reference_block(&mut self, block: &str) {
        if !block.trim().is_empty() && self.markdown.ends_with("\n\n") {
            self.markdown.pop();
        }
    }

    fn compact_outline_gap(&mut self, compact_before: bool) {
        if compact_before && self.markdown.ends_with("\n\n") {
            self.markdown.truncate(self.markdown.len() - 2);
            self.markdown.push_str("  \n");
        }
    }

    fn push_list_block(&mut self, block: &str, depth: usize, marker: &str) {
        let block = block.trim();
        if block.is_empty() {
            return;
        }
        let indent = "  ".repeat(depth);
        let continuation = " ".repeat(marker.chars().count());
        for (index, line) in block.lines().enumerate() {
            self.markdown.push_str(&indent);
            if index == 0 {
                self.markdown.push_str(marker);
            } else {
                self.markdown.push_str(&continuation);
            }
            self.markdown.push_str(line);
            self.markdown.push('\n');
        }
        self.markdown.push('\n');
    }
}

fn starts_with_markdown_table(block: &str) -> bool {
    let mut lines = block.trim_start().lines();
    matches!(
        (lines.next(), lines.next()),
        (Some(first), Some(separator))
            if first.trim_end().ends_with('|')
                && first.trim_start().starts_with('|')
                && separator.trim_start().starts_with("| --- |")
    )
}

fn outline_section_heading(element: &OutlineElement) -> Option<&'static str> {
    let [Content::RichText(text)] = element.contents() else {
        return None;
    };
    let title = text.text().trim();
    if title.eq_ignore_ascii_case("references") {
        Some("References")
    } else if title.eq_ignore_ascii_case("see also") {
        Some("See Also")
    } else {
        None
    }
}

fn task_state(element: &OutlineElement) -> Option<bool> {
    element.contents().iter().find_map(|content| match content {
        Content::RichText(text) => text
            .note_tags()
            .iter()
            .find(|tag| {
                tag.item_status().task_tag()
                    || tag.definition().is_some_and(|definition| {
                        format!("{:?}", definition.shape()).contains("CheckBox")
                    })
            })
            .map(|tag| tag.item_status().completed()),
        _ => None,
    })
}

#[derive(Clone, Copy, Default)]
struct RunStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    superscript: bool,
    subscript: bool,
    colored: bool,
    hidden: bool,
}

impl From<&ParagraphStyling> for RunStyle {
    fn from(style: &ParagraphStyling) -> Self {
        Self {
            bold: style.bold(),
            italic: style.italic(),
            underline: style.underline(),
            strikethrough: style.strikethrough(),
            superscript: style.superscript(),
            subscript: style.subscript(),
            colored: matches!(
                style.font_color(),
                Some(ColorRef::Manual { r, g, b }) if (r, g, b) != (0, 0, 0)
            ),
            hidden: style.hidden(),
        }
    }
}

fn render_rich_text(text: &RichText, in_table: bool, linkify_bare_urls: bool) -> String {
    let units: Vec<u16> = text.text().encode_utf16().collect();
    if units.is_empty() {
        return String::new();
    }

    let end = units.len() as u32;
    let hyperlinks = text.hyperlinks();
    let mut boundaries = vec![0, end];
    boundaries.extend(
        text.text_run_indices()
            .iter()
            .copied()
            .map(|value| value.min(end)),
    );
    for link in &hyperlinks {
        boundaries.push(link.start().min(end));
        boundaries.push(link.end().min(end));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let run_ends = text.text_run_indices();
    let run_styles = text.text_run_formatting();
    let mut math_objects = text.math_inline_objects().iter().copied();
    let math_objects_by_run: Vec<Option<MathDescriptor>> = run_styles
        .iter()
        .map(|style| {
            style
                .math_formatting()
                .then(|| math_objects.next().map(MathDescriptor::from))
                .flatten()
        })
        .collect();
    let mut output = String::new();
    let mut math = Vec::new();

    for pair in boundaries.windows(2) {
        let start = pair[0];
        let stop = pair[1];
        if stop <= start {
            continue;
        }

        let run_index = run_ends
            .iter()
            .position(|run_end| start < *run_end)
            .or_else(|| (!run_styles.is_empty()).then_some(run_styles.len() - 1));
        let paragraph_style = text.paragraph_style();
        let style_data = run_index
            .and_then(|index| run_styles.get(index))
            .unwrap_or(paragraph_style);
        let style = RunStyle::from(style_data);
        if style.hidden {
            flush_math(&mut output, &mut math);
            continue;
        }

        let raw = String::from_utf16_lossy(&units[start as usize..stop as usize]);
        if style_data.math_formatting() {
            let run_start = run_index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| run_ends.get(index))
                .copied()
                .unwrap_or(0);
            let object = if start == run_start {
                run_index
                    .and_then(|index| math_objects_by_run.get(index))
                    .copied()
                    .flatten()
            } else {
                None
            };
            for (index, ch) in raw.chars().enumerate() {
                math.push(MathAtom {
                    ch,
                    object: (index == 0).then_some(object).flatten(),
                });
            }
            continue;
        }

        flush_math(&mut output, &mut math);
        let raw = remove_control_markers(&raw);
        let linked = hyperlinks
            .iter()
            .find(|link| start >= link.start() && stop <= link.end())
            .map_or_else(
                || render_text_run(&raw, style, in_table, linkify_bare_urls),
                |link| {
                    let escaped = escape_markdown(&raw, in_table);
                    let styled = apply_style(&escaped, style);
                    if is_onenote_link(link.target()) {
                        styled
                    } else {
                        format!("[{styled}](<{}>)", escape_link_target(link.target()))
                    }
                },
            );
        output.push_str(&linked);
    }

    flush_math(&mut output, &mut math);
    output
}

fn render_text_run(raw: &str, style: RunStyle, in_table: bool, linkify: bool) -> String {
    if !linkify {
        return apply_style(&escape_markdown(raw, in_table), style);
    }

    let mut output = String::new();
    let mut remaining = raw;
    while let Some(start) = next_web_url_start(remaining) {
        let (before, url_and_after) = remaining.split_at(start);
        output.push_str(&apply_style(&escape_markdown(before, in_table), style));

        let whitespace = url_and_after
            .find(char::is_whitespace)
            .unwrap_or(url_and_after.len());
        let candidate = &url_and_after[..whitespace];
        let url_len = bare_url_len(candidate);
        let (url, after_url) = url_and_after.split_at(url_len);
        let label = apply_style(&escape_markdown(url, in_table), style);
        output.push_str(&format!("[{label}](<{}>)", escape_link_target(url)));
        remaining = after_url;
    }
    output.push_str(&apply_style(&escape_markdown(remaining, in_table), style));
    output
}

fn next_web_url_start(value: &str) -> Option<usize> {
    ["https://", "http://"]
        .iter()
        .filter_map(|prefix| value.find(prefix))
        .min()
}

fn is_onenote_link(value: &str) -> bool {
    value
        .trim_start()
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("onenote:"))
}

fn bare_url_len(value: &str) -> usize {
    let mut end = value.len();
    loop {
        let candidate = &value[..end];
        let Some((index, last)) = candidate.char_indices().next_back() else {
            return 0;
        };
        let trim = matches!(last, '.' | ',' | ';' | ':' | '!' | '?')
            || (last == ')' && candidate.matches(')').count() > candidate.matches('(').count())
            || (last == ']' && candidate.matches(']').count() > candidate.matches('[').count())
            || (last == '}' && candidate.matches('}').count() > candidate.matches('{').count());
        if !trim {
            return end;
        }
        end = index;
    }
}

#[derive(Clone, Copy)]
struct MathDescriptor {
    object_type: MathObjectType,
    column: Option<u8>,
    ch: Option<char>,
    ch1: Option<char>,
    ch2: Option<char>,
}

impl From<MathInlineObject> for MathDescriptor {
    fn from(value: MathInlineObject) -> Self {
        Self {
            object_type: value.object_type(),
            column: value.column(),
            ch: value.char(),
            ch1: value.char1(),
            ch2: value.char2(),
        }
    }
}

#[derive(Clone, Copy)]
struct MathAtom {
    ch: char,
    object: Option<MathDescriptor>,
}

fn flush_math(output: &mut String, atoms: &mut Vec<MathAtom>) {
    if atoms.is_empty() {
        return;
    }

    let mut parser = MathParser::new(atoms);
    let latex = parser.parse_sequence();
    let leading_space = atoms
        .iter()
        .take_while(|atom| atom.ch.is_whitespace())
        .map(|atom| atom.ch)
        .collect::<String>();
    let trailing_space = atoms
        .iter()
        .rev()
        .take_while(|atom| atom.ch.is_whitespace())
        .map(|atom| atom.ch)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let latex = latex.trim();

    output.push_str(&leading_space);
    if !latex.is_empty() {
        output.push('$');
        output.push_str(latex);
        output.push('$');
    }
    output.push_str(&trailing_space);
    atoms.clear();
}

struct MathParser<'a> {
    atoms: &'a [MathAtom],
    position: usize,
}

impl<'a> MathParser<'a> {
    fn new(atoms: &'a [MathAtom]) -> Self {
        Self { atoms, position: 0 }
    }

    fn parse_sequence(&mut self) -> String {
        let mut output = String::new();
        while let Some(atom) = self.atoms.get(self.position).copied() {
            match atom.ch {
                '\u{fdd0}' => output.push_str(&self.parse_object(atom.object)),
                '\u{fdee}' | '\u{fdef}' => break,
                '\0' => self.position += 1,
                ch if ch.is_ascii_alphabetic() => output.push_str(&self.parse_ascii_word()),
                ch => {
                    self.position += 1;
                    output.push_str(&latex_math_char(ch));
                }
            }
        }
        output
    }

    fn parse_ascii_word(&mut self) -> String {
        let mut word = String::new();
        while let Some(atom) = self.atoms.get(self.position) {
            if atom.ch.is_ascii_alphabetic() {
                word.push(atom.ch);
                self.position += 1;
            } else {
                break;
            }
        }

        if word.len() == 1 {
            word
        } else {
            match word.as_str() {
                "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "sinh" | "cosh" | "tanh"
                | "log" | "ln" | "exp" | "lim" | "min" | "max" | "det" | "gcd" => {
                    format!("\\{word} ")
                }
                _ => format!("\\operatorname{{{word}}}"),
            }
        }
    }

    fn parse_object(&mut self, descriptor: Option<MathDescriptor>) -> String {
        self.position += 1;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.parse_sequence());
            match self.atoms.get(self.position).map(|atom| atom.ch) {
                Some('\u{fdee}') => self.position += 1,
                Some('\u{fdef}') => {
                    self.position += 1;
                    break;
                }
                _ => break,
            }
        }

        descriptor.map_or_else(
            || arguments.concat(),
            |descriptor| format_math_object(descriptor, &arguments),
        )
    }
}

fn format_math_object(descriptor: MathDescriptor, arguments: &[String]) -> String {
    let argument = |index: usize| arguments.get(index).map(String::as_str).unwrap_or("");
    let braced = |value: &str| format!("{{{value}}}");
    match descriptor.object_type {
        MathObjectType::SimpleText | MathObjectType::PlainText | MathObjectType::Box => {
            arguments.concat()
        }
        MathObjectType::Accent => {
            let command = match descriptor.ch {
                Some('\u{0302}') => "hat",
                Some('\u{0303}') => "tilde",
                Some('\u{0304}') => "bar",
                Some('\u{0305}') => "overline",
                Some('\u{0307}') => "dot",
                Some('\u{0308}') => "ddot",
                Some('\u{20d7}') => "vec",
                _ => "widehat",
            };
            format!("\\{command}{{{}}}", argument(0))
        }
        MathObjectType::BoxedFormula => format!("\\boxed{{{}}}", argument(0)),
        MathObjectType::Brackets => format_delimited(descriptor, arguments, false),
        MathObjectType::BracketsWithSeps => format_delimited(descriptor, arguments, true),
        MathObjectType::EquationArray => format_equation_array(arguments),
        MathObjectType::Fraction => {
            format!("\\frac{{{}}}{{{}}}", argument(0), argument(1))
        }
        MathObjectType::FunctionApply => {
            let function = argument(0).trim();
            format!("{function} {}", argument(1))
        }
        MathObjectType::LeftSubSup => format!(
            "{{}}_{{{}}}^{{{}}}{}",
            argument(1),
            argument(2),
            braced(argument(0))
        ),
        MathObjectType::LowerLimit => format!("{}_{{{}}}", braced(argument(0)), argument(1)),
        MathObjectType::Matrix => format_matrix(descriptor.column, arguments),
        MathObjectType::Nary => format_nary(descriptor.ch, arguments),
        MathObjectType::OpChar => descriptor.ch.map_or_else(String::new, latex_math_char),
        MathObjectType::Overbar => format!("\\overline{{{}}}", argument(0)),
        MathObjectType::Phantom => format!("\\phantom{{{}}}", argument(0)),
        MathObjectType::Radical => {
            if argument(0).trim().is_empty() {
                format!("\\sqrt{{{}}}", argument(1))
            } else {
                format!("\\sqrt[{}]{{{}}}", argument(0), argument(1))
            }
        }
        MathObjectType::SlashedFraction => {
            format!("{{{}}}/{{{}}}", argument(0), argument(1))
        }
        MathObjectType::Stack => format!("\\substack{{{} \\\\ {}}}", argument(0), argument(1)),
        MathObjectType::StretchStack => match descriptor.ch {
            Some('⏟') => format!("\\underbrace{{{}}}", argument(0)),
            Some('⏞') => format!("\\overbrace{{{}}}", argument(0)),
            _ => argument(0).to_owned(),
        },
        MathObjectType::Subscript => {
            format!("{}_{{{}}}", braced(argument(0)), argument(1))
        }
        MathObjectType::SubSup => format!(
            "{}_{{{}}}^{{{}}}",
            braced(argument(0)),
            argument(1),
            argument(2)
        ),
        MathObjectType::Superscript => {
            format!("{}^{{{}}}", braced(argument(0)), argument(1))
        }
        MathObjectType::Underbar => format!("\\underline{{{}}}", argument(0)),
        MathObjectType::UpperLimit => format!("{}^{{{}}}", braced(argument(0)), argument(1)),
    }
}

fn format_delimited(
    descriptor: MathDescriptor,
    arguments: &[String],
    with_separators: bool,
) -> String {
    let left = latex_delimiter(descriptor.ch, true);
    let right = latex_delimiter(descriptor.ch1, false);
    let separator = latex_delimiter(descriptor.ch2, true);
    let contents = if with_separators {
        arguments.join(&format!(" \\middle{separator} "))
    } else {
        arguments.concat()
    };
    format!("\\left{left} {contents} \\right{right} ")
}

fn latex_delimiter(value: Option<char>, left: bool) -> &'static str {
    match value {
        Some('(') => "(",
        Some(')') => ")",
        Some('[') => "[",
        Some(']') => "]",
        Some('{') => "\\{",
        Some('}') => "\\}",
        Some('|') if left => "\\lvert",
        Some('|') => "\\rvert",
        Some('‖') if left => "\\lVert",
        Some('‖') => "\\rVert",
        Some('⌊') => "\\lfloor",
        Some('⌋') => "\\rfloor",
        Some('⌈') => "\\lceil",
        Some('⌉') => "\\rceil",
        Some('⟨') => "\\langle",
        Some('⟩') => "\\rangle",
        None => ".",
        _ => ".",
    }
}

fn format_equation_array(arguments: &[String]) -> String {
    let rows = arguments
        .iter()
        .map(|value| align_equation_row(&value.replace("\\&", "&")))
        .collect::<Vec<_>>()
        .join(" \\\\ ");
    format!("\\begin{{aligned}}{rows}\\end{{aligned}}")
}

fn align_equation_row(row: &str) -> String {
    if row.contains('&') {
        return row.to_owned();
    }
    let Some(relation) = first_math_relation(row) else {
        return row.to_owned();
    };
    let mut output = String::with_capacity(row.len() + 1);
    output.push_str(&row[..relation]);
    output.push('&');
    output.push_str(&row[relation..]);
    output
}

fn standalone_math_latex(block: &str) -> Option<&str> {
    let block = block.trim();
    let latex = block.strip_prefix('$')?.strip_suffix('$')?;
    (!latex.contains('$')).then_some(latex.trim())
}

fn format_multiline_equations(block: &str) -> Option<String> {
    if !block.contains('\n') {
        return None;
    }

    let lines = block.lines().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut found_equation = false;
    let mut index = 0;

    while index < lines.len() {
        if standalone_math_latex(lines[index]).is_none() {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        found_equation = true;
        let mut rows = Vec::new();
        let mut trailing_space = "";
        while index < lines.len() {
            let Some(latex) = standalone_math_latex(lines[index]) else {
                break;
            };
            rows.push(latex.to_owned());
            trailing_space = &lines[index][lines[index].trim_end().len()..];
            index += 1;
        }
        output.push(format!(
            "$\\displaystyle\\qquad{}${trailing_space}",
            format_equation_array(&rows)
        ));
    }

    found_equation.then(|| output.join("\n"))
}

fn starts_with_math_relation(latex: &str) -> bool {
    first_math_relation(latex.trim_start()) == Some(0)
}

fn first_math_relation(latex: &str) -> Option<usize> {
    let commands = ["\\propto", "\\approx", "\\ne", "\\le", "\\ge", "\\sim"];
    let mut group_depth = 0_u32;
    let mut delimiter_depth = 0_u32;

    for (index, ch) in latex.char_indices() {
        if latex_command_at(latex, index, "\\left") {
            delimiter_depth += 1;
            continue;
        }
        if latex_command_at(latex, index, "\\right") {
            delimiter_depth = delimiter_depth.saturating_sub(1);
            continue;
        }
        match ch {
            '{' if !is_latex_escaped(latex, index) => {
                group_depth += 1;
                continue;
            }
            '}' if !is_latex_escaped(latex, index) => {
                group_depth = group_depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if group_depth != 0 || delimiter_depth != 0 {
            continue;
        }
        if ch == '=' {
            return Some(index);
        }
        for command in commands {
            if latex_command_at(latex, index, command) {
                return Some(index);
            }
        }
    }
    None
}

fn latex_command_at(latex: &str, index: usize, command: &str) -> bool {
    latex[index..].starts_with(command)
        && latex[index + command.len()..]
            .chars()
            .next()
            .is_none_or(|next| !next.is_ascii_alphabetic())
}

fn is_latex_escaped(latex: &str, index: usize) -> bool {
    latex[..index]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

fn format_matrix(columns: Option<u8>, arguments: &[String]) -> String {
    let columns = usize::from(columns.unwrap_or(1)).max(1);
    let rows = arguments
        .chunks(columns)
        .map(|row| row.join(" & "))
        .collect::<Vec<_>>()
        .join(" \\\\ ");
    format!("\\begin{{matrix}}{rows}\\end{{matrix}}")
}

fn format_nary(operator: Option<char>, arguments: &[String]) -> String {
    let command = match operator {
        Some('∑') => "\\sum",
        Some('∏') => "\\prod",
        Some('∫') => "\\int",
        Some('∬') => "\\iint",
        Some('∭') => "\\iiint",
        Some('⋃') => "\\bigcup",
        Some('⋂') => "\\bigcap",
        Some(ch) => return format!("{}{}", latex_math_char(ch), arguments.concat()),
        None => "\\sum",
    };
    let lower = arguments.first().map(String::as_str).unwrap_or("");
    let upper = arguments.get(1).map(String::as_str).unwrap_or("");
    let body = arguments.get(2).map(String::as_str).unwrap_or("");
    let mut output = command.to_owned();
    if !lower.trim().is_empty() {
        output.push_str(&format!("_{{{lower}}}"));
    }
    if !upper.trim().is_empty() {
        output.push_str(&format!("^{{{upper}}}"));
    }
    if !body.is_empty() {
        output.push(' ');
        output.push_str(body);
    }
    output
}

fn latex_math_char(ch: char) -> String {
    if let Some(value) = styled_math_alphanumeric(ch) {
        return value;
    }
    if let Some(command) = greek_command(ch) {
        return format!("\\{command} ");
    }

    match ch {
        '−' => "-".to_owned(),
        '±' => "\\pm ".to_owned(),
        '∓' => "\\mp ".to_owned(),
        '×' => "\\times ".to_owned(),
        '÷' => "\\div ".to_owned(),
        '⋅' | '·' => "\\cdot ".to_owned(),
        '∑' => "\\sum ".to_owned(),
        '∏' => "\\prod ".to_owned(),
        '∫' => "\\int ".to_owned(),
        '∞' => "\\infty ".to_owned(),
        '∂' => "\\partial ".to_owned(),
        '∇' => "\\nabla ".to_owned(),
        '∀' => "\\forall ".to_owned(),
        '∃' => "\\exists ".to_owned(),
        '∈' => "\\in ".to_owned(),
        '∉' => "\\notin ".to_owned(),
        '∅' => "\\varnothing ".to_owned(),
        '∝' => "\\propto ".to_owned(),
        '≈' => "\\approx ".to_owned(),
        '≠' => "\\ne ".to_owned(),
        '≤' => "\\le ".to_owned(),
        '≥' => "\\ge ".to_owned(),
        '→' => "\\to ".to_owned(),
        '←' => "\\leftarrow ".to_owned(),
        '⇒' => "\\Rightarrow ".to_owned(),
        '⇔' => "\\Leftrightarrow ".to_owned(),
        '↦' => "\\mapsto ".to_owned(),
        '∧' => "\\land ".to_owned(),
        '∨' => "\\lor ".to_owned(),
        '¬' => "\\neg ".to_owned(),
        '⊂' => "\\subset ".to_owned(),
        '⊆' => "\\subseteq ".to_owned(),
        '⊃' => "\\supset ".to_owned(),
        '⊇' => "\\supseteq ".to_owned(),
        '⊥' => "\\perp ".to_owned(),
        '′' => "\\prime ".to_owned(),
        '…' => "\\ldots ".to_owned(),
        '⋯' => "\\cdots ".to_owned(),
        '|' => "\\mid ".to_owned(),
        ' ' | ' ' => "\\quad ".to_owned(),
        ' ' | ' ' => "\\,".to_owned(),
        '\u{00a0}' => "~".to_owned(),
        '\\' => "\\backslash ".to_owned(),
        '{' | '}' | '#' | '$' | '%' | '&' | '_' => format!("\\{ch}"),
        '^' => "\\hat{}".to_owned(),
        '~' => "\\sim ".to_owned(),
        '\r' | '\n' | '\u{000b}' | '\u{000c}' | '\t' => " ".to_owned(),
        '\u{200b}' | '\u{2060}' | '\u{feff}' => String::new(),
        ch if ch.is_control() => String::new(),
        _ => ch.to_string(),
    }
}

fn styled_math_alphanumeric(ch: char) -> Option<String> {
    let code = ch as u32;
    let ranges = [
        (0x1d400, 26, 'A', "mathbf"),
        (0x1d41a, 26, 'a', "mathbf"),
        (0x1d434, 26, 'A', ""),
        (0x1d44e, 26, 'a', ""),
        (0x1d468, 26, 'A', "boldsymbol"),
        (0x1d482, 26, 'a', "boldsymbol"),
        (0x1d49c, 26, 'A', "mathcal"),
        (0x1d4b6, 26, 'a', "mathcal"),
        (0x1d4d0, 26, 'A', "boldsymbol"),
        (0x1d4ea, 26, 'a', "boldsymbol"),
        (0x1d504, 26, 'A', "mathfrak"),
        (0x1d51e, 26, 'a', "mathfrak"),
        (0x1d538, 26, 'A', "mathbb"),
        (0x1d552, 26, 'a', "mathbb"),
        (0x1d56c, 26, 'A', "mathbf"),
        (0x1d586, 26, 'a', "mathbf"),
        (0x1d5a0, 26, 'A', "mathsf"),
        (0x1d5ba, 26, 'a', "mathsf"),
        (0x1d5d4, 26, 'A', "mathbf"),
        (0x1d5ee, 26, 'a', "mathbf"),
        (0x1d608, 26, 'A', "mathsf"),
        (0x1d622, 26, 'a', "mathsf"),
        (0x1d63c, 26, 'A', "boldsymbol"),
        (0x1d656, 26, 'a', "boldsymbol"),
        (0x1d670, 26, 'A', "mathtt"),
        (0x1d68a, 26, 'a', "mathtt"),
        (0x1d7ce, 10, '0', "mathbf"),
        (0x1d7d8, 10, '0', "mathbb"),
        (0x1d7e2, 10, '0', "mathsf"),
        (0x1d7ec, 10, '0', "mathbf"),
        (0x1d7f6, 10, '0', "mathtt"),
    ];
    for (start, count, base, command) in ranges {
        if (start..start + count).contains(&code) {
            let value = char::from_u32(base as u32 + code - start)?;
            return Some(if command.is_empty() {
                value.to_string()
            } else {
                format!("\\{command}{{{value}}}")
            });
        }
    }

    if let Some(value) = legacy_styled_letter(ch) {
        return Some(value.to_owned());
    }
    styled_greek(code)
}

fn legacy_styled_letter(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ℎ' => "h",
        'ℬ' => "\\mathcal{B}",
        'ℰ' => "\\mathcal{E}",
        'ℱ' => "\\mathcal{F}",
        'ℋ' => "\\mathcal{H}",
        'ℐ' => "\\mathcal{I}",
        'ℒ' => "\\mathcal{L}",
        'ℳ' => "\\mathcal{M}",
        'ℛ' => "\\mathcal{R}",
        'ℭ' => "\\mathfrak{C}",
        'ℌ' => "\\mathfrak{H}",
        'ℑ' => "\\mathfrak{I}",
        'ℜ' => "\\mathfrak{R}",
        'ℨ' => "\\mathfrak{Z}",
        'ℂ' => "\\mathbb{C}",
        'ℍ' => "\\mathbb{H}",
        'ℕ' => "\\mathbb{N}",
        'ℙ' => "\\mathbb{P}",
        'ℚ' => "\\mathbb{Q}",
        'ℝ' => "\\mathbb{R}",
        'ℤ' => "\\mathbb{Z}",
        _ => return None,
    })
}

fn styled_greek(code: u32) -> Option<String> {
    let (offset, bold) = if (0x1d6a8..=0x1d6e1).contains(&code) {
        (code - 0x1d6a8, true)
    } else if (0x1d6e2..=0x1d71b).contains(&code) {
        (code - 0x1d6e2, false)
    } else if (0x1d71c..=0x1d755).contains(&code) {
        (code - 0x1d71c, true)
    } else if (0x1d756..=0x1d78f).contains(&code) {
        (code - 0x1d756, true)
    } else if (0x1d790..=0x1d7c9).contains(&code) {
        (code - 0x1d790, true)
    } else {
        return None;
    };
    let names = [
        "A",
        "B",
        "Gamma",
        "Delta",
        "E",
        "Z",
        "H",
        "Theta",
        "I",
        "K",
        "Lambda",
        "M",
        "N",
        "Xi",
        "O",
        "Pi",
        "P",
        "Theta",
        "Sigma",
        "T",
        "Upsilon",
        "Phi",
        "X",
        "Psi",
        "Omega",
        "nabla",
        "alpha",
        "beta",
        "gamma",
        "delta",
        "epsilon",
        "zeta",
        "eta",
        "theta",
        "iota",
        "kappa",
        "lambda",
        "mu",
        "nu",
        "xi",
        "o",
        "pi",
        "rho",
        "varsigma",
        "sigma",
        "tau",
        "upsilon",
        "phi",
        "chi",
        "psi",
        "omega",
        "partial",
        "varepsilon",
        "vartheta",
        "varkappa",
        "varphi",
        "varrho",
        "varpi",
    ];
    let name = names.get(offset as usize)?;
    let value = if name.len() == 1 {
        (*name).to_owned()
    } else {
        format!("\\{name}")
    };
    Some(if bold {
        format!("\\boldsymbol{{{value}}}")
    } else {
        format!("{value} ")
    })
}

fn greek_command(ch: char) -> Option<&'static str> {
    Some(match ch {
        'Γ' => "Gamma",
        'Δ' => "Delta",
        'Θ' => "Theta",
        'Λ' => "Lambda",
        'Ξ' => "Xi",
        'Π' => "Pi",
        'Σ' => "Sigma",
        'Υ' => "Upsilon",
        'Φ' => "Phi",
        'Ψ' => "Psi",
        'Ω' => "Omega",
        'α' => "alpha",
        'β' => "beta",
        'γ' => "gamma",
        'δ' => "delta",
        'ε' => "epsilon",
        'ζ' => "zeta",
        'η' => "eta",
        'θ' => "theta",
        'ι' => "iota",
        'κ' => "kappa",
        'λ' => "lambda",
        'μ' => "mu",
        'ν' => "nu",
        'ξ' => "xi",
        'π' => "pi",
        'ρ' => "rho",
        'ς' => "varsigma",
        'σ' => "sigma",
        'τ' => "tau",
        'υ' => "upsilon",
        'φ' => "phi",
        'χ' => "chi",
        'ψ' => "psi",
        'ω' => "omega",
        'ϵ' => "varepsilon",
        'ϑ' => "vartheta",
        'ϕ' => "varphi",
        'ϱ' => "varrho",
        'ϖ' => "varpi",
        _ => return None,
    })
}

fn remove_control_markers(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '\0' && !('\u{fdd0}'..='\u{fdef}').contains(ch))
        .collect()
}

fn apply_style(value: &str, style: RunStyle) -> String {
    let Some(first) = value.find(|ch: char| !ch.is_whitespace()) else {
        return value.to_owned();
    };
    let last = value
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(value.len());
    let leading = &value[..first];
    let trailing = &value[last..];
    let mut core = value[first..last].to_owned();

    if style.underline {
        core = format!("<u>{core}</u>");
    }
    if style.subscript {
        core = format!("<sub>{core}</sub>");
    }
    if style.superscript {
        core = format!("<sup>{core}</sup>");
    }
    if style.strikethrough {
        core = format!("~~{core}~~");
    }
    if style.colored {
        core = format!("<em>{core}</em>");
    } else if style.italic {
        core = format!("*{core}*");
    }
    if style.bold {
        core = format!("**{core}**");
    }

    format!("{leading}{core}{trailing}")
}

fn escape_markdown(value: &str, in_table: bool) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '*' | '_' | '[' | ']' | '<' | '>' => {
                output.push('\\');
                output.push(ch);
            }
            '|' if in_table => output.push_str("\\|"),
            '\r' => {}
            '\n' | '\u{000b}' | '\u{000c}' if in_table => output.push_str("<br>"),
            '\n' | '\u{000b}' | '\u{000c}' => output.push_str("  \n"),
            '\t' => output.push(' '),
            ch if ch.is_control() => {}
            _ => output.push(ch),
        }
    }
    output
}

fn escape_link_target(value: &str) -> String {
    value
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('\\', "%5C")
        .replace(' ', "%20")
        .replace(['\r', '\n'], "")
}

fn table_cell(value: String) -> String {
    let flattened = value.trim().replace("\n\n", "<br>").replace('\n', "<br>");
    escape_unescaped_pipes(&flattened)
}

fn markdown_table_row(cells: &[String]) -> String {
    let mut output = String::from("|");
    for cell in cells {
        output.push(' ');
        output.push_str(cell);
        output.push_str(" |");
    }
    output
}

fn escape_unescaped_pipes(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut escaped = false;
    for ch in value.chars() {
        if ch == '|' && !escaped {
            output.push('\\');
        }
        output.push(ch);
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
    }
    output
}

struct AssetWriter {
    directory: PathBuf,
    relative_directory: String,
    used: HashSet<String>,
    count: usize,
}

impl AssetWriter {
    fn new(directory: PathBuf, relative_directory: String) -> Self {
        Self {
            directory,
            relative_directory,
            used: HashSet::new(),
            count: 0,
        }
    }

    fn next_number(&self) -> usize {
        self.count + 1
    }

    fn write_reader(
        &mut self,
        preferred_name: &str,
        extension: Option<&str>,
        mut reader: Box<dyn io::Read>,
    ) -> io::Result<String> {
        fs::create_dir_all(&self.directory)?;
        let mut name = safe_filename(preferred_name);
        if let Some(extension) = extension
            && Path::new(&name).extension().is_none()
        {
            name.push('.');
            name.push_str(extension);
        }
        if name.is_empty() {
            name = format!("asset-{:04}.bin", self.next_number());
        }

        let mut prefix = Vec::with_capacity(12);
        reader.by_ref().take(12).read_to_end(&mut prefix)?;
        if Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bin"))
            && let Some(extension) = inferred_bin_extension(&prefix)
        {
            let mut corrected = PathBuf::from(&name);
            corrected.set_extension(extension);
            name = corrected.to_string_lossy().into_owned();
        }
        name = self.unique_name(name);

        let destination = self.directory.join(&name);
        let extension = Path::new(&name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let png_name = extension.eq_ignore_ascii_case("png");
        let jpeg_name =
            extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg");
        let mut output = fs::File::create(destination)?;
        let tiff_as_png = png_name && is_tiff_payload(&prefix);
        output.write_all(&prefix)?;
        io::copy(&mut reader, &mut output)?;
        drop(output);

        let destination = self.directory.join(&name);
        if tiff_as_png {
            convert_tiff_to_png(&destination)?;
        }
        if png_name && (tiff_as_png || is_png_payload(&prefix)) {
            optimize_png(&destination)?;
        } else if jpeg_name && is_jpeg_payload(&prefix) {
            optimize_jpeg(&destination)?;
        }
        self.count += 1;

        Ok(format!(
            "{}/{}",
            self.relative_directory,
            url_encode_path(&name)
        ))
    }

    fn unique_name(&mut self, name: String) -> String {
        let path = Path::new(&name);
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("asset");
        let extension = path.extension().and_then(|value| value.to_str());
        let mut candidate = name.clone();
        let mut suffix = 2;
        while self.used.contains(&candidate.to_lowercase()) {
            candidate = extension.map_or_else(
                || format!("{stem}-{suffix}"),
                |extension| format!("{stem}-{suffix}.{extension}"),
            );
            suffix += 1;
        }
        self.used.insert(candidate.to_lowercase());
        candidate
    }
}

fn is_tiff_payload(prefix: &[u8]) -> bool {
    prefix.starts_with(b"II*\0")
        || prefix.starts_with(b"MM\0*")
        || prefix.starts_with(b"II+\0")
        || prefix.starts_with(b"MM\0+")
}

fn is_png_payload(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\x89PNG")
}

fn is_jpeg_payload(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\xff\xd8\xff")
}

fn inferred_bin_extension(prefix: &[u8]) -> Option<&'static str> {
    if is_png_payload(prefix) {
        Some("png")
    } else if is_jpeg_payload(prefix) {
        Some("jpg")
    } else if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        Some("gif")
    } else if is_tiff_payload(prefix) {
        Some("tiff")
    } else {
        None
    }
}

fn convert_tiff_to_png(path: &Path) -> io::Result<()> {
    let converted = path.with_extension("converted.png");
    let mut first_frame = OsString::from(path.as_os_str());
    first_frame.push("[0]");
    let result = Command::new("magick")
        .arg(first_frame)
        .args(["-alpha", "on", "-depth", "8", "-define", "png:color-type=6"])
        .arg(&converted)
        .output();

    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let _ = fs::remove_file(&converted);
            return Err(io::Error::other(format!(
                "ImageMagick failed to convert TIFF asset '{}': {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "failed to run ImageMagick for TIFF asset '{}': {error}",
                    path.display()
                ),
            ));
        }
    }

    fs::rename(converted, path)
}

fn optimize_png(path: &Path) -> io::Result<()> {
    let optimized = path.with_extension("pngquant.png");
    let result = Command::new("pngquant")
        .args(["--force", "--strip", "--output"])
        .arg(&optimized)
        .arg("--")
        .arg(path)
        .output();

    match result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let _ = fs::remove_file(&optimized);
            return Err(io::Error::other(format!(
                "pngquant failed for '{}': {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("failed to run pngquant for '{}': {error}", path.display()),
            ));
        }
    }

    if fs::metadata(&optimized)?.len() < fs::metadata(path)?.len() {
        fs::rename(&optimized, path)?;
    } else {
        fs::remove_file(optimized)?;
    }
    Ok(())
}

fn optimize_jpeg(path: &Path) -> io::Result<()> {
    let output = Command::new("jpegoptim")
        .args(["--auto-mode", "--quiet"])
        .arg(path)
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to run jpegoptim for '{}': {error}", path.display()),
            )
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "jpegoptim failed for '{}': {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn safe_filename(value: &str) -> String {
    let leaf = value.rsplit(['/', '\\']).next().unwrap_or(value);
    let mut output = String::new();
    let mut previous_was_replacement = false;
    for ch in leaf.chars().take(120) {
        let allowed = ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_');
        if allowed {
            output.push(ch);
            previous_was_replacement = false;
        } else if !previous_was_replacement {
            output.push('_');
            previous_was_replacement = true;
        }
    }
    protect_reserved_name(output.trim_matches(['.', '_', ' ']).to_owned())
}

fn safe_title_component(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_replacement = false;
    for ch in value.chars().take(120) {
        let allowed = ch.is_alphanumeric() || matches!(ch, ' ' | '.' | '-' | '–' | '_');
        if allowed {
            output.push(ch);
            previous_was_replacement = false;
        } else if !previous_was_replacement {
            output.push('_');
            previous_was_replacement = true;
        }
    }
    protect_reserved_name(output.trim_matches(['.', '_', ' ']).to_owned())
}

fn protect_reserved_name(output: String) -> String {
    let base = output
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    if reserved {
        format!("_{output}")
    } else {
        output
    }
}

fn safe_extension(value: &str) -> String {
    let extension: String = value
        .trim_start_matches('.')
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(10)
        .collect();
    if extension.is_empty() {
        "bin".to_owned()
    } else {
        extension.to_ascii_lowercase()
    }
}

fn url_encode_path(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

#[cfg(test)]
mod tests;
