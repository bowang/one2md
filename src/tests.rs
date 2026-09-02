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
}

#[test]
fn markdown_tables_keep_the_required_leading_blank_line() {
    let directory = tempfile::tempdir().unwrap();
    let mut assets = AssetWriter::new(directory.path().join("_assets"), "../_assets".to_owned());
    let mut renderer = Renderer::new(&mut assets);
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
fn input_cannot_be_used_as_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.one");
    fs::write(&path, b"not a real OneNote file").unwrap();
    let error = protect_input_from_overwrite(&path, &path).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn batch_conversion_skips_malformed_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.one");
    fs::write(&malformed, b"not a OneNote section").unwrap();
    let valid = Path::new(env!("CARGO_MANIFEST_DIR")).join("gm.one");
    let output = directory.path().join("output");

    let summary = convert_files([&malformed, &valid], &output).unwrap();

    assert_eq!(summary.pages, 38);
    assert!(summary.warnings.iter().any(|warning| {
        warning.contains("malformed.one") && warning.contains("skipped malformed backup")
    }));
    assert!(output.join("gm/_index.md").is_file());
}

#[test]
fn hierarchical_conversion_preserves_parent_folders_and_root_assets() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("backup");
    let input = source.join("Notebook/Section Group/Research.one");
    fs::create_dir_all(input.parent().unwrap()).unwrap();
    fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("gm.one"), &input).unwrap();
    let output = directory.path().join("output");

    let summary = convert_files_with_hierarchy([&input], &source, &output).unwrap();
    let section = output.join("Notebook/Section Group/Research");
    let markdown = fs::read_to_string(section.join("Conjugate prior.md")).unwrap();

    assert_eq!(summary.pages, 38);
    assert!(section.join("_index.md").is_file());
    assert!(output.join("_assets").is_dir());
    assert!(markdown.contains("../../../_assets/"));
    assert!(!output.join("Notebook/_assets").exists());
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
        format_multiline_equations("Variance  \n$x=1$  \n$=2$  \nafter"),
        Some(
            "Variance  \n$\\displaystyle\\qquad\\begin{aligned}x&=1 \\\\ &=2\\end{aligned}$  \nafter"
                .to_owned()
        )
    );
}

#[test]
fn gm_fixture_converts_equations_end_to_end() {
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("gm.one");
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output");
    let summary = convert_file(&input, &output).unwrap();
    let section = output.join("gm");
    let pages = fs::read_dir(&section)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    let page_subdirectories = fs::read_dir(&section)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    let assets = fs::read_dir(output.join("_assets"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .count();
    let markdown = pages
        .iter()
        .map(fs::read_to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    let variational = fs::read_to_string(section.join("Variational inference.md")).unwrap();
    let conditional = fs::read_to_string(section.join("Conditional Monte Carlo.md")).unwrap();
    let antithetic = fs::read_to_string(section.join("Antithetic variate.md")).unwrap();
    let index = fs::read_to_string(section.join("_index.md")).unwrap();
    let variational_references = variational.split_once("## References\n").unwrap().1;

    assert_eq!(summary.pages, 38);
    assert_eq!(pages.len(), 39);
    assert_eq!(assets, 37);
    assert_eq!(page_subdirectories, 0);
    assert!(section.join("Probabilistic graphical models.md").is_file());
    assert!(section.join("Chapman-Kolmogorov equation.md").is_file());
    assert!(
        section
            .join("Invariant risk minimization - IRM.md")
            .is_file()
    );
    assert!(section.join("Laplaces approximation.md").is_file());
    assert!(index.starts_with("# gm\n\n- [Probabilistic graphical models]"));
    assert!(index.contains(
        "[Invariant risk minimization (IRM)](<Invariant%20risk%20minimization%20-%20IRM.md>)"
    ));
    assert!(markdown.contains("# Invariant risk minimization (IRM)"));
    assert!(markdown.contains("\\frac{1}{Z}\\prod_{c} {\\psi }_{c}"));
    assert!(markdown.contains("\\sum_{k=0}^{\\infty } {p}_{ik}^{s}{p}_{kj}^{t}"));
    assert!(
        markdown.contains(
            "$\\displaystyle\\qquad\\begin{aligned}\\tilde{Y}&=Y-\\hat{Y}\\end{aligned}$"
        )
    );
    assert!(markdown.contains("Given observable data $x$"));
    assert!(markdown.contains("<em>Probabilistic inference</em>"));
    assert!(markdown.contains("where  \n\u{2003}\u{2003}$x$ : observables"));
    assert!(markdown.contains(
        "- Exact inference  \n  - Brute force  \n  - Variable elimination  \n  - Belief propagation"
    ));
    assert!(markdown.contains("We want to estimate  \n$\\displaystyle"));
    assert!(!markdown.contains("We want to estimate\n\n$\\displaystyle"));
    assert!(markdown.contains(
        "Let ${Z}_{j}={X}_{1j}-{X}_{2j}$ and $Z\\left( n \\right) =\\sum_{j=1}^{n} {Z}_{j}/n$\n\n$\\displaystyle"
    ));
    assert!(markdown.contains("\\sqrt{2\\eta }"));
    assert!(markdown.contains(
        "$\\displaystyle\\qquad\\begin{aligned}q\\left( \\mu  \\right) &\\propto p\\left( \\mu ,x \\right) =p"
    ));
    assert!(markdown.contains("\\\\ &\\propto \\exp"));
    assert!(markdown.contains("\\\\ &=\\exp"));
    assert!(markdown.contains("{\\nabla }_{w\\mid w=1.0}"));
    assert!(!markdown.contains("w&=1.0"));
    assert!(
        conditional
            .contains("\\mathbb{E}\\left[ X\\mid {Z}_{1}={z}_{1} \\right] &=\\mathbb{E}\\left[")
    );
    assert!(!conditional.contains("\\mid {Z}_{1}&={z}_{1}"));
    assert!(antithetic.contains(
        "\\operatorname{Var}\\left( {Y}_{i}+{Y}_{i}^{\\prime } \\right) \\\\ &=\\frac{1}"
    ));
    assert!(!antithetic.contains("\n$="));
    assert!(!markdown.contains("$$\n\\begin{aligned}"));
    assert!(!markdown.contains("<div style="));
    assert!(!markdown.contains("- !["));
    assert!(
        !markdown
            .match_indices("![")
            .any(|(index, _)| { !markdown[index + 2..].starts_with(']') })
    );
    assert!(markdown.contains("## References\n- [Machine Learning: Variational Inference]"));
    assert!(!markdown.contains("## References\n\n"));
    assert!(markdown.contains("## See Also\n- Law of total probability"));
    assert!(!markdown.contains("## See Also\n\n"));
    assert!(markdown.contains("See also: Thompson sampling"));
    assert!(markdown.contains("- [Attachment: 19a.pdf](../_assets/19a.pdf)"));
    assert!(!variational_references.contains("![]("));
    assert!(!variational_references.contains("\n\n"));
    assert!(
        !variational_references
            .lines()
            .any(|line| line.ends_with("  "))
    );
    assert!(!markdown.contains("onenote:"));
    assert!(!markdown.contains("&emsp;"));
    assert!(markdown.contains(
        "- [https://www.cs.princeton.edu/courses/archive/fall11/cos597C/lectures/variational-inference-i.pdf](<https://www.cs.princeton.edu/courses/archive/fall11/cos597C/lectures/variational-inference-i.pdf>)"
    ));
    assert!(markdown.contains(
        "Variational Inference: A Review for Statisticians [https://arxiv.org/abs/1601.00670](<https://arxiv.org/abs/1601.00670>)"
    ));
    assert!(markdown.contains("](../_assets/"));
    assert!(!markdown.contains("|  |  |\n| --- | --- |"));
    assert_eq!(markdown.matches('$').count() % 2, 0);
    assert!(!markdown.contains(['\u{fdd0}', '\u{fdee}', '\u{fdef}']));
    assert!(!markdown.chars().any(|ch| ('𝀀'..='𝟿').contains(&ch)));
}
