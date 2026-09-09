use super::*;

fn math_descriptor(object_type: MathObjectType, ch: Option<char>) -> MathDescriptor {
    MathDescriptor {
        object_type,
        column: None,
        ch,
        ch1: None,
        ch2: None,
    }
}

fn parse_math_fixture(raw: &str, descriptors: &[MathDescriptor]) -> String {
    let mut descriptors = descriptors.iter().copied();
    let atoms = raw
        .chars()
        .map(|ch| MathAtom {
            ch,
            object: (ch == '\u{fdd0}').then(|| descriptors.next()).flatten(),
        })
        .collect::<Vec<_>>();
    MathParser::new(&atoms).parse_sequence()
}

#[test]
fn markdown_escaping_preserves_text_and_table_cells() {
    assert_eq!(escape_markdown("a * b_[c]", false), "a \\* b\\_\\[c\\]");
    assert_eq!(escape_markdown("a|b\nc", true), "a\\|b<br>c");
    assert_eq!(
        escape_markdown("first  \nsecond", false),
        "first<br>\nsecond"
    );
}

#[test]
fn recognizes_common_fixed_width_fonts_without_matching_proportional_fonts() {
    for font in [
        "Consolas",
        "Courier New",
        "Menlo",
        "Cascadia Code",
        "Aptos Mono",
        "SFMono-Regular",
    ] {
        assert!(
            is_fixed_width_font(font),
            "expected {font} to be fixed-width"
        );
    }
    for font in ["Arial", "Calibri", "Cambria Math", "Monotype Corsiva"] {
        assert!(
            !is_fixed_width_font(font),
            "expected {font} to be proportional"
        );
    }
}

#[test]
fn fixed_width_runs_become_safe_markdown_code_spans() {
    let style = RunStyle {
        fixed_width: true,
        ..RunStyle::default()
    };
    assert_eq!(render_styled_text("a_b * c", style, false), "`a_b * c`");
    assert_eq!(
        render_styled_text("`quoted`", style, false),
        "`` `quoted` ``"
    );
    assert_eq!(
        render_styled_text("left|right", style, true),
        "`left|right`"
    );
}

#[test]
fn adjacent_fixed_width_runs_share_one_code_span() {
    let style = RunStyle {
        fixed_width: true,
        ..RunStyle::default()
    };
    let mut output = String::new();
    let mut pending = None;
    for run in ["awk", "\u{00a0}", "'{print", "\u{00a0}", "$2}"] {
        push_fixed_width(&mut output, &mut pending, run, style, false);
    }
    flush_fixed_width(&mut output, &mut pending, false);

    assert_eq!(output, "`awk\u{00a0}'{print\u{00a0}$2}`");
    assert!(!output.contains("``"));
}

#[test]
fn multiline_fixed_width_text_uses_a_collision_safe_code_fence() {
    assert_eq!(
        fenced_code_block("fn main() {\n\tprintln!(\"ok\");\n}"),
        "```\nfn main() {\n\tprintln!(\"ok\");\n}\n```"
    );
    assert_eq!(
        fenced_code_block("before\n```\nafter"),
        "````\nbefore\n```\nafter\n````"
    );
    assert_eq!(normalize_code_text("a\r\nb\u{000b}c"), "a\nb\nc");
}

#[test]
fn consecutive_fixed_width_rows_are_merged_into_one_code_block() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(
        directory.path().join("_assets"),
        "../_assets".to_owned(),
        true,
    );
    let page_links = HashMap::new();
    let mut renderer = Renderer::new(&mut assets, &page_links, Path::new("Page.md"));
    renderer.pending_code_rows = vec![
        PendingCodeRow {
            text: "def answer():".to_owned(),
            rendered: "`def answer():`".to_owned(),
        },
        PendingCodeRow {
            text: "    return 42".to_owned(),
            rendered: "`    return 42`".to_owned(),
        },
    ];

    renderer.flush_pending_code();

    assert_eq!(
        renderer.markdown,
        "```\ndef answer():\n    return 42\n```\n\n"
    );
}

#[test]
fn tiff_payloads_are_detected_and_converted_to_png() {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 255])));
    let mut tiff = Cursor::new(Vec::new());
    image.write_to(&mut tiff, ImageFormat::Tiff).unwrap();
    let tiff = tiff.into_inner();

    assert!(is_tiff_payload(&tiff[..4]));
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(directory.path().to_owned(), "_assets".to_owned(), true);
    assets
        .write_reader("mislabeled.png", Some("tiff"), Box::new(Cursor::new(tiff)))
        .unwrap();

    let png = fs::read(directory.path().join("mislabeled.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(png[25], 3, "pngquant should emit an indexed PNG");
    let decoded = image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (64, 64));
}

#[test]
fn png_and_jpeg_assets_are_optimized() {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 255])));
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(directory.path().to_owned(), "_assets".to_owned(), true);

    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let png = png.into_inner();
    let png_link = assets
        .write_reader("image.bin", Some("bin"), Box::new(Cursor::new(png.clone())))
        .unwrap();
    assert_eq!(png_link, "_assets/image.png");
    let optimized_png = fs::read(directory.path().join("image.png")).unwrap();
    assert_eq!(&optimized_png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(optimized_png[25], 3);
    assert!(optimized_png.len() < png.len());

    let mut jpeg = Cursor::new(Vec::new());
    image.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
    let mut jpeg = jpeg.into_inner();
    jpeg.extend_from_slice(&[0; 4096]);
    let jpeg_link = assets
        .write_reader(
            "photo.bin",
            Some("bin"),
            Box::new(Cursor::new(jpeg.clone())),
        )
        .unwrap();
    assert_eq!(jpeg_link, "_assets/photo.jpg");
    let optimized_jpeg = fs::read(directory.path().join("photo.jpg")).unwrap();
    assert!(optimized_jpeg.starts_with(b"\xff\xd8\xff"));
    assert!(optimized_jpeg.len() < jpeg.len());

    let gif_link = assets
        .write_reader(
            "animation.bin",
            Some("bin"),
            Box::new(Cursor::new(b"GIF89a test")),
        )
        .unwrap();
    assert_eq!(gif_link, "_assets/animation.gif");
    assert!(directory.path().join("animation.gif").is_file());

    let unknown_link = assets
        .write_reader(
            "unknown.bin",
            Some("bin"),
            Box::new(Cursor::new(b"unknown data")),
        )
        .unwrap();
    assert_eq!(unknown_link, "_assets/unknown.bin");
}

#[test]
fn image_optimization_can_be_disabled() {
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    assert!(ConversionOptions::default().optimize_images);
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 255])));
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(directory.path().to_owned(), "_assets".to_owned(), false);

    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let png = png.into_inner();
    assets
        .write_reader(
            "unoptimized.png",
            Some("png"),
            Box::new(Cursor::new(png.clone())),
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.path().join("unoptimized.png")).unwrap(),
        png
    );

    let mut jpeg = Cursor::new(Vec::new());
    image.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
    let mut jpeg = jpeg.into_inner();
    jpeg.extend_from_slice(&[0; 4096]);
    assets
        .write_reader(
            "unoptimized.jpg",
            Some("jpg"),
            Box::new(Cursor::new(jpeg.clone())),
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.path().join("unoptimized.jpg")).unwrap(),
        jpeg
    );
}

#[test]
fn markdown_tables_keep_the_required_leading_blank_line() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(
        directory.path().join("_assets"),
        "../_assets".to_owned(),
        true,
    );
    let page_links = HashMap::new();
    let mut renderer = Renderer::new(&mut assets, &page_links, Path::new("Page.md"));
    let table = "| Allocation | ${A}_{i}=1$ |\n| --- | --- |\n| Selection | ${S}_{i}=2$ |";

    renderer
        .markdown
        .push_str("The excess return is decomposed\n\n");
    renderer.prepare_outline_block(table, true);
    renderer.push_block(table);

    assert_eq!(
        renderer.markdown,
        "The excess return is decomposed\n\n| Allocation | ${A}_{i}=1$ |\n| --- | --- |\n| Selection | ${S}_{i}=2$ |\n\n"
    );
}

#[test]
fn adjacent_ordered_items_use_incrementing_markers_without_trailing_spaces() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(
        directory.path().join("_assets"),
        "../_assets".to_owned(),
        true,
    );
    let page_links = HashMap::new();
    let mut renderer = Renderer::new(&mut assets, &page_links, Path::new("Page.md"));

    let first = renderer.ordered_list_marker(0, None);
    renderer.push_list_block("First", 0, &first);
    renderer.prepare_outline_block("Second", true);
    let second = renderer.ordered_list_marker(0, None);
    renderer.push_list_block("Second", 0, &second);

    assert_eq!(renderer.markdown, "1. First\n2. Second\n\n");
    assert!(
        renderer
            .markdown
            .lines()
            .all(|line| line.trim_end() == line)
    );

    assert_eq!(renderer.ordered_list_marker(1, None), "1. ");
    assert_eq!(renderer.ordered_list_marker(1, None), "2. ");
    assert_eq!(renderer.ordered_list_marker(0, None), "3. ");
    assert_eq!(renderer.ordered_list_marker(1, None), "1. ");
    renderer.end_ordered_lists_from(0);
    assert_eq!(renderer.ordered_list_marker(0, None), "1. ");
    assert_eq!(renderer.ordered_list_marker(0, Some(7)), "7. ");
    assert_eq!(renderer.ordered_list_marker(0, None), "8. ");
}

#[test]
fn nested_list_equations_remain_inside_their_parent_item() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(
        directory.path().join("_assets"),
        "../_assets".to_owned(),
        true,
    );
    let page_links = HashMap::new();
    let mut renderer = Renderer::new(&mut assets, &page_links, Path::new("Page.md"));

    renderer.push_list_block("Generate", 0, "1. ");
    renderer.prepare_outline_block("Calculate", true);
    renderer.push_list_block("Calculate", 2, "- ");
    renderer.push_plain_block("$x=1$", 3, false, true);
    renderer.flush_pending_math();
    renderer.prepare_outline_block("Update", true);
    renderer.push_list_block("Update", 0, "2. ");

    assert_eq!(
        renderer.markdown,
        "1. Generate\n        - Calculate\n            <br>$\\displaystyle\\qquad\\begin{aligned}x&=1\\end{aligned}$\n2. Update\n\n"
    );
}

#[test]
fn an_ordinary_section_heading_ends_reference_list_mode() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(
        directory.path().join("_assets"),
        "../_assets".to_owned(),
        true,
    );
    let page_links = HashMap::new();
    let mut renderer = Renderer::new(&mut assets, &page_links, Path::new("Page.md"));

    renderer.update_reference_section(Some("References"), false);
    assert!(renderer.in_references);

    renderer.update_reference_section(None, true);
    assert!(!renderer.in_references);

    renderer.update_reference_section(Some("See Also"), false);
    assert!(!renderer.in_references);
}

#[test]
fn styling_does_not_put_whitespace_inside_delimiters() {
    let style = RunStyle {
        bold: true,
        italic: true,
        ..RunStyle::default()
    };
    assert_eq!(apply_style(" hello ", style), " ***hello*** ");
    assert_eq!(
        apply_style(
            "colored",
            RunStyle {
                colored: true,
                ..RunStyle::default()
            }
        ),
        "<em>colored</em>"
    );
    assert_eq!(
        apply_style(
            "colored italic",
            RunStyle {
                italic: true,
                colored: true,
                ..RunStyle::default()
            }
        ),
        "<em>colored italic</em>"
    );
}

#[test]
fn fully_bold_detection_checks_every_visible_text_run() {
    let bold = RunStyle {
        bold: true,
        ..RunStyle::default()
    };
    let hidden = RunStyle {
        hidden: true,
        ..RunStyle::default()
    };

    assert!(visible_segments_are_fully_bold([
        ("First", bold),
        (" ", RunStyle::default()),
        ("second", bold),
        ("hidden", hidden),
    ]));
    assert!(!visible_segments_are_fully_bold([
        ("First", bold),
        ("second", RunStyle::default()),
    ]));
    assert!(!visible_segments_are_fully_bold([(" ", bold)]));
}

#[test]
fn filenames_cannot_escape_the_asset_directory() {
    assert_eq!(safe_filename("../../CON.txt"), "_CON.txt");
    assert_eq!(safe_filename("folder\\my report?.pdf"), "my_report_.pdf");
    assert_eq!(safe_extension("../PNG"), "png");
    assert_eq!(markdown_filename("Page title"), "Page title.md");
    assert_eq!(
        markdown_filename("Invariant risk minimization (IRM)"),
        "Invariant risk minimization - IRM.md"
    );
    assert_eq!(
        markdown_filename("Laplace's approximation"),
        "Laplaces approximation.md"
    );
    assert_eq!(
        markdown_filename("Bear / bull spreads"),
        "Bear - bull spreads.md"
    );
    assert_eq!(markdown_filename("snake_case"), "snake-case.md");

    let mut used = HashSet::new();
    assert_eq!(
        unique_generated_name("Page.md".to_owned(), &mut used),
        "Page.md"
    );
    assert_eq!(
        unique_generated_name("Page.md".to_owned(), &mut used),
        "Page-2.md"
    );
}

#[test]
fn section_index_preserves_page_order_and_hierarchy() {
    let pages = vec![
        PageIndexEntry {
            title: "Parent".to_owned(),
            filename: "Parent.md".to_owned(),
            level: 1,
        },
        PageIndexEntry {
            title: "Child (C)".to_owned(),
            filename: "Child - C.md".to_owned(),
            level: 2,
        },
        PageIndexEntry {
            title: "Sibling".to_owned(),
            filename: "Sibling.md".to_owned(),
            level: 1,
        },
    ];

    assert_eq!(
        render_section_index("Section", &pages),
        "# Section\n\n- [Parent](<Parent.md>)\n  - [Child (C)](<Child%20-%20C.md>)\n- [Sibling](<Sibling.md>)\n"
    );
}

#[test]
fn section_names_are_unique_within_their_parent_folder() {
    let mut used = HashSet::new();
    assert_eq!(
        unique_section_name("General".to_owned(), Path::new("Notebook A"), &mut used),
        "General"
    );
    assert_eq!(
        unique_section_name("General".to_owned(), Path::new("Notebook B"), &mut used),
        "General"
    );
    assert_eq!(
        unique_section_name("General".to_owned(), Path::new("Notebook A"), &mut used),
        "General-2"
    );
}

#[test]
fn links_are_url_encoded() {
    assert_eq!(
        url_encode_path("My notes/image 1.png"),
        "My%20notes%2Fimage%201.png"
    );
    assert_eq!(
        escape_link_target("https://e.test/a>b"),
        "https://e.test/a%3Eb"
    );
    assert_eq!(escape_link_target("file:///a%20b\\c"), "file:///a%20b%5Cc");
    assert!(is_onenote_link("onenote:Section.one#Page"));
    assert!(is_onenote_link("ONENOTE:https://example.com"));
    assert!(!is_onenote_link("https://example.com"));
    assert_eq!(
        render_text_run(
            "See https://example.com/a_b_(c).",
            RunStyle::default(),
            false,
            true,
        ),
        "See [https://example.com/a\\_b\\_(c)](<https://example.com/a_b_(c)>)."
    );
}

#[test]
fn onenote_page_links_resolve_to_relative_markdown_paths() {
    assert_eq!(
        onenote_page_id("onenote:#Page&section-id={section}&page-id=%7BABC-123%7D&end"),
        Some("abc-123".to_owned())
    );
    let page_links = HashMap::from([
        (
            "abc-123".to_owned(),
            PathBuf::from("Notebook/Section/Law of total probability.md"),
        ),
        (
            "def-456".to_owned(),
            PathBuf::from("Notebook/Other/Target.md"),
        ),
    ]);
    assert_eq!(
        resolve_onenote_link(
            "onenote:#Page&page-id={ABC-123}",
            Path::new("Notebook/Section/Source.md"),
            &page_links,
        ),
        Some("Law%20of%20total%20probability.md".to_owned())
    );
    assert_eq!(
        resolve_onenote_link(
            "ONENOTE:#Target&PAGE-ID={DEF-456}",
            Path::new("Notebook/Section/Source.md"),
            &page_links,
        ),
        Some("../Other/Target.md".to_owned())
    );
    assert_eq!(
        resolve_onenote_link(
            "onenote:#Missing&page-id={missing}",
            Path::new("Notebook/Section/Source.md"),
            &page_links,
        ),
        None
    );
}

#[test]
fn input_cannot_be_used_as_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.one");
    fs::write(&path, b"not a real OneNote file").unwrap();
    let error = protect_input_from_overwrite(&path, &path).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn office_math_objects_become_structured_latex() {
    let fraction = math_descriptor(MathObjectType::Fraction, Some('/'));
    assert_eq!(
        parse_math_fixture("\u{fdd0}1\u{fdee}𝑍\u{fdef}", &[fraction]),
        "\\frac{1}{Z}"
    );

    let sum = math_descriptor(MathObjectType::Nary, Some('∑'));
    let scripts = math_descriptor(MathObjectType::SubSup, Some('_'));
    assert_eq!(
        parse_math_fixture(
            "\u{fdd0}𝑘=0\u{fdee}∞\u{fdee}\u{fdd0}𝑝\u{fdee}𝑖𝑘\u{fdee}𝑠\u{fdef}\u{fdef}",
            &[sum, scripts],
        ),
        "\\sum_{k=0}^{\\infty } {p}_{ik}^{s}"
    );

    let radical = math_descriptor(MathObjectType::Radical, Some('√'));
    assert_eq!(
        parse_math_fixture("\u{fdd0}\u{fdee}2𝜂\u{fdef}", &[radical]),
        "\\sqrt{2\\eta }"
    );

    let absolute_value = MathDescriptor {
        object_type: MathObjectType::Brackets,
        column: None,
        ch: Some('|'),
        ch1: Some('|'),
        ch2: None,
    };
    assert_eq!(
        parse_math_fixture("\u{fdd0}f\u{fdef}", &[absolute_value]),
        "\\left\\lvert f \\right\\rvert "
    );
    assert_eq!(
        format_equation_array(&["x=1".to_owned(), "long=2".to_owned()]),
        "\\begin{aligned}x&=1 \\\\ long&=2\\end{aligned}"
    );
    assert_eq!(
        format_equation_array(&["x&=1".to_owned(), "long&=2".to_owned()]),
        "\\begin{aligned}x&=1 \\\\ long&=2\\end{aligned}"
    );
    assert_eq!(
        format_equation_array(&[
            "q(z)\\propto p(z,x)=p(x\\mid z)p(z)".to_owned(),
            "\\propto \\exp(-z)".to_owned(),
            "=C".to_owned(),
        ]),
        "\\begin{aligned}q(z)&\\propto p(z,x)=p(x\\mid z)p(z) \\\\ &\\propto \\exp(-z) \\\\ &=C\\end{aligned}"
    );
    assert_eq!(
        align_equation_row("{\\nabla}_{w\\mid w=1.0}"),
        "{\\nabla}_{w\\mid w=1.0}"
    );
    assert_eq!(align_equation_row("f_{x=1}=2"), "f_{x=1}&=2");
    assert_eq!(
        align_equation_row("\\mathbb{E}\\left[ X\\mid {Z}_{1}={z}_{1} \\right] =\\mathbb{E}[X]"),
        "\\mathbb{E}\\left[ X\\mid {Z}_{1}={z}_{1} \\right] &=\\mathbb{E}[X]"
    );
}

#[test]
fn mathematical_unicode_alphabet_maps_to_latex_symbols() {
    assert_eq!(latex_math_char('𝜃'), "\\theta ");
    assert_eq!(latex_math_char('𝜙'), "\\varphi ");
    assert_eq!(latex_math_char('𝜓'), "\\psi ");
    assert_eq!(latex_math_char('𝜼'), "\\boldsymbol{\\eta}");
    assert_eq!(latex_math_char('𝔼'), "\\mathbb{E}");
    assert_eq!(latex_math_char('𝒩'), "\\mathcal{N}");
    assert_eq!(latex_math_char('\u{200b}'), "");
}

#[test]
fn multiline_equation_rows_share_an_alignment_column() {
    assert_eq!(
        format_multiline_equations("Variance<br>\n$x=1$<br>\n$=2$<br>\nafter"),
        Some(
            "Variance<br>\n$\\displaystyle\\qquad\\begin{aligned}x&=1 \\\\ &=2\\end{aligned}$<br>\nafter"
                .to_owned()
        )
    );
}
