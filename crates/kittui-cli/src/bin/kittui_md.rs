//! `kittui-md` — standalone rich kittui Markdown viewer.

use std::io::{Read, Write};
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use kittui::scene::{background_linear, rounded_rect, scene};
use kittui::{CellRect, CellSize, Direction, RendererKind, Rgba, Runtime, Scene, Transport};
use kittui_affordances::{
    box_glyph_scene, render_markdown, ComponentKind, MarkdownDocument, MarkdownTable,
    MarkdownTableAlignment, TableGlyphLayout, UiComponent,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[allow(clippy::enum_variant_names)]
enum Mode {
    Rich,
    Plain,
    Components,
    ComponentsJson,
    Outline,
    OutlineJson,
    Anchors,
    AnchorsJson,
    References,
    ReferencesJson,
    Links,
    LinksJson,
    Footnotes,
    FootnotesJson,
    Images,
    ImagesJson,
    Tables,
    TablesJson,
    CodeBlocks,
    CodeBlocksJson,
    MetadataBlocks,
    MetadataBlocksJson,
    Definitions,
    DefinitionsJson,
    Math,
    MathJson,
    Html,
    HtmlJson,
    Modes,
    ModesJson,
    SchemasJson,
    ModeInfo,
    ModeInfoJson,
    ModeSearch,
    ModeSearchJson,
    ModeCategory,
    ModeCategoryJson,
    ModeCategories,
    ModeCategoriesJson,
    About,
    AboutJson,
    Capabilities,
    CapabilitiesJson,
    Version,
    VersionJson,
    InputFormats,
    InputFormatsJson,
    OutputFormats,
    OutputFormatsJson,
    Defaults,
    DefaultsJson,
    Examples,
    ExamplesJson,
    Limits,
    LimitsJson,
    Keybindings,
    KeybindingsJson,
    ExitCodes,
    ExitCodesJson,
    Counts,
    CountsJson,
    Stats,
    StatsJson,
    MetadataJson,
}

#[derive(Clone, Debug)]
struct Config {
    mode: Mode,
    width: u16,
    offset_rows: u16,
    height_rows: Option<u16>,
    interactive: bool,
    path: Option<String>,
    mode_info_name: Option<String>,
    mode_search_query: Option<String>,
    mode_category_name: Option<String>,
}

#[derive(Clone, Debug)]
struct LaidOutComponent<'a> {
    component: &'a UiComponent,
    rect: CellRect,
    table_index: Option<usize>,
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittui-md: {e}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<()> {
    let cfg = parse_args(std::env::args().skip(1))?;
    match cfg.mode {
        Mode::Modes => return write_modes(&mut std::io::stdout().lock()),
        Mode::ModesJson => return write_modes_json(&mut std::io::stdout().lock()),
        Mode::SchemasJson => return write_schemas_json(&mut std::io::stdout().lock()),
        Mode::ModeInfo => {
            let name = cfg
                .mode_info_name
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-info requires a value"))?;
            return write_mode_info(name, &mut std::io::stdout().lock());
        }
        Mode::ModeInfoJson => {
            let name = cfg
                .mode_info_name
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-info-json requires a value"))?;
            return write_mode_info_json(name, &mut std::io::stdout().lock());
        }
        Mode::ModeSearch => {
            let query = cfg
                .mode_search_query
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-search requires a value"))?;
            return write_mode_search(query, &mut std::io::stdout().lock());
        }
        Mode::ModeSearchJson => {
            let query = cfg
                .mode_search_query
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-search-json requires a value"))?;
            return write_mode_search_json(query, &mut std::io::stdout().lock());
        }
        Mode::ModeCategory => {
            let category = cfg
                .mode_category_name
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-category requires a value"))?;
            return write_mode_category(category, &mut std::io::stdout().lock());
        }
        Mode::ModeCategoryJson => {
            let category = cfg
                .mode_category_name
                .as_deref()
                .ok_or_else(|| anyhow!("--mode-category-json requires a value"))?;
            return write_mode_category_json(category, &mut std::io::stdout().lock());
        }
        Mode::ModeCategories => return write_mode_categories(&mut std::io::stdout().lock()),
        Mode::ModeCategoriesJson => {
            return write_mode_categories_json(&mut std::io::stdout().lock())
        }
        Mode::About => return write_about(&mut std::io::stdout().lock()),
        Mode::AboutJson => return write_about_json(&mut std::io::stdout().lock()),
        Mode::Capabilities => return write_capabilities(&mut std::io::stdout().lock()),
        Mode::CapabilitiesJson => return write_capabilities_json(&mut std::io::stdout().lock()),
        Mode::Version => return write_version(&mut std::io::stdout().lock()),
        Mode::VersionJson => return write_version_json(&mut std::io::stdout().lock()),
        Mode::InputFormats => return write_input_formats(&mut std::io::stdout().lock()),
        Mode::InputFormatsJson => return write_input_formats_json(&mut std::io::stdout().lock()),
        Mode::OutputFormats => return write_output_formats(&mut std::io::stdout().lock()),
        Mode::OutputFormatsJson => return write_output_formats_json(&mut std::io::stdout().lock()),
        Mode::Defaults => return write_defaults(&mut std::io::stdout().lock()),
        Mode::DefaultsJson => return write_defaults_json(&mut std::io::stdout().lock()),
        Mode::Examples => return write_examples(&mut std::io::stdout().lock()),
        Mode::ExamplesJson => return write_examples_json(&mut std::io::stdout().lock()),
        Mode::Limits => return write_limits(&mut std::io::stdout().lock()),
        Mode::LimitsJson => return write_limits_json(&mut std::io::stdout().lock()),
        Mode::Keybindings => return write_keybindings(&mut std::io::stdout().lock()),
        Mode::KeybindingsJson => return write_keybindings_json(&mut std::io::stdout().lock()),
        Mode::ExitCodes => return write_exit_codes(&mut std::io::stdout().lock()),
        Mode::ExitCodesJson => return write_exit_codes_json(&mut std::io::stdout().lock()),
        _ => {}
    }
    let markdown = if let Some(path) = &cfg.path {
        std::fs::read_to_string(path)?
    } else {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    };
    let doc = render_markdown(&markdown, cfg.width);
    match cfg.mode {
        Mode::Plain => write_plain(&doc, cfg.width, &mut std::io::stdout().lock()),
        Mode::Components => write_components(&doc, &mut std::io::stdout().lock()),
        Mode::ComponentsJson => write_components_json(&doc, &mut std::io::stdout().lock()),
        Mode::Outline => write_outline(&doc, &mut std::io::stdout().lock()),
        Mode::OutlineJson => write_outline_json(&doc, &mut std::io::stdout().lock()),
        Mode::Anchors => write_anchors(&doc, &mut std::io::stdout().lock()),
        Mode::AnchorsJson => write_anchors_json(&doc, &mut std::io::stdout().lock()),
        Mode::References => write_references(&doc, &mut std::io::stdout().lock()),
        Mode::ReferencesJson => write_references_json(&doc, &mut std::io::stdout().lock()),
        Mode::Links => write_links(&doc, &mut std::io::stdout().lock()),
        Mode::LinksJson => write_links_json(&doc, &mut std::io::stdout().lock()),
        Mode::Footnotes => write_footnotes(&doc, &mut std::io::stdout().lock()),
        Mode::FootnotesJson => write_footnotes_json(&doc, &mut std::io::stdout().lock()),
        Mode::Images => write_images(&doc, &mut std::io::stdout().lock()),
        Mode::ImagesJson => write_images_json(&doc, &mut std::io::stdout().lock()),
        Mode::Tables => write_tables(&doc, &mut std::io::stdout().lock()),
        Mode::TablesJson => write_tables_json(&doc, &mut std::io::stdout().lock()),
        Mode::CodeBlocks => write_code_blocks(&doc, &mut std::io::stdout().lock()),
        Mode::CodeBlocksJson => write_code_blocks_json(&doc, &mut std::io::stdout().lock()),
        Mode::MetadataBlocks => write_metadata_blocks(&doc, &mut std::io::stdout().lock()),
        Mode::MetadataBlocksJson => write_metadata_blocks_json(&doc, &mut std::io::stdout().lock()),
        Mode::Definitions => write_definitions(&doc, &mut std::io::stdout().lock()),
        Mode::DefinitionsJson => write_definitions_json(&doc, &mut std::io::stdout().lock()),
        Mode::Math => write_math(&doc, &mut std::io::stdout().lock()),
        Mode::MathJson => write_math_json(&doc, &mut std::io::stdout().lock()),
        Mode::Html => write_html(&doc, &mut std::io::stdout().lock()),
        Mode::HtmlJson => write_html_json(&doc, &mut std::io::stdout().lock()),
        Mode::Modes => unreachable!("mode listing returns before reading input"),
        Mode::ModesJson => unreachable!("mode listing returns before reading input"),
        Mode::SchemasJson => unreachable!("schema listing returns before reading input"),
        Mode::ModeInfo => unreachable!("mode info returns before reading input"),
        Mode::ModeInfoJson => unreachable!("mode info returns before reading input"),
        Mode::ModeSearch => unreachable!("mode search returns before reading input"),
        Mode::ModeSearchJson => unreachable!("mode search returns before reading input"),
        Mode::ModeCategory => unreachable!("mode category returns before reading input"),
        Mode::ModeCategoryJson => unreachable!("mode category returns before reading input"),
        Mode::ModeCategories => unreachable!("mode categories return before reading input"),
        Mode::ModeCategoriesJson => unreachable!("mode categories return before reading input"),
        Mode::About => unreachable!("about returns before reading input"),
        Mode::AboutJson => unreachable!("about returns before reading input"),
        Mode::Capabilities => unreachable!("capabilities return before reading input"),
        Mode::CapabilitiesJson => unreachable!("capabilities return before reading input"),
        Mode::Version => unreachable!("version returns before reading input"),
        Mode::VersionJson => unreachable!("version returns before reading input"),
        Mode::InputFormats => unreachable!("input formats return before reading input"),
        Mode::InputFormatsJson => unreachable!("input formats return before reading input"),
        Mode::OutputFormats => unreachable!("output formats return before reading input"),
        Mode::OutputFormatsJson => unreachable!("output formats return before reading input"),
        Mode::Defaults => unreachable!("defaults return before reading input"),
        Mode::DefaultsJson => unreachable!("defaults return before reading input"),
        Mode::Examples => unreachable!("examples return before reading input"),
        Mode::ExamplesJson => unreachable!("examples return before reading input"),
        Mode::Limits => unreachable!("limits return before reading input"),
        Mode::LimitsJson => unreachable!("limits return before reading input"),
        Mode::Keybindings => unreachable!("keybindings return before reading input"),
        Mode::KeybindingsJson => unreachable!("keybindings return before reading input"),
        Mode::ExitCodes => unreachable!("exit codes return before reading input"),
        Mode::ExitCodesJson => unreachable!("exit codes return before reading input"),
        Mode::Counts => write_counts(&doc, &mut std::io::stdout().lock()),
        Mode::CountsJson => write_counts_json(&doc, &mut std::io::stdout().lock()),
        Mode::Stats => write_stats(
            &doc,
            &markdown,
            cfg.path.as_deref(),
            cfg.width,
            &mut std::io::stdout().lock(),
        ),
        Mode::StatsJson => write_stats_json(
            &doc,
            &markdown,
            cfg.path.as_deref(),
            cfg.width,
            &mut std::io::stdout().lock(),
        ),
        Mode::MetadataJson => write_metadata_json(
            &doc,
            &markdown,
            cfg.width,
            cfg.path.as_deref(),
            &mut std::io::stdout().lock(),
        ),
        Mode::Rich if cfg.interactive => run_interactive(&markdown, cfg),
        Mode::Rich => write_rich(&doc, &cfg, &mut std::io::stdout().lock()),
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut mode = Mode::Rich;
    let mut mode_flag: Option<&'static str> = None;
    let mut width = terminal_cols().unwrap_or(80).clamp(20, 120);
    let mut offset_rows = 0;
    let mut height_rows = None;
    let mut interactive = false;
    let mut path = None;
    let mut mode_info_name = None;
    let mut mode_search_query = None;
    let mut mode_category_name = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plain" => set_mode(&mut mode, &mut mode_flag, "--plain", Mode::Plain)?,
            "--rich" => set_mode(&mut mode, &mut mode_flag, "--rich", Mode::Rich)?,
            "--components" => {
                set_mode(&mut mode, &mut mode_flag, "--components", Mode::Components)?
            }
            "--widgets" => set_mode(&mut mode, &mut mode_flag, "--widgets", Mode::Components)?,
            "--components-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--components-json",
                Mode::ComponentsJson,
            )?,
            "--outline" => set_mode(&mut mode, &mut mode_flag, "--outline", Mode::Outline)?,
            "--toc" => set_mode(&mut mode, &mut mode_flag, "--toc", Mode::Outline)?,
            "--headings" => set_mode(&mut mode, &mut mode_flag, "--headings", Mode::Outline)?,
            "--outline-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--outline-json",
                Mode::OutlineJson,
            )?,
            "--anchors" => set_mode(&mut mode, &mut mode_flag, "--anchors", Mode::Anchors)?,
            "--slugs" => set_mode(&mut mode, &mut mode_flag, "--slugs", Mode::Anchors)?,
            "--anchors-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--anchors-json",
                Mode::AnchorsJson,
            )?,
            "--references" => {
                set_mode(&mut mode, &mut mode_flag, "--references", Mode::References)?
            }
            "--refs" => set_mode(&mut mode, &mut mode_flag, "--refs", Mode::References)?,
            "--references-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--references-json",
                Mode::ReferencesJson,
            )?,
            "--links" => set_mode(&mut mode, &mut mode_flag, "--links", Mode::Links)?,
            "--urls" => set_mode(&mut mode, &mut mode_flag, "--urls", Mode::Links)?,
            "--links-json" => set_mode(&mut mode, &mut mode_flag, "--links-json", Mode::LinksJson)?,
            "--footnotes" => set_mode(&mut mode, &mut mode_flag, "--footnotes", Mode::Footnotes)?,
            "--notes" => set_mode(&mut mode, &mut mode_flag, "--notes", Mode::Footnotes)?,
            "--footnotes-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--footnotes-json",
                Mode::FootnotesJson,
            )?,
            "--images" => set_mode(&mut mode, &mut mode_flag, "--images", Mode::Images)?,
            "--pictures" => set_mode(&mut mode, &mut mode_flag, "--pictures", Mode::Images)?,
            "--images-json" => {
                set_mode(&mut mode, &mut mode_flag, "--images-json", Mode::ImagesJson)?
            }
            "--tables" => set_mode(&mut mode, &mut mode_flag, "--tables", Mode::Tables)?,
            "--grid" => set_mode(&mut mode, &mut mode_flag, "--grid", Mode::Tables)?,
            "--tables-json" => {
                set_mode(&mut mode, &mut mode_flag, "--tables-json", Mode::TablesJson)?
            }
            "--code-blocks" => {
                set_mode(&mut mode, &mut mode_flag, "--code-blocks", Mode::CodeBlocks)?
            }
            "--snippets" => set_mode(&mut mode, &mut mode_flag, "--snippets", Mode::CodeBlocks)?,
            "--code-blocks-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--code-blocks-json",
                Mode::CodeBlocksJson,
            )?,
            "--metadata-blocks" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-blocks",
                Mode::MetadataBlocks,
            )?,
            "--metadata" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata",
                Mode::MetadataBlocks,
            )?,
            "--frontmatter" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--frontmatter",
                Mode::MetadataBlocks,
            )?,
            "--metadata-blocks-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-blocks-json",
                Mode::MetadataBlocksJson,
            )?,
            "--definitions" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--definitions",
                Mode::Definitions,
            )?,
            "--glossary" => set_mode(&mut mode, &mut mode_flag, "--glossary", Mode::Definitions)?,
            "--definitions-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--definitions-json",
                Mode::DefinitionsJson,
            )?,
            "--math" => set_mode(&mut mode, &mut mode_flag, "--math", Mode::Math)?,
            "--equations" => set_mode(&mut mode, &mut mode_flag, "--equations", Mode::Math)?,
            "--math-json" => set_mode(&mut mode, &mut mode_flag, "--math-json", Mode::MathJson)?,
            "--html" => set_mode(&mut mode, &mut mode_flag, "--html", Mode::Html)?,
            "--markup" => set_mode(&mut mode, &mut mode_flag, "--markup", Mode::Html)?,
            "--html-json" => set_mode(&mut mode, &mut mode_flag, "--html-json", Mode::HtmlJson)?,
            "--modes" => set_mode(&mut mode, &mut mode_flag, "--modes", Mode::Modes)?,
            "--modes-json" => set_mode(&mut mode, &mut mode_flag, "--modes-json", Mode::ModesJson)?,
            "--schemas-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--schemas-json",
                Mode::SchemasJson,
            )?,
            "--mode-info" => {
                mode_info_name = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-info requires a value"))?,
                );
                set_mode(&mut mode, &mut mode_flag, "--mode-info", Mode::ModeInfo)?;
            }
            "--mode-info-json" => {
                mode_info_name = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-info-json requires a value"))?,
                );
                set_mode(
                    &mut mode,
                    &mut mode_flag,
                    "--mode-info-json",
                    Mode::ModeInfoJson,
                )?;
            }
            "--mode-search" => {
                mode_search_query = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-search requires a value"))?,
                );
                set_mode(&mut mode, &mut mode_flag, "--mode-search", Mode::ModeSearch)?;
            }
            "--mode-search-json" => {
                mode_search_query = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-search-json requires a value"))?,
                );
                set_mode(
                    &mut mode,
                    &mut mode_flag,
                    "--mode-search-json",
                    Mode::ModeSearchJson,
                )?;
            }
            "--mode-category" => {
                mode_category_name = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-category requires a value"))?,
                );
                set_mode(
                    &mut mode,
                    &mut mode_flag,
                    "--mode-category",
                    Mode::ModeCategory,
                )?;
            }
            "--mode-category-json" => {
                mode_category_name = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--mode-category-json requires a value"))?,
                );
                set_mode(
                    &mut mode,
                    &mut mode_flag,
                    "--mode-category-json",
                    Mode::ModeCategoryJson,
                )?;
            }
            "--mode-categories" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--mode-categories",
                Mode::ModeCategories,
            )?,
            "--mode-categories-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--mode-categories-json",
                Mode::ModeCategoriesJson,
            )?,
            "--about" => set_mode(&mut mode, &mut mode_flag, "--about", Mode::About)?,
            "--about-json" => set_mode(&mut mode, &mut mode_flag, "--about-json", Mode::AboutJson)?,
            "--capabilities" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--capabilities",
                Mode::Capabilities,
            )?,
            "--capabilities-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--capabilities-json",
                Mode::CapabilitiesJson,
            )?,
            "--version" => set_mode(&mut mode, &mut mode_flag, "--version", Mode::Version)?,
            "--version-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--version-json",
                Mode::VersionJson,
            )?,
            "--input-formats" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--input-formats",
                Mode::InputFormats,
            )?,
            "--input-formats-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--input-formats-json",
                Mode::InputFormatsJson,
            )?,
            "--output-formats" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--output-formats",
                Mode::OutputFormats,
            )?,
            "--output-formats-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--output-formats-json",
                Mode::OutputFormatsJson,
            )?,
            "--defaults" => set_mode(&mut mode, &mut mode_flag, "--defaults", Mode::Defaults)?,
            "--defaults-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--defaults-json",
                Mode::DefaultsJson,
            )?,
            "--examples" => set_mode(&mut mode, &mut mode_flag, "--examples", Mode::Examples)?,
            "--examples-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--examples-json",
                Mode::ExamplesJson,
            )?,
            "--limits" => set_mode(&mut mode, &mut mode_flag, "--limits", Mode::Limits)?,
            "--limits-json" => {
                set_mode(&mut mode, &mut mode_flag, "--limits-json", Mode::LimitsJson)?
            }
            "--keybindings" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--keybindings",
                Mode::Keybindings,
            )?,
            "--keybindings-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--keybindings-json",
                Mode::KeybindingsJson,
            )?,
            "--exit-codes" => set_mode(&mut mode, &mut mode_flag, "--exit-codes", Mode::ExitCodes)?,
            "--exit-codes-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--exit-codes-json",
                Mode::ExitCodesJson,
            )?,
            "--counts" => set_mode(&mut mode, &mut mode_flag, "--counts", Mode::Counts)?,
            "--counts-json" => {
                set_mode(&mut mode, &mut mode_flag, "--counts-json", Mode::CountsJson)?
            }
            "--stats" => set_mode(&mut mode, &mut mode_flag, "--stats", Mode::Stats)?,
            "--summary" => set_mode(&mut mode, &mut mode_flag, "--summary", Mode::Stats)?,
            "--stats-json" => set_mode(&mut mode, &mut mode_flag, "--stats-json", Mode::StatsJson)?,
            "--metadata-json" => set_mode(
                &mut mode,
                &mut mode_flag,
                "--metadata-json",
                Mode::MetadataJson,
            )?,
            "--json" => set_mode(&mut mode, &mut mode_flag, "--json", Mode::MetadataJson)?,
            "--mode" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--mode requires a value"))?;
                let (selected, flag) = mode_from_name(&value)?;
                set_mode(&mut mode, &mut mode_flag, flag, selected)?;
            }
            "--width" => {
                width = args
                    .next()
                    .ok_or_else(|| anyhow!("--width requires a value"))?
                    .parse()?
            }
            "--offset" => {
                offset_rows = args
                    .next()
                    .ok_or_else(|| anyhow!("--offset requires a value"))?
                    .parse()?
            }
            "--interactive" | "-i" => interactive = true,
            "--height" => {
                height_rows = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--height requires a value"))?
                        .parse()?,
                )
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => return Err(anyhow!("unknown flag {arg}")),
            _ => {
                if path.replace(arg).is_some() {
                    return Err(anyhow!("expected at most one input path"));
                }
            }
        }
    }
    Ok(Config {
        mode,
        width: width.clamp(20, 200),
        offset_rows,
        height_rows,
        interactive,
        path,
        mode_info_name,
        mode_search_query,
        mode_category_name,
    })
}

fn set_mode(
    mode: &mut Mode,
    seen: &mut Option<&'static str>,
    flag: &'static str,
    next: Mode,
) -> Result<()> {
    if let Some(prev) = *seen {
        return Err(anyhow!(
            "output modes are mutually exclusive: {prev} and {flag}"
        ));
    }
    *mode = next;
    *seen = Some(flag);
    Ok(())
}

fn mode_from_name(name: &str) -> Result<(Mode, &'static str)> {
    let normalized = name.trim_start_matches("--");
    match normalized {
        "rich" => Ok((Mode::Rich, "--rich")),
        "plain" => Ok((Mode::Plain, "--plain")),
        "components" => Ok((Mode::Components, "--components")),
        "widgets" => Ok((Mode::Components, "--widgets")),
        "components-json" => Ok((Mode::ComponentsJson, "--components-json")),
        "outline" => Ok((Mode::Outline, "--outline")),
        "toc" => Ok((Mode::Outline, "--toc")),
        "headings" => Ok((Mode::Outline, "--headings")),
        "outline-json" => Ok((Mode::OutlineJson, "--outline-json")),
        "anchors" => Ok((Mode::Anchors, "--anchors")),
        "slugs" => Ok((Mode::Anchors, "--slugs")),
        "anchors-json" => Ok((Mode::AnchorsJson, "--anchors-json")),
        "references" => Ok((Mode::References, "--references")),
        "refs" => Ok((Mode::References, "--refs")),
        "references-json" => Ok((Mode::ReferencesJson, "--references-json")),
        "links" => Ok((Mode::Links, "--links")),
        "urls" => Ok((Mode::Links, "--urls")),
        "links-json" => Ok((Mode::LinksJson, "--links-json")),
        "footnotes" => Ok((Mode::Footnotes, "--footnotes")),
        "notes" => Ok((Mode::Footnotes, "--notes")),
        "footnotes-json" => Ok((Mode::FootnotesJson, "--footnotes-json")),
        "images" => Ok((Mode::Images, "--images")),
        "pictures" => Ok((Mode::Images, "--pictures")),
        "images-json" => Ok((Mode::ImagesJson, "--images-json")),
        "tables" => Ok((Mode::Tables, "--tables")),
        "grid" => Ok((Mode::Tables, "--grid")),
        "tables-json" => Ok((Mode::TablesJson, "--tables-json")),
        "code-blocks" => Ok((Mode::CodeBlocks, "--code-blocks")),
        "snippets" => Ok((Mode::CodeBlocks, "--snippets")),
        "code-blocks-json" => Ok((Mode::CodeBlocksJson, "--code-blocks-json")),
        "metadata-blocks" => Ok((Mode::MetadataBlocks, "--metadata-blocks")),
        "metadata" => Ok((Mode::MetadataBlocks, "--metadata")),
        "frontmatter" => Ok((Mode::MetadataBlocks, "--frontmatter")),
        "metadata-blocks-json" => Ok((Mode::MetadataBlocksJson, "--metadata-blocks-json")),
        "definitions" => Ok((Mode::Definitions, "--definitions")),
        "glossary" => Ok((Mode::Definitions, "--glossary")),
        "definitions-json" => Ok((Mode::DefinitionsJson, "--definitions-json")),
        "math" => Ok((Mode::Math, "--math")),
        "equations" => Ok((Mode::Math, "--equations")),
        "math-json" => Ok((Mode::MathJson, "--math-json")),
        "html" => Ok((Mode::Html, "--html")),
        "markup" => Ok((Mode::Html, "--markup")),
        "html-json" => Ok((Mode::HtmlJson, "--html-json")),
        "modes" => Ok((Mode::Modes, "--modes")),
        "modes-json" => Ok((Mode::ModesJson, "--modes-json")),
        "schemas-json" => Ok((Mode::SchemasJson, "--schemas-json")),
        "mode-info" => Ok((Mode::ModeInfo, "--mode-info")),
        "mode-info-json" => Ok((Mode::ModeInfoJson, "--mode-info-json")),
        "mode-search" => Ok((Mode::ModeSearch, "--mode-search")),
        "mode-search-json" => Ok((Mode::ModeSearchJson, "--mode-search-json")),
        "mode-category" => Ok((Mode::ModeCategory, "--mode-category")),
        "mode-category-json" => Ok((Mode::ModeCategoryJson, "--mode-category-json")),
        "mode-categories" => Ok((Mode::ModeCategories, "--mode-categories")),
        "mode-categories-json" => Ok((Mode::ModeCategoriesJson, "--mode-categories-json")),
        "about" => Ok((Mode::About, "--about")),
        "about-json" => Ok((Mode::AboutJson, "--about-json")),
        "capabilities" => Ok((Mode::Capabilities, "--capabilities")),
        "capabilities-json" => Ok((Mode::CapabilitiesJson, "--capabilities-json")),
        "version" => Ok((Mode::Version, "--version")),
        "version-json" => Ok((Mode::VersionJson, "--version-json")),
        "input-formats" => Ok((Mode::InputFormats, "--input-formats")),
        "input-formats-json" => Ok((Mode::InputFormatsJson, "--input-formats-json")),
        "output-formats" => Ok((Mode::OutputFormats, "--output-formats")),
        "output-formats-json" => Ok((Mode::OutputFormatsJson, "--output-formats-json")),
        "defaults" => Ok((Mode::Defaults, "--defaults")),
        "defaults-json" => Ok((Mode::DefaultsJson, "--defaults-json")),
        "examples" => Ok((Mode::Examples, "--examples")),
        "examples-json" => Ok((Mode::ExamplesJson, "--examples-json")),
        "limits" => Ok((Mode::Limits, "--limits")),
        "limits-json" => Ok((Mode::LimitsJson, "--limits-json")),
        "keybindings" => Ok((Mode::Keybindings, "--keybindings")),
        "keybindings-json" => Ok((Mode::KeybindingsJson, "--keybindings-json")),
        "exit-codes" => Ok((Mode::ExitCodes, "--exit-codes")),
        "exit-codes-json" => Ok((Mode::ExitCodesJson, "--exit-codes-json")),
        "counts" => Ok((Mode::Counts, "--counts")),
        "counts-json" => Ok((Mode::CountsJson, "--counts-json")),
        "stats" => Ok((Mode::Stats, "--stats")),
        "summary" => Ok((Mode::Stats, "--summary")),
        "stats-json" => Ok((Mode::StatsJson, "--stats-json")),
        "metadata-json" => Ok((Mode::MetadataJson, "--metadata-json")),
        "json" => Ok((Mode::MetadataJson, "--json")),
        _ => Err(anyhow!("unknown --mode value {name}")),
    }
}

fn print_help() {
    println!("kittui-md [--mode NAME|--rich|--plain|--components|--widgets|--components-json|--outline|--toc|--headings|--outline-json|--anchors|--slugs|--anchors-json|--references|--refs|--references-json|--links|--urls|--links-json|--footnotes|--notes|--footnotes-json|--images|--pictures|--images-json|--tables|--grid|--tables-json|--code-blocks|--snippets|--code-blocks-json|--metadata-blocks|--metadata|--frontmatter|--metadata-blocks-json|--definitions|--glossary|--definitions-json|--math|--equations|--math-json|--html|--markup|--html-json|--modes|--modes-json|--schemas-json|--mode-info NAME|--mode-info-json NAME|--mode-search QUERY|--mode-search-json QUERY|--mode-category CATEGORY|--mode-category-json CATEGORY|--mode-categories|--mode-categories-json|--about|--about-json|--capabilities|--capabilities-json|--version|--version-json|--input-formats|--input-formats-json|--output-formats|--output-formats-json|--defaults|--defaults-json|--examples|--examples-json|--limits|--limits-json|--keybindings|--keybindings-json|--exit-codes|--exit-codes-json|--counts|--counts-json|--stats|--summary|--stats-json|--metadata-json|--json] [--interactive] [--width N] [--offset ROWS] [--height ROWS] [file]");
    println!(
        "Render Markdown as kittui/kitty graphics components. Reads stdin when file is omitted."
    );
}

fn run_interactive(markdown: &str, mut cfg: Config) -> Result<()> {
    let path = cfg.path.clone().ok_or_else(|| {
        anyhow!("--interactive requires an input file so stdin can be used for keys")
    })?;
    let mut doc = render_markdown(markdown, cfg.width);
    let _raw = RawTerminal::enter()?;
    let mut stdout = std::io::stdout().lock();
    let mut stdin = std::io::stdin().lock();
    let viewport = cfg
        .height_rows
        .unwrap_or_else(|| terminal_rows().unwrap_or(24).saturating_sub(2).max(1));
    cfg.height_rows = Some(viewport);
    let mut total_rows = document_rows(&doc, cfg.width);
    let mut show_help = false;
    let mut show_outline = false;
    let mut show_links = false;
    let mut show_images = false;
    let mut show_tables = false;
    let mut show_code_blocks = false;
    let mut show_footnotes = false;
    let mut show_definitions = false;
    let mut show_math = false;
    let mut show_html = false;
    let mut status: Option<String> = None;
    loop {
        write!(stdout, "\x1b[2J\x1b[H")?;
        if show_help {
            write_interactive_help(viewport, &mut stdout)?;
        } else if show_outline {
            write_interactive_outline(&doc, viewport, &mut stdout)?;
        } else if show_links {
            write_interactive_links(&doc, viewport, &mut stdout)?;
        } else if show_images {
            write_interactive_images(&doc, viewport, &mut stdout)?;
        } else if show_tables {
            write_interactive_tables(&doc, viewport, &mut stdout)?;
        } else if show_code_blocks {
            write_interactive_code_blocks(&doc, viewport, &mut stdout)?;
        } else if show_footnotes {
            write_interactive_footnotes(&doc, viewport, &mut stdout)?;
        } else if show_definitions {
            write_interactive_definitions(&doc, viewport, &mut stdout)?;
        } else if show_math {
            write_interactive_math(&doc, viewport, &mut stdout)?;
        } else if show_html {
            write_interactive_html(&doc, viewport, &mut stdout)?;
        } else {
            write_rich(&doc, &cfg, &mut stdout)?;
        }
        write_interactive_footer(
            show_help,
            show_outline,
            show_links,
            show_images,
            show_tables,
            show_code_blocks,
            show_footnotes,
            show_definitions,
            show_math,
            show_html,
            status.as_deref(),
            &path,
            cfg.offset_rows,
            viewport,
            total_rows,
            &mut stdout,
        )?;
        stdout.flush()?;
        let action = read_pager_action(&mut stdin)?;
        if action == PagerAction::Quit {
            break;
        }
        if action == PagerAction::Help {
            show_help = !show_help;
            if show_help {
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Outline {
            show_outline = !show_outline;
            if show_outline {
                show_help = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Links {
            show_links = !show_links;
            if show_links {
                show_help = false;
                show_outline = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Images {
            show_images = !show_images;
            if show_images {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Tables {
            show_tables = !show_tables;
            if show_tables {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::CodeBlocks {
            show_code_blocks = !show_code_blocks;
            if show_code_blocks {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Footnotes {
            show_footnotes = !show_footnotes;
            if show_footnotes {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Definitions {
            show_definitions = !show_definitions;
            if show_definitions {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Math {
            show_math = !show_math;
            if show_math {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_html = false;
            }
            continue;
        }
        if action == PagerAction::Html {
            show_html = !show_html;
            if show_html {
                show_help = false;
                show_outline = false;
                show_links = false;
                show_images = false;
                show_tables = false;
                show_code_blocks = false;
                show_footnotes = false;
                show_definitions = false;
                show_math = false;
            }
            continue;
        }
        if action == PagerAction::Reload {
            match reload_interactive_document(&path, cfg.width) {
                Ok(reloaded) => {
                    doc = reloaded;
                    total_rows = document_rows(&doc, cfg.width);
                    cfg.offset_rows = cfg.offset_rows.min(total_rows.saturating_sub(viewport));
                    show_outline = false;
                    show_links = false;
                    show_images = false;
                    show_tables = false;
                    show_code_blocks = false;
                    show_footnotes = false;
                    show_definitions = false;
                    show_math = false;
                    show_html = false;
                    status = Some(format!("reloaded {path} — {total_rows} rows"));
                }
                Err(err) => {
                    status = Some(format!("reload failed: {err}"));
                }
            }
            show_help = false;
            continue;
        }
        if action == PagerAction::ClearStatus {
            status = None;
            continue;
        }
        if show_help
            || show_outline
            || show_links
            || show_images
            || show_tables
            || show_code_blocks
            || show_footnotes
            || show_definitions
            || show_math
            || show_html
        {
            continue;
        }
        cfg.offset_rows = apply_pager_action(cfg.offset_rows, viewport, total_rows, action);
    }
    write!(stdout, "\x1b[0m\x1b[?25h\x1b[2J\x1b[H")?;
    stdout.flush()?;
    Ok(())
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PagerAction {
    Noop,
    Quit,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Help,
    Outline,
    Links,
    Images,
    Tables,
    CodeBlocks,
    Footnotes,
    Definitions,
    Math,
    Html,
    Reload,
    ClearStatus,
}

fn read_pager_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'q' | 3 => PagerAction::Quit,
        b'k' | b'w' => PagerAction::Up,
        b'j' | b'\n' | b'\r' => PagerAction::Down,
        b' ' => PagerAction::PageDown,
        b'b' => PagerAction::PageUp,
        b'g' => PagerAction::Home,
        b'G' => PagerAction::End,
        b'h' | b'?' => PagerAction::Help,
        b'o' => PagerAction::Outline,
        b'l' => PagerAction::Links,
        b'i' => PagerAction::Images,
        b't' => PagerAction::Tables,
        b's' => PagerAction::CodeBlocks,
        b'f' => PagerAction::Footnotes,
        b'd' => PagerAction::Definitions,
        b'm' => PagerAction::Math,
        b'x' => PagerAction::Html,
        b'r' => PagerAction::Reload,
        b'c' => PagerAction::ClearStatus,
        27 => read_escape_action(input)?,
        _ => PagerAction::Noop,
    })
}

fn read_escape_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    if input.read(&mut buf)? == 0 {
        return Ok(PagerAction::Quit);
    }
    match buf[0] {
        b'[' => read_csi_action(input),
        b'O' => read_ss3_action(input),
        _ => Ok(PagerAction::Noop),
    }
}

fn read_csi_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'A' => PagerAction::Up,
        b'B' => PagerAction::Down,
        b'H' => PagerAction::Home,
        b'F' => PagerAction::End,
        b'1' | b'7' => {
            consume_optional_tilde(input)?;
            PagerAction::Home
        }
        b'4' | b'8' => {
            consume_optional_tilde(input)?;
            PagerAction::End
        }
        b'5' => {
            consume_optional_tilde(input)?;
            PagerAction::PageUp
        }
        b'6' => {
            consume_optional_tilde(input)?;
            PagerAction::PageDown
        }
        _ => PagerAction::Noop,
    })
}

fn read_ss3_action(input: &mut impl Read) -> Result<PagerAction> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(match buf[0] {
        b'H' => PagerAction::Home,
        b'F' => PagerAction::End,
        _ => PagerAction::Noop,
    })
}

fn consume_optional_tilde(input: &mut impl Read) -> Result<()> {
    let mut buf = [0u8; 1];
    if input.read(&mut buf)? == 0 || buf[0] == b'~' {
        return Ok(());
    }
    Ok(())
}

fn apply_pager_action(
    offset: u16,
    viewport_rows: u16,
    total_rows: u16,
    action: PagerAction,
) -> u16 {
    let max_offset = total_rows.saturating_sub(viewport_rows);
    match action {
        PagerAction::Noop => offset.min(max_offset),
        PagerAction::Quit => offset,
        PagerAction::Up => offset.saturating_sub(1),
        PagerAction::Down => offset.saturating_add(1).min(max_offset),
        PagerAction::PageUp => offset.saturating_sub(viewport_rows.max(1)),
        PagerAction::PageDown => offset.saturating_add(viewport_rows.max(1)).min(max_offset),
        PagerAction::Home => 0,
        PagerAction::End => max_offset,
        PagerAction::Help
        | PagerAction::Outline
        | PagerAction::Links
        | PagerAction::Images
        | PagerAction::Tables
        | PagerAction::CodeBlocks
        | PagerAction::Footnotes
        | PagerAction::Definitions
        | PagerAction::Math
        | PagerAction::Html
        | PagerAction::Reload
        | PagerAction::ClearStatus => offset.min(max_offset),
    }
}

fn write_interactive_help(viewport_rows: u16, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md interactive help")?;
    writeln!(out)?;
    for binding in KEYBINDINGS
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        writeln!(
            out,
            "{}: {} — {}",
            binding.action,
            binding.keys.join(", "),
            binding.description
        )?;
    }
    Ok(())
}

fn write_interactive_outline(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md outline — {} headings", doc.outline.len())?;
    writeln!(out)?;
    if doc.outline.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for line in outline_lines(doc)
        .into_iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn write_interactive_links(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md links — {} links", doc.links.len())?;
    writeln!(out)?;
    if doc.links.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for link in doc
        .links
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        if let Some(title) = &link.title {
            writeln!(out, "[{}] {} \"{}\"", link.label, link.url, title)?;
        } else {
            writeln!(out, "[{}] {}", link.label, link.url)?;
        }
    }
    Ok(())
}

fn write_interactive_images(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md images — {} images", doc.images.len())?;
    writeln!(out)?;
    if doc.images.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for image in doc
        .images
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        if let Some(title) = &image.title {
            writeln!(out, "![{}] {} \"{}\"", image.alt, image.url, title)?;
        } else {
            writeln!(out, "![{}] {}", image.alt, image.url)?;
        }
    }
    Ok(())
}

fn write_interactive_tables(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md tables — {} tables", doc.tables.len())?;
    writeln!(out)?;
    if doc.tables.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (index, table) in doc
        .tables
        .iter()
        .enumerate()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        let footprint = table.footprint();
        let columns = table.column_widths().len();
        let alignments = table
            .alignments
            .iter()
            .map(|alignment| alignment.as_str())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "table #{index}: rows={} columns={} footprint={}x{} alignments={}",
            table.rows.len(),
            columns,
            footprint.cols,
            footprint.rows,
            if alignments.is_empty() {
                "none"
            } else {
                &alignments
            }
        )?;
    }
    Ok(())
}

fn write_interactive_code_blocks(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(
        out,
        "kittui-md code blocks — {} blocks",
        doc.code_blocks.len()
    )?;
    writeln!(out)?;
    if doc.code_blocks.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (index, block) in doc
        .code_blocks
        .iter()
        .enumerate()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        let language = block.language.as_deref().unwrap_or("plain");
        let lines = block.text.lines().count().max(1);
        let preview = block.text.lines().next().unwrap_or("");
        writeln!(
            out,
            "code #{index}: language={language} lines={lines} preview={preview}"
        )?;
    }
    Ok(())
}

fn write_interactive_footnotes(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(
        out,
        "kittui-md footnotes — {} references, {} definitions",
        doc.footnote_references.len(),
        doc.footnotes.len()
    )?;
    writeln!(out)?;
    if doc.footnote_references.is_empty() && doc.footnotes.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    let mut written = 0usize;
    for reference in &doc.footnote_references {
        if written >= viewport_rows.saturating_sub(3) as usize {
            return Ok(());
        }
        writeln!(out, "ref [^{reference}]")?;
        written += 1;
    }
    for footnote in &doc.footnotes {
        if written >= viewport_rows.saturating_sub(3) as usize {
            return Ok(());
        }
        writeln!(out, "def [^{}]: {}", footnote.label, footnote.text)?;
        written += 1;
    }
    Ok(())
}

fn write_interactive_definitions(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(
        out,
        "kittui-md definitions — {} definitions",
        doc.definitions.len()
    )?;
    writeln!(out)?;
    if doc.definitions.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for definition in doc
        .definitions
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        writeln!(out, "{}: {}", definition.term, definition.definition)?;
    }
    Ok(())
}

fn write_interactive_math(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md math — {} expressions", doc.math.len())?;
    writeln!(out)?;
    if doc.math.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for math in doc
        .math
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        writeln!(out, "{}: {}", math.kind.as_str(), math.source)?;
    }
    Ok(())
}

fn write_interactive_html(
    doc: &MarkdownDocument,
    viewport_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md html — {} fragments", doc.html.len())?;
    writeln!(out)?;
    if doc.html.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for html in doc
        .html
        .iter()
        .take(viewport_rows.saturating_sub(3) as usize)
    {
        writeln!(out, "{}: {}", html.kind.as_str(), html.source)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_interactive_footer(
    show_help: bool,
    show_outline: bool,
    show_links: bool,
    show_images: bool,
    show_tables: bool,
    show_code_blocks: bool,
    show_footnotes: bool,
    show_definitions: bool,
    show_math: bool,
    show_html: bool,
    status: Option<&str>,
    path: &str,
    offset_rows: u16,
    viewport_rows: u16,
    total_rows: u16,
    out: &mut impl Write,
) -> Result<()> {
    let max_offset = total_rows.saturating_sub(viewport_rows);
    writeln!(
        out,
        "source: {path} • offset {}/{} • viewport {} • rows {}",
        offset_rows.min(max_offset),
        max_offset,
        viewport_rows,
        total_rows
    )?;
    if let Some(status) = status {
        writeln!(out, "status: {status}")?;
    }
    if show_help {
        writeln!(
            out,
            "h/? close help • o outline • l links • i images • t tables • s code • f footnotes • d definitions • m math • x html • r reload • c clear status • q quit"
        )?;
    } else if show_outline {
        writeln!(
            out,
            "o close outline • l links • i images • t tables • s code • f footnotes • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_links {
        writeln!(
            out,
            "l close links • o outline • i images • t tables • s code • f footnotes • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_images {
        writeln!(
            out,
            "i close images • o outline • l links • t tables • s code • f footnotes • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_tables {
        writeln!(
            out,
            "t close tables • o outline • l links • i images • s code • f footnotes • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_code_blocks {
        writeln!(
            out,
            "s close code • o outline • l links • i images • t tables • f footnotes • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_footnotes {
        writeln!(
            out,
            "f close footnotes • o outline • l links • i images • t tables • s code • d definitions • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_definitions {
        writeln!(
            out,
            "d close definitions • o outline • l links • i images • t tables • s code • f footnotes • m math • x html • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_math {
        writeln!(
            out,
            "m close math • x html • o outline • l links • i images • t tables • s code • f footnotes • d definitions • h/? help • r reload • c clear status • q quit"
        )?;
    } else if show_html {
        writeln!(
            out,
            "x close html • o outline • l links • i images • t tables • s code • f footnotes • d definitions • m math • h/? help • r reload • c clear status • q quit"
        )?;
    } else {
        writeln!(
            out,
            "j/k scroll • space/page down • b/page up • g/G ends • h/? help • o outline • l links • i images • t tables • s code • f footnotes • d definitions • m math • x html • r reload • c clear status • q quit"
        )?;
    }
    Ok(())
}

fn reload_interactive_document(path: &str, width: u16) -> Result<MarkdownDocument> {
    let markdown = std::fs::read_to_string(path)?;
    Ok(render_markdown(&markdown, width))
}

fn document_rows(doc: &MarkdownDocument, width: u16) -> u16 {
    layout_components(&doc.components, &doc.tables, width)
        .last()
        .map(|item| item.rect.y.saturating_add(item.rect.rows))
        .unwrap_or(0)
}

struct RawTerminal {
    original: libc::termios,
}

impl RawTerminal {
    fn enter() -> Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        let rc = unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) };
        if rc != 0 {
            return Err(anyhow!(
                "tcgetattr failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        let rc = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
        if rc != 0 {
            return Err(anyhow!(
                "tcsetattr raw failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { original })
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
    }
}

struct ModeInfo {
    flag: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
}

const MODE_INFOS: &[ModeInfo] = &[
    ModeInfo {
        flag: "--rich",
        aliases: &[],
        description: "render rich kitty graphics components",
    },
    ModeInfo {
        flag: "--plain",
        aliases: &[],
        description: "print component records plus text metadata sections",
    },
    ModeInfo {
        flag: "--components",
        aliases: &["--widgets"],
        description: "print generated UI component records",
    },
    ModeInfo {
        flag: "--components-json",
        aliases: &[],
        description: "emit generated UI component records as JSON",
    },
    ModeInfo {
        flag: "--outline",
        aliases: &["--toc", "--headings"],
        description: "print heading outline records",
    },
    ModeInfo {
        flag: "--outline-json",
        aliases: &[],
        description: "emit heading outline records as JSON",
    },
    ModeInfo {
        flag: "--anchors",
        aliases: &["--slugs"],
        description: "print heading anchor records",
    },
    ModeInfo {
        flag: "--anchors-json",
        aliases: &[],
        description: "emit heading anchor records as JSON",
    },
    ModeInfo {
        flag: "--references",
        aliases: &["--refs"],
        description: "print combined link, image, and footnote references",
    },
    ModeInfo {
        flag: "--references-json",
        aliases: &[],
        description: "emit combined link, image, and footnote references as JSON",
    },
    ModeInfo {
        flag: "--links",
        aliases: &["--urls"],
        description: "print link records",
    },
    ModeInfo {
        flag: "--links-json",
        aliases: &[],
        description: "emit link records as JSON",
    },
    ModeInfo {
        flag: "--footnotes",
        aliases: &["--notes"],
        description: "print footnote references and definitions",
    },
    ModeInfo {
        flag: "--footnotes-json",
        aliases: &[],
        description: "emit footnote references and definitions as JSON",
    },
    ModeInfo {
        flag: "--images",
        aliases: &["--pictures"],
        description: "print image records",
    },
    ModeInfo {
        flag: "--images-json",
        aliases: &[],
        description: "emit image records as JSON",
    },
    ModeInfo {
        flag: "--tables",
        aliases: &["--grid"],
        description: "print table layout records",
    },
    ModeInfo {
        flag: "--tables-json",
        aliases: &[],
        description: "emit table layout records as JSON",
    },
    ModeInfo {
        flag: "--code-blocks",
        aliases: &["--snippets"],
        description: "print code block records",
    },
    ModeInfo {
        flag: "--code-blocks-json",
        aliases: &[],
        description: "emit code block records as JSON",
    },
    ModeInfo {
        flag: "--metadata-blocks",
        aliases: &["--metadata", "--frontmatter"],
        description: "print frontmatter/metadata block records",
    },
    ModeInfo {
        flag: "--metadata-blocks-json",
        aliases: &[],
        description: "emit frontmatter/metadata block records as JSON",
    },
    ModeInfo {
        flag: "--definitions",
        aliases: &["--glossary"],
        description: "print definition-list records",
    },
    ModeInfo {
        flag: "--definitions-json",
        aliases: &[],
        description: "emit definition-list records as JSON",
    },
    ModeInfo {
        flag: "--math",
        aliases: &["--equations"],
        description: "print math expression records",
    },
    ModeInfo {
        flag: "--math-json",
        aliases: &[],
        description: "emit math expression records as JSON",
    },
    ModeInfo {
        flag: "--html",
        aliases: &["--markup"],
        description: "print HTML placeholder records",
    },
    ModeInfo {
        flag: "--html-json",
        aliases: &[],
        description: "emit HTML placeholder records as JSON",
    },
    ModeInfo {
        flag: "--modes",
        aliases: &[],
        description: "list available output modes",
    },
    ModeInfo {
        flag: "--modes-json",
        aliases: &[],
        description: "emit available output modes as JSON",
    },
    ModeInfo {
        flag: "--schemas-json",
        aliases: &[],
        description: "emit JSON output schema summaries",
    },
    ModeInfo {
        flag: "--mode-info",
        aliases: &[],
        description: "describe one output mode",
    },
    ModeInfo {
        flag: "--mode-info-json",
        aliases: &[],
        description: "emit one output mode description as JSON",
    },
    ModeInfo {
        flag: "--mode-search",
        aliases: &[],
        description: "search output modes by flag, alias, or description",
    },
    ModeInfo {
        flag: "--mode-search-json",
        aliases: &[],
        description: "emit output mode search results as JSON",
    },
    ModeInfo {
        flag: "--mode-category",
        aliases: &[],
        description: "list output modes in one category",
    },
    ModeInfo {
        flag: "--mode-category-json",
        aliases: &[],
        description: "emit output modes in one category as JSON",
    },
    ModeInfo {
        flag: "--mode-categories",
        aliases: &[],
        description: "list supported output mode categories",
    },
    ModeInfo {
        flag: "--mode-categories-json",
        aliases: &[],
        description: "emit supported output mode categories as JSON",
    },
    ModeInfo {
        flag: "--about",
        aliases: &[],
        description: "print binary version and capability summary",
    },
    ModeInfo {
        flag: "--about-json",
        aliases: &[],
        description: "emit binary version and capability summary as JSON",
    },
    ModeInfo {
        flag: "--capabilities",
        aliases: &[],
        description: "list high-level binary capabilities",
    },
    ModeInfo {
        flag: "--capabilities-json",
        aliases: &[],
        description: "emit high-level binary capabilities as JSON",
    },
    ModeInfo {
        flag: "--version",
        aliases: &[],
        description: "print binary package version",
    },
    ModeInfo {
        flag: "--version-json",
        aliases: &[],
        description: "emit binary package version as JSON",
    },
    ModeInfo {
        flag: "--input-formats",
        aliases: &[],
        description: "list supported input formats",
    },
    ModeInfo {
        flag: "--input-formats-json",
        aliases: &[],
        description: "emit supported input formats as JSON",
    },
    ModeInfo {
        flag: "--output-formats",
        aliases: &[],
        description: "list supported output format families",
    },
    ModeInfo {
        flag: "--output-formats-json",
        aliases: &[],
        description: "emit supported output format families as JSON",
    },
    ModeInfo {
        flag: "--defaults",
        aliases: &[],
        description: "print default viewer settings",
    },
    ModeInfo {
        flag: "--defaults-json",
        aliases: &[],
        description: "emit default viewer settings as JSON",
    },
    ModeInfo {
        flag: "--examples",
        aliases: &[],
        description: "print common invocation examples",
    },
    ModeInfo {
        flag: "--examples-json",
        aliases: &[],
        description: "emit common invocation examples as JSON",
    },
    ModeInfo {
        flag: "--limits",
        aliases: &[],
        description: "print numeric CLI limits",
    },
    ModeInfo {
        flag: "--limits-json",
        aliases: &[],
        description: "emit numeric CLI limits as JSON",
    },
    ModeInfo {
        flag: "--keybindings",
        aliases: &[],
        description: "print interactive pager keybindings",
    },
    ModeInfo {
        flag: "--keybindings-json",
        aliases: &[],
        description: "emit interactive pager keybindings as JSON",
    },
    ModeInfo {
        flag: "--exit-codes",
        aliases: &[],
        description: "print process exit code meanings",
    },
    ModeInfo {
        flag: "--exit-codes-json",
        aliases: &[],
        description: "emit process exit code meanings as JSON",
    },
    ModeInfo {
        flag: "--counts",
        aliases: &[],
        description: "print compact structural counts",
    },
    ModeInfo {
        flag: "--counts-json",
        aliases: &[],
        description: "emit compact structural counts as JSON",
    },
    ModeInfo {
        flag: "--stats",
        aliases: &["--summary"],
        description: "print source, render, and count summary",
    },
    ModeInfo {
        flag: "--stats-json",
        aliases: &[],
        description: "emit source, render, and count summary as JSON",
    },
    ModeInfo {
        flag: "--metadata-json",
        aliases: &["--json"],
        description: "emit full document metadata as JSON",
    },
];

struct JsonSchemaInfo {
    mode: &'static str,
    top_level_keys: &'static [&'static str],
    description: &'static str,
}

const JSON_SCHEMA_INFOS: &[JsonSchemaInfo] = &[
    JsonSchemaInfo {
        mode: "--components-json",
        top_level_keys: &["schema_version", "components"],
        description: "Generated UI component records.",
    },
    JsonSchemaInfo {
        mode: "--outline-json",
        top_level_keys: &["schema_version", "outline"],
        description: "Heading outline records.",
    },
    JsonSchemaInfo {
        mode: "--anchors-json",
        top_level_keys: &["schema_version", "anchors"],
        description: "Heading anchor records.",
    },
    JsonSchemaInfo {
        mode: "--references-json",
        top_level_keys: &[
            "schema_version",
            "links",
            "images",
            "footnote_references",
            "footnotes",
        ],
        description: "Combined reference records.",
    },
    JsonSchemaInfo {
        mode: "--links-json",
        top_level_keys: &["schema_version", "links"],
        description: "Markdown link records.",
    },
    JsonSchemaInfo {
        mode: "--footnotes-json",
        top_level_keys: &["schema_version", "references", "definitions"],
        description: "Footnote references and definitions.",
    },
    JsonSchemaInfo {
        mode: "--images-json",
        top_level_keys: &["schema_version", "images"],
        description: "Markdown image records.",
    },
    JsonSchemaInfo {
        mode: "--tables-json",
        top_level_keys: &["schema_version", "tables"],
        description: "Markdown table layout records.",
    },
    JsonSchemaInfo {
        mode: "--code-blocks-json",
        top_level_keys: &["schema_version", "code_blocks"],
        description: "Markdown code block records.",
    },
    JsonSchemaInfo {
        mode: "--metadata-blocks-json",
        top_level_keys: &["schema_version", "metadata_blocks"],
        description: "YAML/pluses metadata block records.",
    },
    JsonSchemaInfo {
        mode: "--definitions-json",
        top_level_keys: &["schema_version", "definitions"],
        description: "Definition-list records.",
    },
    JsonSchemaInfo {
        mode: "--math-json",
        top_level_keys: &["schema_version", "math"],
        description: "Math expression records.",
    },
    JsonSchemaInfo {
        mode: "--html-json",
        top_level_keys: &["schema_version", "html"],
        description: "HTML placeholder records.",
    },
    JsonSchemaInfo {
        mode: "--modes-json",
        top_level_keys: &["schema_version", "modes"],
        description: "Available output mode catalog.",
    },
    JsonSchemaInfo {
        mode: "--schemas-json",
        top_level_keys: &["schema_version", "schemas"],
        description: "JSON output schema summary catalog.",
    },
    JsonSchemaInfo {
        mode: "--mode-info-json",
        top_level_keys: &["schema_version", "mode"],
        description: "Single output mode description.",
    },
    JsonSchemaInfo {
        mode: "--mode-search-json",
        top_level_keys: &["schema_version", "query", "matches"],
        description: "Output mode search results.",
    },
    JsonSchemaInfo {
        mode: "--mode-category-json",
        top_level_keys: &["schema_version", "category", "modes"],
        description: "Output modes in one category.",
    },
    JsonSchemaInfo {
        mode: "--mode-categories-json",
        top_level_keys: &["schema_version", "categories"],
        description: "Supported output mode categories.",
    },
    JsonSchemaInfo {
        mode: "--about-json",
        top_level_keys: &[
            "schema_version",
            "binary",
            "package_version",
            "default_mode",
            "capabilities",
        ],
        description: "Binary version and capability summary.",
    },
    JsonSchemaInfo {
        mode: "--capabilities-json",
        top_level_keys: &["schema_version", "capabilities"],
        description: "High-level binary capabilities.",
    },
    JsonSchemaInfo {
        mode: "--version-json",
        top_level_keys: &["schema_version", "binary", "package_version"],
        description: "Binary package version.",
    },
    JsonSchemaInfo {
        mode: "--input-formats-json",
        top_level_keys: &["schema_version", "input_formats"],
        description: "Supported input formats.",
    },
    JsonSchemaInfo {
        mode: "--output-formats-json",
        top_level_keys: &["schema_version", "output_formats"],
        description: "Supported output format families.",
    },
    JsonSchemaInfo {
        mode: "--defaults-json",
        top_level_keys: &["schema_version", "defaults"],
        description: "Default viewer settings.",
    },
    JsonSchemaInfo {
        mode: "--examples-json",
        top_level_keys: &["schema_version", "examples"],
        description: "Common invocation examples.",
    },
    JsonSchemaInfo {
        mode: "--limits-json",
        top_level_keys: &["schema_version", "limits"],
        description: "Numeric CLI limits.",
    },
    JsonSchemaInfo {
        mode: "--keybindings-json",
        top_level_keys: &["schema_version", "keybindings"],
        description: "Interactive pager keybindings.",
    },
    JsonSchemaInfo {
        mode: "--exit-codes-json",
        top_level_keys: &["schema_version", "exit_codes"],
        description: "Process exit code meanings.",
    },
    JsonSchemaInfo {
        mode: "--counts-json",
        top_level_keys: &["schema_version", "counts"],
        description: "Compact structural counts.",
    },
    JsonSchemaInfo {
        mode: "--stats-json",
        top_level_keys: &["schema_version", "source", "render", "counts"],
        description: "Source, render, and count summary.",
    },
    JsonSchemaInfo {
        mode: "--metadata-json",
        top_level_keys: &[
            "schema_version",
            "source",
            "render",
            "counts",
            "components",
            "components_detail",
            "outline",
            "links",
            "images",
            "footnote_references",
            "footnotes",
            "definitions",
            "math",
            "html",
            "metadata_blocks",
            "code_blocks",
            "tables",
        ],
        description: "Full document metadata payload.",
    },
];

fn write_modes(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md modes — {} output modes", MODE_INFOS.len())?;
    for info in MODE_INFOS {
        if info.aliases.is_empty() {
            writeln!(out, "{} — {}", info.flag, info.description)?;
        } else {
            writeln!(
                out,
                "{} ({}) — {}",
                info.flag,
                info.aliases.join(", "),
                info.description
            )?;
        }
    }
    Ok(())
}

fn mode_category(flag: &str) -> &'static str {
    match flag {
        "--rich" | "--plain" => "render",
        "--components" | "--outline" | "--anchors" | "--references" | "--links" | "--footnotes"
        | "--images" | "--tables" | "--code-blocks" | "--metadata-blocks" | "--definitions"
        | "--math" | "--html" => "inspect",
        "--counts" | "--stats" => "stats",
        "--modes"
        | "--modes-json"
        | "--schemas-json"
        | "--mode-info"
        | "--mode-info-json"
        | "--mode-search"
        | "--mode-search-json"
        | "--mode-category"
        | "--mode-category-json"
        | "--mode-categories"
        | "--mode-categories-json"
        | "--about"
        | "--about-json"
        | "--capabilities"
        | "--capabilities-json"
        | "--version"
        | "--version-json"
        | "--input-formats"
        | "--input-formats-json"
        | "--output-formats"
        | "--output-formats-json"
        | "--defaults"
        | "--defaults-json"
        | "--examples"
        | "--examples-json"
        | "--limits"
        | "--limits-json"
        | "--keybindings"
        | "--keybindings-json"
        | "--exit-codes"
        | "--exit-codes-json" => "discovery",
        _ if flag.ends_with("-json") || flag == "--json" => "json",
        _ => "other",
    }
}

fn write_modes_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "modes": MODE_INFOS.iter().enumerate().map(|(index, info)| serde_json::json!({
            "index": index,
            "flag": info.flag,
            "aliases": info.aliases,
            "description": info.description,
            "category": mode_category(info.flag),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_schemas_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "schemas": JSON_SCHEMA_INFOS.iter().enumerate().map(|(index, info)| serde_json::json!({
            "index": index,
            "mode": info.mode,
            "category": mode_category(info.mode),
            "top_level_keys": info.top_level_keys,
            "description": info.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn mode_info_for_name(name: &str) -> Result<&'static ModeInfo> {
    let normalized = name.trim_start_matches("--");
    MODE_INFOS
        .iter()
        .find(|info| {
            info.flag.trim_start_matches("--") == normalized
                || info
                    .aliases
                    .iter()
                    .any(|alias| alias.trim_start_matches("--") == normalized)
        })
        .ok_or_else(|| anyhow!("unknown mode info value {name}"))
}

fn schema_info_for_mode(flag: &str) -> Option<&'static JsonSchemaInfo> {
    JSON_SCHEMA_INFOS.iter().find(|info| info.mode == flag)
}

fn write_mode_info(name: &str, out: &mut impl Write) -> Result<()> {
    let info = mode_info_for_name(name)?;
    writeln!(out, "kittui-md mode info — {}", info.flag)?;
    if info.aliases.is_empty() {
        writeln!(out, "aliases: <none>")?;
    } else {
        writeln!(out, "aliases: {}", info.aliases.join(", "))?;
    }
    writeln!(out, "description: {}", info.description)?;
    if let Some(schema) = schema_info_for_mode(info.flag) {
        writeln!(
            out,
            "json_top_level_keys: {}",
            schema.top_level_keys.join(", ")
        )?;
    } else {
        writeln!(out, "json_top_level_keys: <none>")?;
    }
    Ok(())
}

fn write_mode_info_json(name: &str, out: &mut impl Write) -> Result<()> {
    let info = mode_info_for_name(name)?;
    let schema = schema_info_for_mode(info.flag).map(|schema| {
        serde_json::json!({
            "top_level_keys": schema.top_level_keys,
            "description": schema.description,
        })
    });
    let value = serde_json::json!({
        "schema_version": 1,
        "mode": {
            "flag": info.flag,
            "aliases": info.aliases,
            "description": info.description,
            "category": mode_category(info.flag),
            "json_schema": schema,
        },
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn mode_search_matches(query: &str) -> Vec<&'static ModeInfo> {
    let needle = query.to_ascii_lowercase();
    MODE_INFOS
        .iter()
        .filter(|info| {
            info.flag.to_ascii_lowercase().contains(&needle)
                || info
                    .aliases
                    .iter()
                    .any(|alias| alias.to_ascii_lowercase().contains(&needle))
                || info.description.to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

fn write_mode_search(query: &str, out: &mut impl Write) -> Result<()> {
    let matches = mode_search_matches(query);
    writeln!(
        out,
        "kittui-md mode search — {} matches for {:?}",
        matches.len(),
        query
    )?;
    if matches.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for info in matches {
        if info.aliases.is_empty() {
            writeln!(out, "{} — {}", info.flag, info.description)?;
        } else {
            writeln!(
                out,
                "{} ({}) — {}",
                info.flag,
                info.aliases.join(", "),
                info.description
            )?;
        }
    }
    Ok(())
}

fn write_mode_search_json(query: &str, out: &mut impl Write) -> Result<()> {
    let matches = mode_search_matches(query);
    let value = serde_json::json!({
        "schema_version": 1,
        "query": query,
        "matches": matches.iter().enumerate().map(|(index, info)| {
            let schema = schema_info_for_mode(info.flag).map(|schema| serde_json::json!({
                "top_level_keys": schema.top_level_keys,
                "description": schema.description,
            }));
            serde_json::json!({
                "index": index,
                "flag": info.flag,
                "aliases": info.aliases,
                "description": info.description,
                "category": mode_category(info.flag),
                "json_schema": schema,
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

const MODE_CATEGORIES: &[&str] = &["render", "inspect", "json", "discovery", "stats", "other"];

fn validate_mode_category(category: &str) -> Result<&str> {
    if MODE_CATEGORIES.contains(&category) {
        Ok(category)
    } else {
        Err(anyhow!("unknown mode category {category}"))
    }
}

fn mode_category_matches(category: &str) -> Result<Vec<&'static ModeInfo>> {
    let category = validate_mode_category(category)?;
    Ok(MODE_INFOS
        .iter()
        .filter(|info| mode_category(info.flag) == category)
        .collect())
}

fn write_mode_category(category: &str, out: &mut impl Write) -> Result<()> {
    let matches = mode_category_matches(category)?;
    writeln!(
        out,
        "kittui-md mode category — {} modes in {:?}",
        matches.len(),
        category
    )?;
    if matches.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for info in matches {
        if info.aliases.is_empty() {
            writeln!(out, "{} — {}", info.flag, info.description)?;
        } else {
            writeln!(
                out,
                "{} ({}) — {}",
                info.flag,
                info.aliases.join(", "),
                info.description
            )?;
        }
    }
    Ok(())
}

fn write_mode_category_json(category: &str, out: &mut impl Write) -> Result<()> {
    let matches = mode_category_matches(category)?;
    let value = serde_json::json!({
        "schema_version": 1,
        "category": category,
        "modes": matches.iter().enumerate().map(|(index, info)| serde_json::json!({
            "index": index,
            "flag": info.flag,
            "aliases": info.aliases,
            "description": info.description,
            "category": mode_category(info.flag),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn mode_category_count(category: &str) -> usize {
    MODE_INFOS
        .iter()
        .filter(|info| mode_category(info.flag) == category)
        .count()
}

fn write_mode_categories(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md mode categories")?;
    for category in MODE_CATEGORIES {
        writeln!(out, "{category}={}", mode_category_count(category))?;
    }
    Ok(())
}

fn write_mode_categories_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "categories": MODE_CATEGORIES.iter().enumerate().map(|(index, category)| serde_json::json!({
            "index": index,
            "name": category,
            "count": mode_category_count(category),
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

const ABOUT_CAPABILITIES: &[&str] = &[
    "rich-kitty-graphics-rendering",
    "plain-text-rendering",
    "interactive-pager",
    "focused-inspection-modes",
    "machine-readable-json-outputs",
    "mode-discovery",
];

fn write_about(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md about")?;
    writeln!(out, "binary=kittui-md")?;
    writeln!(out, "package_version={}", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "default_mode=rich")?;
    writeln!(out, "capabilities={}", ABOUT_CAPABILITIES.join(","))?;
    Ok(())
}

fn write_about_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "binary": "kittui-md",
        "package_version": env!("CARGO_PKG_VERSION"),
        "default_mode": "rich",
        "capabilities": ABOUT_CAPABILITIES,
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_capabilities(out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md capabilities — {} capabilities",
        ABOUT_CAPABILITIES.len()
    )?;
    for capability in ABOUT_CAPABILITIES {
        writeln!(out, "{capability}")?;
    }
    Ok(())
}

fn write_capabilities_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "capabilities": ABOUT_CAPABILITIES,
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_version(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md {}", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn write_version_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "binary": "kittui-md",
        "package_version": env!("CARGO_PKG_VERSION"),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

struct InputFormatInfo {
    name: &'static str,
    extensions: &'static [&'static str],
    description: &'static str,
}

const INPUT_FORMATS: &[InputFormatInfo] = &[InputFormatInfo {
    name: "markdown",
    extensions: &["md", "markdown", "mdown"],
    description: "CommonMark-compatible Markdown parsed by pulldown-cmark.",
}];

fn write_input_formats(out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md input formats — {} formats",
        INPUT_FORMATS.len()
    )?;
    for format in INPUT_FORMATS {
        writeln!(
            out,
            "{} ({}) — {}",
            format.name,
            format.extensions.join(", "),
            format.description
        )?;
    }
    Ok(())
}

fn write_input_formats_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "input_formats": INPUT_FORMATS.iter().enumerate().map(|(index, format)| serde_json::json!({
            "index": index,
            "name": format.name,
            "extensions": format.extensions,
            "description": format.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

struct OutputFormatInfo {
    name: &'static str,
    mode_categories: &'static [&'static str],
    description: &'static str,
}

const OUTPUT_FORMATS: &[OutputFormatInfo] = &[
    OutputFormatInfo {
        name: "rich-kitty-graphics",
        mode_categories: &["render"],
        description: "Kitty graphics protocol output with text overlays.",
    },
    OutputFormatInfo {
        name: "plain-text",
        mode_categories: &["render", "inspect", "stats", "discovery"],
        description: "Human-readable terminal text output.",
    },
    OutputFormatInfo {
        name: "json",
        mode_categories: &["json", "discovery", "stats"],
        description: "Schema-versioned machine-readable JSON output.",
    },
];

fn write_output_formats(out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md output formats — {} formats",
        OUTPUT_FORMATS.len()
    )?;
    for format in OUTPUT_FORMATS {
        writeln!(
            out,
            "{} ({}) — {}",
            format.name,
            format.mode_categories.join(", "),
            format.description
        )?;
    }
    Ok(())
}

fn write_output_formats_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "output_formats": OUTPUT_FORMATS.iter().enumerate().map(|(index, format)| serde_json::json!({
            "index": index,
            "name": format.name,
            "mode_categories": format.mode_categories,
            "description": format.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_defaults(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md defaults")?;
    writeln!(out, "mode=rich")?;
    writeln!(out, "width.min=20")?;
    writeln!(out, "width.max=200")?;
    writeln!(out, "width.terminal_default_min=20")?;
    writeln!(out, "width.terminal_default_max=120")?;
    writeln!(out, "offset_rows=0")?;
    writeln!(out, "interactive=false")?;
    writeln!(out, "input=stdin-or-one-file")?;
    Ok(())
}

fn write_defaults_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "defaults": {
            "mode": "rich",
            "width": {
                "min": 20,
                "max": 200,
                "terminal_default_min": 20,
                "terminal_default_max": 120,
            },
            "offset_rows": 0,
            "interactive": false,
            "input": "stdin-or-one-file",
        },
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

struct ExampleInfo {
    name: &'static str,
    argv: &'static [&'static str],
    description: &'static str,
}

const EXAMPLES: &[ExampleInfo] = &[
    ExampleInfo {
        name: "rich-file",
        argv: &["kittui-md", "docs/examples/kittui-md-proof.md"],
        description: "Render a Markdown file in rich kitty graphics mode.",
    },
    ExampleInfo {
        name: "plain-file",
        argv: &["kittui-md", "--plain", "docs/examples/kittui-md-proof.md"],
        description: "Render a Markdown file as plain text records.",
    },
    ExampleInfo {
        name: "component-json",
        argv: &[
            "kittui-md",
            "--components-json",
            "docs/examples/kittui-md-proof.md",
        ],
        description: "Emit generated component records as JSON.",
    },
    ExampleInfo {
        name: "mode-search",
        argv: &["kittui-md", "--mode-search-json", "table"],
        description: "Search the mode catalog for table-related outputs.",
    },
];

fn write_examples(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md examples — {} examples", EXAMPLES.len())?;
    for example in EXAMPLES {
        writeln!(
            out,
            "{}: {} — {}",
            example.name,
            example.argv.join(" "),
            example.description
        )?;
    }
    Ok(())
}

fn write_examples_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "examples": EXAMPLES.iter().enumerate().map(|(index, example)| serde_json::json!({
            "index": index,
            "name": example.name,
            "argv": example.argv,
            "description": example.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_limits(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md limits")?;
    writeln!(out, "width.min=20")?;
    writeln!(out, "width.max=200")?;
    writeln!(out, "width.terminal_default_min=20")?;
    writeln!(out, "width.terminal_default_max=120")?;
    writeln!(out, "offset_rows.min=0")?;
    writeln!(out, "height_rows.min=1")?;
    Ok(())
}

fn write_limits_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "limits": {
            "width": {
                "min": 20,
                "max": 200,
                "terminal_default_min": 20,
                "terminal_default_max": 120,
            },
            "offset_rows": { "min": 0 },
            "height_rows": { "min": 1 },
        },
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

struct KeybindingInfo {
    action: &'static str,
    keys: &'static [&'static str],
    description: &'static str,
}

const KEYBINDINGS: &[KeybindingInfo] = &[
    KeybindingInfo {
        action: "scroll-up",
        keys: &["k", "w", "Up"],
        description: "Scroll the interactive pager up by one row.",
    },
    KeybindingInfo {
        action: "scroll-down",
        keys: &["j", "s", "Enter", "Down"],
        description: "Scroll the interactive pager down by one row.",
    },
    KeybindingInfo {
        action: "page-up",
        keys: &["b", "PageUp"],
        description: "Scroll the interactive pager up by one viewport.",
    },
    KeybindingInfo {
        action: "page-down",
        keys: &["Space", "PageDown"],
        description: "Scroll the interactive pager down by one viewport.",
    },
    KeybindingInfo {
        action: "home",
        keys: &["g", "Home"],
        description: "Jump to the first rendered row.",
    },
    KeybindingInfo {
        action: "end",
        keys: &["G", "End"],
        description: "Jump to the last rendered row.",
    },
    KeybindingInfo {
        action: "help",
        keys: &["h", "?"],
        description: "Toggle the interactive help screen.",
    },
    KeybindingInfo {
        action: "outline",
        keys: &["o"],
        description: "Toggle the interactive document outline screen.",
    },
    KeybindingInfo {
        action: "links",
        keys: &["l"],
        description: "Toggle the interactive document links screen.",
    },
    KeybindingInfo {
        action: "images",
        keys: &["i"],
        description: "Toggle the interactive document images screen.",
    },
    KeybindingInfo {
        action: "tables",
        keys: &["t"],
        description: "Toggle the interactive document tables screen.",
    },
    KeybindingInfo {
        action: "code-blocks",
        keys: &["s"],
        description: "Toggle the interactive document code blocks screen.",
    },
    KeybindingInfo {
        action: "footnotes",
        keys: &["f"],
        description: "Toggle the interactive document footnotes screen.",
    },
    KeybindingInfo {
        action: "definitions",
        keys: &["d"],
        description: "Toggle the interactive document definitions screen.",
    },
    KeybindingInfo {
        action: "math",
        keys: &["m"],
        description: "Toggle the interactive document math screen.",
    },
    KeybindingInfo {
        action: "html",
        keys: &["x"],
        description: "Toggle the interactive document HTML screen.",
    },
    KeybindingInfo {
        action: "reload",
        keys: &["r"],
        description: "Reload the Markdown file from disk.",
    },
    KeybindingInfo {
        action: "clear-status",
        keys: &["c"],
        description: "Clear the current interactive status message.",
    },
    KeybindingInfo {
        action: "quit",
        keys: &["q", "Ctrl-C"],
        description: "Exit the interactive pager.",
    },
];

fn write_keybindings(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md keybindings — {} actions", KEYBINDINGS.len())?;
    for binding in KEYBINDINGS {
        writeln!(
            out,
            "{}: {} — {}",
            binding.action,
            binding.keys.join(", "),
            binding.description
        )?;
    }
    Ok(())
}

fn write_keybindings_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "keybindings": KEYBINDINGS.iter().enumerate().map(|(index, binding)| serde_json::json!({
            "index": index,
            "action": binding.action,
            "keys": binding.keys,
            "description": binding.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

struct ExitCodeInfo {
    code: i32,
    name: &'static str,
    description: &'static str,
}

const EXIT_CODES: &[ExitCodeInfo] = &[
    ExitCodeInfo {
        code: 0,
        name: "success",
        description: "Command completed successfully.",
    },
    ExitCodeInfo {
        code: 1,
        name: "error",
        description:
            "Invalid arguments, unreadable input, rendering failure, or another runtime error.",
    },
];

fn write_exit_codes(out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md exit codes — {} codes", EXIT_CODES.len())?;
    for code in EXIT_CODES {
        writeln!(out, "{} {} — {}", code.code, code.name, code.description)?;
    }
    Ok(())
}

fn write_exit_codes_json(out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "exit_codes": EXIT_CODES.iter().enumerate().map(|(index, code)| serde_json::json!({
            "index": index,
            "code": code.code,
            "name": code.name,
            "description": code.description,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_components(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md components — {} components",
        doc.components.len()
    )?;
    if doc.components.is_empty() {
        writeln!(out, "<empty>")?;
    } else {
        for component in &doc.components {
            write_plain_component(out, component)?;
        }
    }
    Ok(())
}

fn write_components_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "components": doc.components.iter().enumerate().map(|(index, component)| serde_json::json!({
            "index": index,
            "kind": format!("{:?}", component.kind),
            "text": component.text,
            "width_cells": component.width_cells,
            "height_cells": component.height_cells,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_outline(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md outline — {} headings", doc.outline.len())?;
    if doc.outline.is_empty() {
        writeln!(out, "<empty>")?;
    } else {
        for line in outline_lines(doc) {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

fn write_outline_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "outline": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "text": heading.text,
            "anchor": heading.anchor,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_anchors(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md anchors — {} headings", doc.outline.len())?;
    if doc.outline.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for heading in &doc.outline {
        writeln!(
            out,
            "h{} #{} {}",
            heading.level, heading.anchor, heading.text
        )?;
    }
    Ok(())
}

fn write_anchors_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "anchors": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "anchor": heading.anchor,
            "text": heading.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_stats(
    doc: &MarkdownDocument,
    source: &str,
    source_path: Option<&str>,
    width_cells: u16,
    out: &mut impl Write,
) -> Result<()> {
    writeln!(out, "kittui-md stats")?;
    writeln!(out, "source.bytes={}", source.len())?;
    writeln!(out, "source.lines={}", source.lines().count())?;
    writeln!(out, "source.path={}", source_path.unwrap_or("<stdin>"))?;
    writeln!(out, "render.width_cells={width_cells}")?;
    write_count_lines(doc, out)
}

fn write_stats_json(
    doc: &MarkdownDocument,
    source: &str,
    source_path: Option<&str>,
    width_cells: u16,
    out: &mut impl Write,
) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "source": {
            "bytes": source.len(),
            "lines": source.lines().count(),
            "path": source_path.unwrap_or("<stdin>"),
        },
        "render": {
            "mode": "stats-json",
            "width_cells": width_cells,
        },
        "counts": metadata_counts(doc),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_counts(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md counts")?;
    write_count_lines(doc, out)
}

fn write_count_lines(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "components={}", doc.components.len())?;
    writeln!(out, "headings={}", doc.outline.len())?;
    writeln!(out, "heading_anchors={}", doc.outline.len())?;
    writeln!(out, "links={}", doc.links.len())?;
    writeln!(out, "images={}", doc.images.len())?;
    writeln!(out, "tables={}", doc.tables.len())?;
    writeln!(out, "footnote_references={}", doc.footnote_references.len())?;
    writeln!(out, "footnotes={}", doc.footnotes.len())?;
    writeln!(out, "definitions={}", doc.definitions.len())?;
    writeln!(out, "math={}", doc.math.len())?;
    writeln!(out, "html={}", doc.html.len())?;
    writeln!(out, "metadata_blocks={}", doc.metadata_blocks.len())?;
    writeln!(out, "code_blocks={}", doc.code_blocks.len())?;
    Ok(())
}

fn write_links(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md links — {} links", doc.links.len())?;
    if doc.links.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, link) in doc.links.iter().enumerate() {
        writeln!(out, "link #{}", i + 1)?;
        writeln!(out, "  label={}", link.label)?;
        writeln!(out, "  url={}", link.url)?;
        if let Some(title) = &link.title {
            writeln!(out, "  title={title}")?;
        }
    }
    Ok(())
}

fn write_links_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_footnotes(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let total = doc.footnote_references.len() + doc.footnotes.len();
    writeln!(out, "kittui-md footnotes — {total} entries")?;
    if total == 0 {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "definitions:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    Ok(())
}

fn write_footnotes_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "references": doc.footnote_references.iter().enumerate().map(|(index, label)| serde_json::json!({
            "index": index,
            "label": label,
        })).collect::<Vec<_>>(),
        "definitions": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_images(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md images — {} images", doc.images.len())?;
    if doc.images.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, image) in doc.images.iter().enumerate() {
        writeln!(out, "image #{}", i + 1)?;
        writeln!(out, "  alt={}", image.alt)?;
        writeln!(out, "  url={}", image.url)?;
        if let Some(title) = &image.title {
            writeln!(out, "  title={title}")?;
        }
    }
    Ok(())
}

fn write_images_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_tables(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md tables — {} tables", doc.tables.len())?;
    if doc.tables.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, table) in doc.tables.iter().enumerate() {
        let footprint = table.footprint();
        writeln!(out, "table #{}", i + 1)?;
        writeln!(out, "  rows={}", table.rows.len())?;
        writeln!(out, "  columns={}", table.column_widths().len())?;
        writeln!(out, "  column_widths={:?}", table.column_widths())?;
        writeln!(
            out,
            "  alignments={:?}",
            table
                .alignments
                .iter()
                .map(|alignment| alignment.as_str())
                .collect::<Vec<_>>()
        )?;
        writeln!(out, "  footprint={}x{}", footprint.cols, footprint.rows)?;
        for row in &table.rows {
            writeln!(out, "  | {} |", row.join(" | "))?;
        }
    }
    Ok(())
}

fn write_tables_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "tables": doc.tables.iter().enumerate().map(|(index, table)| {
            let footprint = table.footprint();
            serde_json::json!({
                "index": index,
                "rows": table.rows,
                "alignments": table.alignments.iter().map(|alignment| alignment.as_str()).collect::<Vec<_>>(),
                "column_widths": table.column_widths(),
                "footprint": {
                    "cols": footprint.cols,
                    "rows": footprint.rows,
                },
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_html(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md html — {} fragments", doc.html.len())?;
    if doc.html.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, html) in doc.html.iter().enumerate() {
        writeln!(out, "html #{}", i + 1)?;
        writeln!(out, "  kind={}", html.kind.as_str())?;
        writeln!(out, "  source={}", html.source)?;
    }
    Ok(())
}

fn write_html_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "html": doc.html.iter().enumerate().map(|(index, html)| serde_json::json!({
            "index": index,
            "kind": html.kind.as_str(),
            "source": html.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_math(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(out, "kittui-md math — {} expressions", doc.math.len())?;
    if doc.math.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, math) in doc.math.iter().enumerate() {
        writeln!(out, "math #{}", i + 1)?;
        writeln!(out, "  kind={}", math.kind.as_str())?;
        writeln!(out, "  source={}", math.source)?;
    }
    Ok(())
}

fn write_math_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "math": doc.math.iter().enumerate().map(|(index, math)| serde_json::json!({
            "index": index,
            "kind": math.kind.as_str(),
            "source": math.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_definitions(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md definitions — {} definitions",
        doc.definitions.len()
    )?;
    if doc.definitions.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, definition) in doc.definitions.iter().enumerate() {
        writeln!(out, "definition #{}", i + 1)?;
        writeln!(out, "  term={}", definition.term)?;
        writeln!(out, "  definition={}", definition.definition)?;
    }
    Ok(())
}

fn write_definitions_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "definitions": doc.definitions.iter().enumerate().map(|(index, definition)| serde_json::json!({
            "index": index,
            "term": definition.term,
            "definition": definition.definition,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_code_blocks(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md code blocks — {} code blocks",
        doc.code_blocks.len()
    )?;
    if doc.code_blocks.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, block) in doc.code_blocks.iter().enumerate() {
        writeln!(out, "code block #{}", i + 1)?;
        writeln!(
            out,
            "  language={}",
            block.language.as_deref().unwrap_or("<plain>")
        )?;
        writeln!(out, "---")?;
        writeln!(out, "{}", block.text)?;
        writeln!(out, "---")?;
    }
    Ok(())
}

fn write_code_blocks_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "code_blocks": doc.code_blocks.iter().enumerate().map(|(index, block)| serde_json::json!({
            "index": index,
            "language": block.language,
            "text": block.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_metadata_blocks(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md metadata blocks — {} metadata blocks",
        doc.metadata_blocks.len()
    )?;
    if doc.metadata_blocks.is_empty() {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    for (i, metadata) in doc.metadata_blocks.iter().enumerate() {
        writeln!(out, "metadata block #{}", i + 1)?;
        writeln!(out, "  kind={}", metadata.kind.as_str())?;
        writeln!(out, "---")?;
        writeln!(out, "{}", metadata.source)?;
        writeln!(out, "---")?;
    }
    Ok(())
}

fn write_metadata_blocks_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "metadata_blocks": doc.metadata_blocks.iter().enumerate().map(|(index, metadata)| serde_json::json!({
            "index": index,
            "kind": metadata.kind.as_str(),
            "source": metadata.source,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_references(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let total =
        doc.links.len() + doc.images.len() + doc.footnote_references.len() + doc.footnotes.len();
    writeln!(out, "kittui-md references — {total} entries")?;
    if total == 0 {
        writeln!(out, "<empty>")?;
        return Ok(());
    }
    if !doc.links.is_empty() {
        writeln!(out, "links:")?;
        for link in &doc.links {
            if let Some(title) = &link.title {
                writeln!(out, "  [{}] {} \"{}\"", link.label, link.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", link.label, link.url)?;
            }
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "images:")?;
        for image in &doc.images {
            if let Some(title) = &image.title {
                writeln!(out, "  [{}] {} \"{}\"", image.alt, image.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", image.alt, image.url)?;
            }
        }
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "footnote references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "footnotes:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    Ok(())
}

fn write_references_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
        "footnote_references": doc.footnote_references.iter().enumerate().map(|(index, label)| serde_json::json!({
            "index": index,
            "label": label,
        })).collect::<Vec<_>>(),
        "footnotes": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn metadata_counts(doc: &MarkdownDocument) -> serde_json::Value {
    serde_json::json!({
        "components": doc.components.len(),
        "headings": doc.outline.len(),
        "heading_anchors": doc.outline.len(),
        "links": doc.links.len(),
        "images": doc.images.len(),
        "tables": doc.tables.len(),
        "footnote_references": doc.footnote_references.len(),
        "footnotes": doc.footnotes.len(),
        "definitions": doc.definitions.len(),
        "math": doc.math.len(),
        "html": doc.html.len(),
        "metadata_blocks": doc.metadata_blocks.len(),
        "code_blocks": doc.code_blocks.len(),
    })
}

fn write_counts_json(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "counts": metadata_counts(doc),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_metadata_json(
    doc: &MarkdownDocument,
    source: &str,
    width_cells: u16,
    source_path: Option<&str>,
    out: &mut impl Write,
) -> Result<()> {
    let value = serde_json::json!({
        "schema_version": 1,
        "source": {
            "bytes": source.len(),
            "lines": source.lines().count(),
            "path": source_path,
        },
        "render": {
            "mode": "metadata-json",
            "width_cells": width_cells,
        },
        "counts": metadata_counts(doc),
        "components": doc.components.len(),
        "components_detail": doc.components.iter().enumerate().map(|(index, component)| serde_json::json!({
            "index": index,
            "kind": format!("{:?}", component.kind),
            "text": component.text,
            "width_cells": component.width_cells,
            "height_cells": component.height_cells,
        })).collect::<Vec<_>>(),
        "links": doc.links.iter().enumerate().map(|(index, link)| serde_json::json!({
            "index": index,
            "label": link.label,
            "url": link.url,
            "title": link.title,
        })).collect::<Vec<_>>(),
        "images": doc.images.iter().enumerate().map(|(index, image)| serde_json::json!({
            "index": index,
            "alt": image.alt,
            "url": image.url,
            "title": image.title,
        })).collect::<Vec<_>>(),
        "footnote_references": doc.footnote_references,
        "footnotes": doc.footnotes.iter().enumerate().map(|(index, footnote)| serde_json::json!({
            "index": index,
            "label": footnote.label,
            "text": footnote.text,
        })).collect::<Vec<_>>(),
        "definitions": doc.definitions.iter().enumerate().map(|(index, definition)| serde_json::json!({
            "index": index,
            "term": definition.term,
            "definition": definition.definition,
        })).collect::<Vec<_>>(),
        "math": doc.math.iter().enumerate().map(|(index, math)| serde_json::json!({
            "index": index,
            "kind": math.kind.as_str(),
            "source": math.source,
        })).collect::<Vec<_>>(),
        "html": doc.html.iter().enumerate().map(|(index, html)| serde_json::json!({
            "index": index,
            "kind": html.kind.as_str(),
            "source": html.source,
        })).collect::<Vec<_>>(),
        "metadata_blocks": doc.metadata_blocks.iter().enumerate().map(|(index, metadata)| serde_json::json!({
            "index": index,
            "kind": metadata.kind.as_str(),
            "source": metadata.source,
        })).collect::<Vec<_>>(),
        "code_blocks": doc.code_blocks.iter().enumerate().map(|(index, code)| serde_json::json!({
            "index": index,
            "language": code.language,
            "text": code.text,
        })).collect::<Vec<_>>(),
        "outline": doc.outline.iter().enumerate().map(|(index, heading)| serde_json::json!({
            "index": index,
            "level": heading.level,
            "text": heading.text,
            "anchor": heading.anchor,
        })).collect::<Vec<_>>(),
        "tables": doc.tables.iter().enumerate().map(|(index, table)| {
            let footprint = table.footprint();
            serde_json::json!({
                "index": index,
                "rows": table.rows,
                "alignments": table.alignments.iter().map(|alignment| alignment.as_str()).collect::<Vec<_>>(),
                "column_widths": table.column_widths(),
                "footprint": {
                    "cols": footprint.cols,
                    "rows": footprint.rows,
                },
            })
        }).collect::<Vec<_>>(),
    });
    serde_json::to_writer_pretty(&mut *out, &value)?;
    writeln!(out)?;
    Ok(())
}

fn write_plain(doc: &MarkdownDocument, width: u16, out: &mut impl Write) -> Result<()> {
    writeln!(
        out,
        "kittui-md — {} components, {} links, {} images",
        doc.components.len(),
        doc.links.len(),
        doc.images.len()
    )?;
    writeln!(out, "{}", "═".repeat(width as usize))?;
    for comp in &doc.components {
        write_plain_component(out, comp)?;
    }
    write_metadata_sections(doc, out)?;
    Ok(())
}

fn write_plain_component(out: &mut impl Write, comp: &UiComponent) -> Result<()> {
    let prefix = format!("[{:?}] ", comp.kind);
    let continuation = " ".repeat(prefix.len());
    let mut lines = comp.text.lines();
    if let Some(first) = lines.next() {
        writeln!(out, "{prefix}{first}")?;
        for line in lines {
            writeln!(out, "{continuation}{line}")?;
        }
    } else {
        writeln!(out, "{prefix}")?;
    }
    Ok(())
}

fn write_metadata_sections(doc: &MarkdownDocument, out: &mut impl Write) -> Result<()> {
    if !doc.outline.is_empty() {
        writeln!(out, "\noutline:")?;
        for line in outline_lines(doc) {
            writeln!(out, "  {line}")?;
        }
    }
    if !doc.links.is_empty() {
        writeln!(out, "\nlinks:")?;
        for link in &doc.links {
            if let Some(title) = &link.title {
                writeln!(out, "  [{}] {} \"{}\"", link.label, link.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", link.label, link.url)?;
            }
        }
    }
    if !doc.images.is_empty() {
        writeln!(out, "\nimages:")?;
        for image in &doc.images {
            if let Some(title) = &image.title {
                writeln!(out, "  [{}] {} \"{}\"", image.alt, image.url, title)?;
            } else {
                writeln!(out, "  [{}] {}", image.alt, image.url)?;
            }
        }
    }
    if !doc.footnote_references.is_empty() {
        writeln!(out, "\nfootnote references:")?;
        for label in &doc.footnote_references {
            writeln!(out, "  [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        writeln!(out, "\nfootnotes:")?;
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    if !doc.definitions.is_empty() {
        writeln!(out, "\ndefinitions:")?;
        for definition in &doc.definitions {
            writeln!(out, "  {} — {}", definition.term, definition.definition)?;
        }
    }
    if !doc.math.is_empty() {
        writeln!(out, "\nmath:")?;
        for math in &doc.math {
            writeln!(out, "  {} {}", math.kind.as_str(), math.source)?;
        }
    }
    if !doc.html.is_empty() {
        writeln!(out, "\nhtml:")?;
        for html in &doc.html {
            writeln!(out, "  {} {}", html.kind.as_str(), html.source)?;
        }
    }
    if !doc.metadata_blocks.is_empty() {
        writeln!(out, "\nmetadata blocks:")?;
        for metadata in &doc.metadata_blocks {
            writeln!(
                out,
                "  {} {}",
                metadata.kind.as_str(),
                metadata.source.lines().next().unwrap_or("")
            )?;
        }
    }
    if !doc.code_blocks.is_empty() {
        writeln!(out, "\ncode blocks:")?;
        for code in &doc.code_blocks {
            writeln!(
                out,
                "  {} {}",
                code.language.as_deref().unwrap_or("<plain>"),
                code.text.lines().next().unwrap_or("")
            )?;
        }
    }
    Ok(())
}

fn write_rich(doc: &MarkdownDocument, cfg: &Config, out: &mut impl Write) -> Result<()> {
    let layout = layout_components(&doc.components, &doc.tables, cfg.width);
    let visible = visible_components(&layout, cfg.offset_rows, cfg.height_rows);
    let cell = CellSize::default();
    let runtime = Runtime::builder().renderer(RendererKind::Cpu).build()?;

    write!(out, "\x1b[?25l")?;
    for item in &visible {
        let local_rect = CellRect::new(
            0,
            item.rect.y.saturating_sub(cfg.offset_rows),
            item.rect.cols,
            item.rect.rows,
        );
        let scene = component_scene(item.component, local_rect, cell);
        let placed = runtime.place(&scene)?;
        write!(
            out,
            "{}{}{}",
            placed.upload,
            placed.placement,
            kittui_kitty::cursor_move(local_rect.x, local_rect.y, Transport::Direct)
        )?;
        write!(out, "{}", placed.embed)?;
        if let Some(table_index) = item.table_index {
            if let Some(table) = doc.tables.get(table_index) {
                write_table_glyphs(out, &runtime, table, placed.image_id, local_rect, cell)?;
                write_table_text(out, table, local_rect)?;
                continue;
            }
        }
        write_component_text(out, item.component, local_rect)?;
    }
    let footer_y = visible
        .last()
        .map(|item| {
            item.rect
                .y
                .saturating_sub(cfg.offset_rows)
                .saturating_add(item.rect.rows)
                .saturating_add(1)
        })
        .unwrap_or(0);
    write!(
        out,
        "{}",
        kittui_kitty::cursor_move(0, footer_y, Transport::Direct)
    )?;
    writeln!(
        out,
        "\x1b[0m\x1b[?25h{}",
        rich_status_line(doc, cfg, document_rows(doc, cfg.width))
    )?;
    if !doc.outline.is_empty() {
        writeln!(out, "  outline:")?;
        for line in outline_lines(doc) {
            writeln!(out, "    {line}")?;
        }
    }
    if !doc.links.is_empty() {
        for link in &doc.links {
            writeln!(out, "  🔗 {} — {}", link.label, link.url)?;
        }
    }
    if !doc.images.is_empty() {
        for image in &doc.images {
            writeln!(out, "  🖼  {} — {}", image.alt, image.url)?;
        }
    }
    if !doc.footnote_references.is_empty() {
        for label in &doc.footnote_references {
            writeln!(out, "  ↩ [^{label}]")?;
        }
    }
    if !doc.footnotes.is_empty() {
        for footnote in &doc.footnotes {
            writeln!(out, "  [^{}] {}", footnote.label, footnote.text)?;
        }
    }
    if !doc.definitions.is_empty() {
        for definition in &doc.definitions {
            writeln!(out, "  📖 {} — {}", definition.term, definition.definition)?;
        }
    }
    if !doc.math.is_empty() {
        for math in &doc.math {
            writeln!(out, "  ∑ {} — {}", math.kind.as_str(), math.source)?;
        }
    }
    if !doc.html.is_empty() {
        for html in &doc.html {
            writeln!(out, "  HTML {} — {}", html.kind.as_str(), html.source)?;
        }
    }
    if !doc.code_blocks.is_empty() {
        for code in &doc.code_blocks {
            writeln!(
                out,
                "  code {} — {}",
                code.language.as_deref().unwrap_or("<plain>"),
                code.text.lines().next().unwrap_or("")
            )?;
        }
    }
    Ok(())
}

fn outline_lines(doc: &MarkdownDocument) -> Vec<String> {
    doc.outline
        .iter()
        .map(|heading| {
            format!(
                "{}{} #{}",
                "  ".repeat(heading.level.saturating_sub(1) as usize),
                heading.text,
                heading.anchor
            )
        })
        .collect()
}

fn rich_status_line(doc: &MarkdownDocument, cfg: &Config, total_rows: u16) -> String {
    let viewport = cfg.height_rows.unwrap_or(total_rows);
    let max_offset = total_rows.saturating_sub(viewport);
    format!(
        "kittui-md rich view — {} components, {} headings, {} heading anchors, {} links, {} images, {} tables, {} footnote refs, {} footnotes, {} definitions, {} math, {} html, {} metadata blocks, {} code blocks; offset={}/{} rows; viewport={}; total_rows={}",
        doc.components.len(),
        doc.outline.len(),
        doc.outline.len(),
        doc.links.len(),
        doc.images.len(),
        doc.tables.len(),
        doc.footnote_references.len(),
        doc.footnotes.len(),
        doc.definitions.len(),
        doc.math.len(),
        doc.html.len(),
        doc.metadata_blocks.len(),
        doc.code_blocks.len(),
        cfg.offset_rows.min(max_offset),
        max_offset,
        viewport,
        total_rows,
    )
}

fn layout_components<'a>(
    components: &'a [UiComponent],
    tables: &'a [MarkdownTable],
    width: u16,
) -> Vec<LaidOutComponent<'a>> {
    let mut y = 0;
    let mut table_index = 0usize;
    let mut out = Vec::with_capacity(components.len());
    for component in components {
        let is_table =
            component.kind == ComponentKind::TextBox && component.text.starts_with("table\n");
        let current_table = if is_table {
            let idx = table_index;
            table_index += 1;
            Some(idx)
        } else {
            None
        };
        let table_rows = current_table
            .and_then(|idx| tables.get(idx))
            .map(|table| table.footprint().rows.saturating_add(2));
        let rows = table_rows.unwrap_or_else(|| component.height_cells.max(1));
        let cols = component.width_cells.min(width).max(1);
        out.push(LaidOutComponent {
            component,
            rect: CellRect::new(0, y, cols, rows),
            table_index: current_table,
        });
        y = y.saturating_add(rows).saturating_add(1);
    }
    out
}

fn visible_components<'a>(
    layout: &'a [LaidOutComponent<'a>],
    offset_rows: u16,
    height_rows: Option<u16>,
) -> Vec<&'a LaidOutComponent<'a>> {
    let end = height_rows.map(|h| offset_rows.saturating_add(h));
    layout
        .iter()
        .filter(|item| {
            let bottom = item.rect.y.saturating_add(item.rect.rows);
            bottom > offset_rows && end.is_none_or(|end| item.rect.y < end)
        })
        .collect()
}

fn component_scene(component: &UiComponent, rect: CellRect, cell: CellSize) -> Scene {
    let mut layers = Vec::new();
    match component.kind {
        ComponentKind::H1 | ComponentKind::Title => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(102, 92, 255, 235),
            Rgba::rgba(18, 214, 196, 220),
        )),
        ComponentKind::H2 | ComponentKind::Header => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(60, 130, 255, 220),
            Rgba::rgba(27, 32, 54, 210),
        )),
        ComponentKind::H3 | ComponentKind::Footer => layers.push(background_linear(
            rect,
            cell,
            Direction::Horizontal,
            Rgba::rgba(145, 105, 255, 210),
            Rgba::rgba(27, 32, 54, 205),
        )),
        ComponentKind::TextChip => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(65, 76, 125, 230),
            Rgba::rgba(176, 196, 255, 240),
            1.0,
            8.0,
        )),
        ComponentKind::Banner => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(80, 62, 35, 230),
            Rgba::rgba(255, 195, 92, 245),
            1.0,
            6.0,
        )),
        ComponentKind::TextBox => layers.push(rounded_rect(
            rect.to_pixels(cell),
            Rgba::rgba(20, 24, 38, 220),
            Rgba::rgba(95, 116, 170, 230),
            1.0,
            6.0,
        )),
    }
    scene(rect, cell, layers)
}

fn write_table_glyphs(
    out: &mut impl Write,
    runtime: &Runtime,
    table: &MarkdownTable,
    anchor_image_id: u32,
    rect: CellRect,
    cell: CellSize,
) -> Result<()> {
    let layout =
        TableGlyphLayout::from_table(anchor_image_id, table).with_background(anchor_image_id);
    let fg = Rgba::rgba(176, 220, 255, 245);
    for (i, glyph_cell) in layout.cells.iter().enumerate() {
        let scene = box_glyph_scene(glyph_cell.glyph, fg, cell);
        let placed = runtime.place(&scene)?;
        let mut options = glyph_cell.placement.clone();
        options.placement_id = Some(10_000 + i as u32);
        if let Some(relative) = &mut options.relative {
            relative.image_id = anchor_image_id;
        }
        let command = kittui_kitty::placement_command_ex(
            placed.image_id,
            CellRect::new(rect.x, rect.y, 1, 1),
            &options,
            Transport::Direct,
        );
        write!(out, "{}{}", placed.upload, command)?;
    }
    Ok(())
}

fn write_table_text(out: &mut impl Write, table: &MarkdownTable, rect: CellRect) -> Result<()> {
    let widths = table.column_widths();
    for (row_idx, row) in table.rows.iter().enumerate() {
        let y = rect.y.saturating_add(1 + row_idx as u16 * 2);
        let mut x = rect.x.saturating_add(2);
        for (col_idx, cell) in row.iter().enumerate() {
            write!(
                out,
                "{}\x1b[37m{}\x1b[0m",
                kittui_kitty::cursor_move(x, y, Transport::Direct),
                align_table_cell_text(
                    cell,
                    widths.get(col_idx).copied().unwrap_or(1) as usize,
                    table
                        .alignments
                        .get(col_idx)
                        .copied()
                        .unwrap_or(MarkdownTableAlignment::None),
                )
            )?;
            x = x
                .saturating_add(widths.get(col_idx).copied().unwrap_or(1))
                .saturating_add(3);
        }
    }
    Ok(())
}

fn align_table_cell_text(text: &str, width: usize, alignment: MarkdownTableAlignment) -> String {
    let truncated = truncate_cells(text, width);
    let len = truncated.chars().count();
    if len >= width {
        return truncated;
    }
    let pad = width - len;
    match alignment {
        MarkdownTableAlignment::Right => format!("{}{}", " ".repeat(pad), truncated),
        MarkdownTableAlignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{}{}", " ".repeat(left), truncated, " ".repeat(right))
        }
        MarkdownTableAlignment::None | MarkdownTableAlignment::Left => {
            format!("{}{}", truncated, " ".repeat(pad))
        }
    }
}

fn write_component_text(
    out: &mut impl Write,
    component: &UiComponent,
    rect: CellRect,
) -> Result<()> {
    let x = if matches!(component.kind, ComponentKind::TextChip) {
        1
    } else {
        2
    };
    let style = match component.kind {
        ComponentKind::H1 | ComponentKind::Title => "\x1b[1;97m",
        ComponentKind::H2 | ComponentKind::Header => "\x1b[1;96m",
        ComponentKind::H3 | ComponentKind::Footer => "\x1b[1;95m",
        ComponentKind::TextChip => "\x1b[1;94m",
        ComponentKind::Banner => "\x1b[1;93m",
        ComponentKind::TextBox => "\x1b[37m",
    };
    let max_cols = rect.cols.saturating_sub(x + 1) as usize;
    let max_rows = if matches!(component.kind, ComponentKind::TextChip) {
        1
    } else {
        rect.rows.saturating_sub(1).max(1) as usize
    };
    let start_y = if max_rows == 1 {
        rect.y.saturating_add(rect.rows / 2)
    } else {
        rect.y.saturating_add(1)
    };
    for (i, line) in wrap_text_lines(&component.text, max_cols, max_rows)
        .iter()
        .enumerate()
    {
        write!(
            out,
            "{}{style}{}\x1b[0m",
            kittui_kitty::cursor_move(
                rect.x.saturating_add(x),
                start_y.saturating_add(i as u16),
                Transport::Direct
            ),
            line
        )?;
    }
    Ok(())
}

fn wrap_text_lines(text: &str, max_cols: usize, max_rows: usize) -> Vec<String> {
    if max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut current = String::new();
        for word in raw.split_whitespace() {
            let word_len = word.chars().count();
            let current_len = current.chars().count();
            if current_len == 0 {
                current = truncate_cells(word, max_cols);
            } else if current_len + 1 + word_len <= max_cols {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                if lines.len() == max_rows {
                    return lines;
                }
                current = truncate_cells(word, max_cols);
            }
        }
        if !current.is_empty() || raw.is_empty() {
            lines.push(current);
            if lines.len() == max_rows {
                return lines;
            }
        }
    }
    lines
}

fn truncate_cells(s: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in s.chars().take(max) {
        out.push(ch);
    }
    out
}

fn terminal_rows() -> Option<u16> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_row > 0 {
        Some(ws.ws_row)
    } else {
        None
    }
}

fn terminal_cols() -> Option<u16> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 {
        Some(ws.ws_col)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittui_affordances::{
        h1, h2, textbox, HeadingOutline, LinkChip, MarkdownCodeBlock, MarkdownDefinition,
        MarkdownFootnote, MarkdownImage, MarkdownMath, MarkdownMathKind, MarkdownMetadataBlock,
        MarkdownMetadataBlockKind, Tone,
    };

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn parse_args_rejects_multiple_output_modes() {
        let err = parse_args(["--plain".to_string(), "--outline".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--plain"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
    }

    #[test]
    fn parse_args_accepts_single_output_mode() {
        let cfg = parse_args(["--components".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Components);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_mode_selector_for_canonical_name() {
        let cfg = parse_args([
            "--mode".to_string(),
            "components-json".to_string(),
            "doc.md".to_string(),
        ])
        .unwrap();
        assert_eq!(cfg.mode, Mode::ComponentsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_mode_selector_for_flag_name() {
        let cfg = parse_args([
            "--mode".to_string(),
            "--stats-json".to_string(),
            "doc.md".to_string(),
        ])
        .unwrap();
        assert_eq!(cfg.mode, Mode::StatsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_mode_selector_for_no_input_discovery_mode() {
        let cfg = parse_args(["--mode".to_string(), "keybindings-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::KeybindingsJson);
        assert_eq!(cfg.path, None);
    }

    #[test]
    fn parse_args_accepts_mode_selector_for_alias_name() {
        let cfg = parse_args([
            "--mode".to_string(),
            "widgets".to_string(),
            "doc.md".to_string(),
        ])
        .unwrap();
        assert_eq!(cfg.mode, Mode::Components);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_unknown_mode_selector() {
        let err =
            parse_args(["--mode".to_string(), "definitely-not-a-mode".to_string()]).unwrap_err();
        assert!(err.to_string().contains("unknown --mode value"), "{err}");
        assert!(err.to_string().contains("definitely-not-a-mode"), "{err}");
    }

    #[test]
    fn parse_args_rejects_mode_selector_plus_direct_mode() {
        let err = parse_args([
            "--mode".to_string(),
            "components".to_string(),
            "--plain".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--components"), "{err}");
        assert!(err.to_string().contains("--plain"), "{err}");
    }

    #[test]
    fn parse_args_accepts_widgets_alias() {
        let cfg = parse_args(["--widgets".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Components);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_components_json_mode() {
        let cfg = parse_args(["--components-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ComponentsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_components_plus_components_json() {
        let err =
            parse_args(["--components".to_string(), "--components-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--components"), "{err}");
        assert!(err.to_string().contains("--components-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_components_plus_widgets() {
        let err = parse_args(["--components".to_string(), "--widgets".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--components"), "{err}");
        assert!(err.to_string().contains("--widgets"), "{err}");
    }

    #[test]
    fn parse_args_accepts_modes_mode() {
        let cfg = parse_args(["--modes".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Modes);
        assert_eq!(cfg.path.as_deref(), None);
    }

    #[test]
    fn parse_args_accepts_modes_json_mode() {
        let cfg = parse_args(["--modes-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModesJson);
        assert_eq!(cfg.path.as_deref(), None);
    }

    #[test]
    fn parse_args_accepts_schemas_json_mode() {
        let cfg = parse_args(["--schemas-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::SchemasJson);
        assert_eq!(cfg.path.as_deref(), None);
    }

    #[test]
    fn parse_args_accepts_mode_info_mode() {
        let cfg = parse_args(["--mode-info".to_string(), "widgets".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeInfo);
        assert_eq!(cfg.mode_info_name.as_deref(), Some("widgets"));
    }

    #[test]
    fn parse_args_accepts_mode_info_json_mode() {
        let cfg = parse_args(["--mode-info-json".to_string(), "--stats-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeInfoJson);
        assert_eq!(cfg.mode_info_name.as_deref(), Some("--stats-json"));
    }

    #[test]
    fn parse_args_rejects_missing_mode_info_value() {
        let err = parse_args(["--mode-info".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("--mode-info requires a value"),
            "{err}"
        );
    }

    #[test]
    fn parse_args_accepts_mode_search_mode() {
        let cfg = parse_args(["--mode-search".to_string(), "json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeSearch);
        assert_eq!(cfg.mode_search_query.as_deref(), Some("json"));
    }

    #[test]
    fn parse_args_accepts_mode_search_json_mode() {
        let cfg = parse_args(["--mode-search-json".to_string(), "table".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeSearchJson);
        assert_eq!(cfg.mode_search_query.as_deref(), Some("table"));
    }

    #[test]
    fn parse_args_rejects_missing_mode_search_value() {
        let err = parse_args(["--mode-search".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("--mode-search requires a value"),
            "{err}"
        );
    }

    #[test]
    fn parse_args_rejects_mode_search_plus_modes() {
        let err = parse_args([
            "--mode-search".to_string(),
            "json".to_string(),
            "--modes".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--mode-search"), "{err}");
        assert!(err.to_string().contains("--modes"), "{err}");
    }

    #[test]
    fn parse_args_accepts_mode_category_mode() {
        let cfg = parse_args(["--mode-category".to_string(), "json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeCategory);
        assert_eq!(cfg.mode_category_name.as_deref(), Some("json"));
    }

    #[test]
    fn parse_args_accepts_mode_category_json_mode() {
        let cfg = parse_args(["--mode-category-json".to_string(), "inspect".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeCategoryJson);
        assert_eq!(cfg.mode_category_name.as_deref(), Some("inspect"));
    }

    #[test]
    fn parse_args_rejects_missing_mode_category_value() {
        let err = parse_args(["--mode-category".to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("--mode-category requires a value"),
            "{err}"
        );
    }

    #[test]
    fn parse_args_rejects_mode_category_plus_modes() {
        let err = parse_args([
            "--mode-category".to_string(),
            "json".to_string(),
            "--modes".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--mode-category"), "{err}");
        assert!(err.to_string().contains("--modes"), "{err}");
    }

    #[test]
    fn parse_args_accepts_mode_categories_mode() {
        let cfg = parse_args(["--mode-categories".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeCategories);
    }

    #[test]
    fn parse_args_accepts_mode_categories_json_mode() {
        let cfg = parse_args(["--mode-categories-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ModeCategoriesJson);
    }

    #[test]
    fn parse_args_rejects_mode_categories_plus_modes() {
        let err = parse_args(["--mode-categories".to_string(), "--modes".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--mode-categories"), "{err}");
        assert!(err.to_string().contains("--modes"), "{err}");
    }

    #[test]
    fn parse_args_accepts_about_mode() {
        let cfg = parse_args(["--about".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::About);
    }

    #[test]
    fn parse_args_accepts_about_json_mode() {
        let cfg = parse_args(["--about-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::AboutJson);
    }

    #[test]
    fn parse_args_rejects_about_plus_about_json() {
        let err = parse_args(["--about".to_string(), "--about-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--about"), "{err}");
        assert!(err.to_string().contains("--about-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_capabilities_mode() {
        let cfg = parse_args(["--capabilities".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Capabilities);
    }

    #[test]
    fn parse_args_accepts_capabilities_json_mode() {
        let cfg = parse_args(["--capabilities-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CapabilitiesJson);
    }

    #[test]
    fn parse_args_rejects_capabilities_plus_capabilities_json() {
        let err = parse_args([
            "--capabilities".to_string(),
            "--capabilities-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--capabilities"), "{err}");
        assert!(err.to_string().contains("--capabilities-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_version_mode() {
        let cfg = parse_args(["--version".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Version);
    }

    #[test]
    fn parse_args_accepts_version_json_mode() {
        let cfg = parse_args(["--version-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::VersionJson);
    }

    #[test]
    fn parse_args_rejects_version_plus_version_json() {
        let err = parse_args(["--version".to_string(), "--version-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--version"), "{err}");
        assert!(err.to_string().contains("--version-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_input_formats_mode() {
        let cfg = parse_args(["--input-formats".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::InputFormats);
    }

    #[test]
    fn parse_args_accepts_input_formats_json_mode() {
        let cfg = parse_args(["--input-formats-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::InputFormatsJson);
    }

    #[test]
    fn parse_args_rejects_input_formats_plus_input_formats_json() {
        let err = parse_args([
            "--input-formats".to_string(),
            "--input-formats-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--input-formats"), "{err}");
        assert!(err.to_string().contains("--input-formats-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_output_formats_mode() {
        let cfg = parse_args(["--output-formats".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::OutputFormats);
    }

    #[test]
    fn parse_args_accepts_output_formats_json_mode() {
        let cfg = parse_args(["--output-formats-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::OutputFormatsJson);
    }

    #[test]
    fn parse_args_rejects_output_formats_plus_output_formats_json() {
        let err = parse_args([
            "--output-formats".to_string(),
            "--output-formats-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--output-formats"), "{err}");
        assert!(err.to_string().contains("--output-formats-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_defaults_mode() {
        let cfg = parse_args(["--defaults".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Defaults);
    }

    #[test]
    fn parse_args_accepts_defaults_json_mode() {
        let cfg = parse_args(["--defaults-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::DefaultsJson);
    }

    #[test]
    fn parse_args_rejects_defaults_plus_defaults_json() {
        let err =
            parse_args(["--defaults".to_string(), "--defaults-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--defaults"), "{err}");
        assert!(err.to_string().contains("--defaults-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_examples_mode() {
        let cfg = parse_args(["--examples".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Examples);
    }

    #[test]
    fn parse_args_accepts_examples_json_mode() {
        let cfg = parse_args(["--examples-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ExamplesJson);
    }

    #[test]
    fn parse_args_rejects_examples_plus_examples_json() {
        let err =
            parse_args(["--examples".to_string(), "--examples-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--examples"), "{err}");
        assert!(err.to_string().contains("--examples-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_limits_mode() {
        let cfg = parse_args(["--limits".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Limits);
    }

    #[test]
    fn parse_args_accepts_limits_json_mode() {
        let cfg = parse_args(["--limits-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::LimitsJson);
    }

    #[test]
    fn parse_args_rejects_limits_plus_limits_json() {
        let err = parse_args(["--limits".to_string(), "--limits-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--limits"), "{err}");
        assert!(err.to_string().contains("--limits-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_keybindings_mode() {
        let cfg = parse_args(["--keybindings".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Keybindings);
    }

    #[test]
    fn parse_args_accepts_keybindings_json_mode() {
        let cfg = parse_args(["--keybindings-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::KeybindingsJson);
    }

    #[test]
    fn parse_args_rejects_keybindings_plus_keybindings_json() {
        let err = parse_args([
            "--keybindings".to_string(),
            "--keybindings-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--keybindings"), "{err}");
        assert!(err.to_string().contains("--keybindings-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_exit_codes_mode() {
        let cfg = parse_args(["--exit-codes".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ExitCodes);
    }

    #[test]
    fn parse_args_accepts_exit_codes_json_mode() {
        let cfg = parse_args(["--exit-codes-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ExitCodesJson);
    }

    #[test]
    fn parse_args_rejects_exit_codes_plus_exit_codes_json() {
        let err =
            parse_args(["--exit-codes".to_string(), "--exit-codes-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--exit-codes"), "{err}");
        assert!(err.to_string().contains("--exit-codes-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_mode_selector_for_about_json() {
        let cfg = parse_args(["--mode".to_string(), "about-json".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::AboutJson);
    }

    #[test]
    fn parse_args_rejects_modes_plus_schemas_json() {
        let err = parse_args(["--modes".to_string(), "--schemas-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--modes"), "{err}");
        assert!(err.to_string().contains("--schemas-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_modes_plus_modes_json() {
        let err = parse_args(["--modes".to_string(), "--modes-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--modes"), "{err}");
        assert!(err.to_string().contains("--modes-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_toc_alias() {
        let cfg = parse_args(["--toc".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Outline);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_headings_alias() {
        let cfg = parse_args(["--headings".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Outline);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_outline_json_mode() {
        let cfg = parse_args(["--outline-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::OutlineJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_outline_plus_outline_json() {
        let err = parse_args(["--outline".to_string(), "--outline-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--outline-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_anchors_mode() {
        let cfg = parse_args(["--anchors".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Anchors);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_slugs_alias() {
        let cfg = parse_args(["--slugs".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Anchors);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_anchors_json_mode() {
        let cfg = parse_args(["--anchors-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::AnchorsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_anchors_plus_anchors_json() {
        let err = parse_args(["--anchors".to_string(), "--anchors-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--anchors"), "{err}");
        assert!(err.to_string().contains("--anchors-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_anchors_plus_slugs() {
        let err = parse_args(["--anchors".to_string(), "--slugs".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--anchors"), "{err}");
        assert!(err.to_string().contains("--slugs"), "{err}");
    }

    #[test]
    fn parse_args_accepts_json_alias() {
        let cfg = parse_args(["--json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_summary_alias() {
        let cfg = parse_args(["--summary".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Stats);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_stats_json_mode() {
        let cfg = parse_args(["--stats-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::StatsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_stats_plus_stats_json() {
        let err = parse_args(["--stats".to_string(), "--stats-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
        assert!(err.to_string().contains("--stats-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_counts_mode() {
        let cfg = parse_args(["--counts".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Counts);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_counts_json_mode() {
        let cfg = parse_args(["--counts-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CountsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_counts_plus_counts_json() {
        let err = parse_args(["--counts".to_string(), "--counts-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--counts"), "{err}");
        assert!(err.to_string().contains("--counts-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_counts_plus_stats() {
        let err = parse_args(["--counts".to_string(), "--stats".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--counts"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
    }

    #[test]
    fn parse_args_accepts_refs_alias() {
        let cfg = parse_args(["--refs".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::References);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_references_json_mode() {
        let cfg = parse_args(["--references-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ReferencesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_references_plus_references_json() {
        let err =
            parse_args(["--references".to_string(), "--references-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--references"), "{err}");
        assert!(err.to_string().contains("--references-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_snippets_alias() {
        let cfg = parse_args(["--snippets".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CodeBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_code_blocks_json_mode() {
        let cfg = parse_args(["--code-blocks-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::CodeBlocksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_code_blocks_plus_code_blocks_json() {
        let err = parse_args([
            "--code-blocks".to_string(),
            "--code-blocks-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--code-blocks"), "{err}");
        assert!(err.to_string().contains("--code-blocks-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_glossary_alias() {
        let cfg = parse_args(["--glossary".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Definitions);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_definitions_json_mode() {
        let cfg = parse_args(["--definitions-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::DefinitionsJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_definitions_plus_definitions_json() {
        let err = parse_args([
            "--definitions".to_string(),
            "--definitions-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--definitions"), "{err}");
        assert!(err.to_string().contains("--definitions-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_markup_alias() {
        let cfg = parse_args(["--markup".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Html);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_html_json_mode() {
        let cfg = parse_args(["--html-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::HtmlJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_html_plus_html_json() {
        let err = parse_args(["--html".to_string(), "--html-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--html"), "{err}");
        assert!(err.to_string().contains("--html-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_equations_alias() {
        let cfg = parse_args(["--equations".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Math);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_math_json_mode() {
        let cfg = parse_args(["--math-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MathJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_math_plus_math_json() {
        let err = parse_args(["--math".to_string(), "--math-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--math"), "{err}");
        assert!(err.to_string().contains("--math-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_math_plus_equations() {
        let err = parse_args(["--math".to_string(), "--equations".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--math"), "{err}");
        assert!(err.to_string().contains("--equations"), "{err}");
    }

    #[test]
    fn parse_args_rejects_html_plus_markup() {
        let err = parse_args(["--html".to_string(), "--markup".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--html"), "{err}");
        assert!(err.to_string().contains("--markup"), "{err}");
    }

    #[test]
    fn parse_args_accepts_pictures_alias() {
        let cfg = parse_args(["--pictures".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Images);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_images_json_mode() {
        let cfg = parse_args(["--images-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::ImagesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_images_plus_images_json() {
        let err = parse_args(["--images".to_string(), "--images-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--images"), "{err}");
        assert!(err.to_string().contains("--images-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_grid_alias() {
        let cfg = parse_args(["--grid".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Tables);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_tables_json_mode() {
        let cfg = parse_args(["--tables-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::TablesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_tables_plus_tables_json() {
        let err = parse_args(["--tables".to_string(), "--tables-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--tables"), "{err}");
        assert!(err.to_string().contains("--tables-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_tables_plus_grid() {
        let err = parse_args(["--tables".to_string(), "--grid".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--tables"), "{err}");
        assert!(err.to_string().contains("--grid"), "{err}");
    }

    #[test]
    fn parse_args_accepts_urls_alias() {
        let cfg = parse_args(["--urls".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Links);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_links_json_mode() {
        let cfg = parse_args(["--links-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::LinksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_links_plus_links_json() {
        let err = parse_args(["--links".to_string(), "--links-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--links"), "{err}");
        assert!(err.to_string().contains("--links-json"), "{err}");
    }

    #[test]
    fn parse_args_accepts_notes_alias() {
        let cfg = parse_args(["--notes".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::Footnotes);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_footnotes_json_mode() {
        let cfg = parse_args(["--footnotes-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::FootnotesJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_footnotes_plus_footnotes_json() {
        let err =
            parse_args(["--footnotes".to_string(), "--footnotes-json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--footnotes"), "{err}");
        assert!(err.to_string().contains("--footnotes-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_footnotes_plus_notes() {
        let err = parse_args(["--footnotes".to_string(), "--notes".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--footnotes"), "{err}");
        assert!(err.to_string().contains("--notes"), "{err}");
    }

    #[test]
    fn parse_args_rejects_links_plus_urls() {
        let err = parse_args(["--links".to_string(), "--urls".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--links"), "{err}");
        assert!(err.to_string().contains("--urls"), "{err}");
    }

    #[test]
    fn parse_args_rejects_images_plus_pictures() {
        let err = parse_args(["--images".to_string(), "--pictures".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--images"), "{err}");
        assert!(err.to_string().contains("--pictures"), "{err}");
    }

    #[test]
    fn parse_args_rejects_definitions_plus_glossary() {
        let err = parse_args(["--definitions".to_string(), "--glossary".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--definitions"), "{err}");
        assert!(err.to_string().contains("--glossary"), "{err}");
    }

    #[test]
    fn parse_args_rejects_code_blocks_plus_snippets() {
        let err = parse_args(["--code-blocks".to_string(), "--snippets".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--code-blocks"), "{err}");
        assert!(err.to_string().contains("--snippets"), "{err}");
    }

    #[test]
    fn parse_args_rejects_references_plus_refs() {
        let err = parse_args(["--references".to_string(), "--refs".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--references"), "{err}");
        assert!(err.to_string().contains("--refs"), "{err}");
    }

    #[test]
    fn parse_args_rejects_stats_plus_summary() {
        let err = parse_args(["--stats".to_string(), "--summary".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--stats"), "{err}");
        assert!(err.to_string().contains("--summary"), "{err}");
    }

    #[test]
    fn parse_args_rejects_metadata_json_plus_json() {
        let err = parse_args(["--metadata-json".to_string(), "--json".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-json"), "{err}");
        assert!(err.to_string().contains("--json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_outline_plus_toc() {
        let err = parse_args(["--outline".to_string(), "--toc".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--toc"), "{err}");
    }

    #[test]
    fn parse_args_rejects_outline_plus_headings() {
        let err = parse_args(["--outline".to_string(), "--headings".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--outline"), "{err}");
        assert!(err.to_string().contains("--headings"), "{err}");
    }

    #[test]
    fn parse_args_accepts_metadata_blocks_mode() {
        let cfg = parse_args(["--metadata-blocks".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_frontmatter_alias() {
        let cfg = parse_args(["--frontmatter".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_metadata_alias() {
        let cfg = parse_args(["--metadata".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocks);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_accepts_metadata_blocks_json_mode() {
        let cfg = parse_args(["--metadata-blocks-json".to_string(), "doc.md".to_string()]).unwrap();
        assert_eq!(cfg.mode, Mode::MetadataBlocksJson);
        assert_eq!(cfg.path.as_deref(), Some("doc.md"));
    }

    #[test]
    fn parse_args_rejects_metadata_blocks_plus_metadata_blocks_json() {
        let err = parse_args([
            "--metadata-blocks".to_string(),
            "--metadata-blocks-json".to_string(),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks-json"), "{err}");
    }

    #[test]
    fn parse_args_rejects_frontmatter_plus_metadata_blocks() {
        let err =
            parse_args(["--metadata-blocks".to_string(), "--frontmatter".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--frontmatter"), "{err}");
    }

    #[test]
    fn parse_args_rejects_metadata_plus_metadata_blocks() {
        let err =
            parse_args(["--metadata-blocks".to_string(), "--metadata".to_string()]).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
        assert!(err.to_string().contains("--metadata-blocks"), "{err}");
        assert!(err.to_string().contains("--metadata"), "{err}");
    }

    #[test]
    fn layout_stacks_components_with_gaps() {
        let comps = vec![h1("Title", 40), h1("Next", 40)];
        let layout = layout_components(&comps, &[], 80);
        assert_eq!(layout[0].rect.y, 0);
        assert_eq!(layout[1].rect.y, 4);
    }

    #[test]
    fn layout_marks_table_components() {
        let tables = vec![MarkdownTable::new(vec![vec!["A".into(), "B".into()]])];
        let comps = vec![textbox("table\nA | B", 40, Tone::Assistant)];
        let layout = layout_components(&comps, &tables, 80);
        assert_eq!(layout[0].table_index, Some(0));
        assert!(layout[0].rect.rows > 2);
    }

    #[test]
    fn pager_actions_clamp_to_document() {
        assert_eq!(apply_pager_action(0, 10, 30, PagerAction::Down), 1);
        assert_eq!(apply_pager_action(3, 10, 30, PagerAction::PageUp), 0);
        assert_eq!(apply_pager_action(0, 10, 30, PagerAction::PageDown), 10);
        assert_eq!(apply_pager_action(19, 10, 30, PagerAction::Down), 20);
        assert_eq!(apply_pager_action(20, 10, 30, PagerAction::Down), 20);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Home), 0);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::End), 20);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Help), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Outline), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Links), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Images), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Tables), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::CodeBlocks), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Footnotes), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Definitions), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Math), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Html), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::Reload), 4);
        assert_eq!(apply_pager_action(4, 10, 30, PagerAction::ClearStatus), 4);
    }

    #[test]
    fn pager_reads_outline_key() {
        let mut cursor = std::io::Cursor::new(b"o".as_slice());
        assert_eq!(
            read_pager_action(&mut cursor).unwrap(),
            PagerAction::Outline
        );
    }

    #[test]
    fn pager_reads_links_key() {
        let mut cursor = std::io::Cursor::new(b"l".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Links);
    }

    #[test]
    fn pager_reads_images_key() {
        let mut cursor = std::io::Cursor::new(b"i".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Images);
    }

    #[test]
    fn pager_reads_tables_key() {
        let mut cursor = std::io::Cursor::new(b"t".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Tables);
    }

    #[test]
    fn pager_reads_code_blocks_key() {
        let mut cursor = std::io::Cursor::new(b"s".as_slice());
        assert_eq!(
            read_pager_action(&mut cursor).unwrap(),
            PagerAction::CodeBlocks
        );
    }

    #[test]
    fn pager_reads_footnotes_key() {
        let mut cursor = std::io::Cursor::new(b"f".as_slice());
        assert_eq!(
            read_pager_action(&mut cursor).unwrap(),
            PagerAction::Footnotes
        );
    }

    #[test]
    fn pager_reads_definitions_key() {
        let mut cursor = std::io::Cursor::new(b"d".as_slice());
        assert_eq!(
            read_pager_action(&mut cursor).unwrap(),
            PagerAction::Definitions
        );
    }

    #[test]
    fn pager_reads_math_key() {
        let mut cursor = std::io::Cursor::new(b"m".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Math);
    }

    #[test]
    fn pager_reads_html_key() {
        let mut cursor = std::io::Cursor::new(b"x".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Html);
    }

    #[test]
    fn pager_reads_clear_status_key() {
        let mut cursor = std::io::Cursor::new(b"c".as_slice());
        assert_eq!(
            read_pager_action(&mut cursor).unwrap(),
            PagerAction::ClearStatus
        );
    }

    #[test]
    fn pager_reads_reload_key() {
        let mut cursor = std::io::Cursor::new(b"r".as_slice());
        assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Reload);
    }

    #[test]
    fn pager_reads_help_keys() {
        for bytes in [b"h".as_slice(), b"?".as_slice()] {
            let mut cursor = std::io::Cursor::new(bytes);
            assert_eq!(read_pager_action(&mut cursor).unwrap(), PagerAction::Help);
        }
    }

    #[test]
    fn interactive_help_lists_keybindings() {
        let mut out = Vec::new();
        write_interactive_help(40, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md interactive help"),
            "{rendered}"
        );
        assert!(rendered.contains("help: h, ?"), "{rendered}");
        assert!(rendered.contains("outline: o"), "{rendered}");
        assert!(rendered.contains("links: l"), "{rendered}");
        assert!(rendered.contains("images: i"), "{rendered}");
        assert!(rendered.contains("tables: t"), "{rendered}");
        assert!(rendered.contains("code-blocks: s"), "{rendered}");
        assert!(rendered.contains("footnotes: f"), "{rendered}");
        assert!(rendered.contains("definitions: d"), "{rendered}");
        assert!(rendered.contains("math: m"), "{rendered}");
        assert!(rendered.contains("html: x"), "{rendered}");
        assert!(rendered.contains("reload: r"), "{rendered}");
        assert!(rendered.contains("clear-status: c"), "{rendered}");
        assert!(rendered.contains("quit: q, Ctrl-C"), "{rendered}");
    }

    #[test]
    fn interactive_outline_lists_headings() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                    anchor: "section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_outline(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md outline — 2 headings"),
            "{rendered}"
        );
        assert!(rendered.contains("Title #title"), "{rendered}");
        assert!(rendered.contains("Section #section"), "{rendered}");
    }

    #[test]
    fn interactive_links_lists_links() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![
                LinkChip {
                    label: "site".to_string(),
                    url: "https://example.com".to_string(),
                    title: Some("Example".to_string()),
                },
                LinkChip {
                    label: "docs".to_string(),
                    url: "docs/readme.md".to_string(),
                    title: None,
                },
            ],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_links(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md links — 2 links"), "{rendered}");
        assert!(
            rendered.contains("[site] https://example.com \"Example\""),
            "{rendered}"
        );
        assert!(rendered.contains("[docs] docs/readme.md"), "{rendered}");
    }

    #[test]
    fn interactive_images_lists_images() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![
                MarkdownImage {
                    alt: "diagram".to_string(),
                    url: "diagram.png".to_string(),
                    title: Some("System diagram".to_string()),
                },
                MarkdownImage {
                    alt: "logo".to_string(),
                    url: "logo.svg".to_string(),
                    title: None,
                },
            ],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_images(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md images — 2 images"),
            "{rendered}"
        );
        assert!(
            rendered.contains("![diagram] diagram.png \"System diagram\""),
            "{rendered}"
        );
        assert!(rendered.contains("![logo] logo.svg"), "{rendered}");
    }

    #[test]
    fn interactive_tables_lists_table_summaries() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![MarkdownTable::with_alignments(
                vec![
                    vec!["Name".to_string(), "Value".to_string()],
                    vec!["Alpha".to_string(), "1".to_string()],
                ],
                vec![MarkdownTableAlignment::Left, MarkdownTableAlignment::Right],
            )],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_tables(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md tables — 1 tables"),
            "{rendered}"
        );
        assert!(
            rendered.contains("table #0: rows=2 columns=2"),
            "{rendered}"
        );
        assert!(rendered.contains("alignments=left,right"), "{rendered}");
    }

    #[test]
    fn interactive_code_blocks_lists_snippet_summaries() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![
                MarkdownCodeBlock {
                    language: Some("rust".to_string()),
                    text: "fn main() {}".to_string(),
                },
                MarkdownCodeBlock {
                    language: None,
                    text: "line one\nline two".to_string(),
                },
            ],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_code_blocks(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md code blocks — 2 blocks"),
            "{rendered}"
        );
        assert!(
            rendered.contains("code #0: language=rust lines=1 preview=fn main() {}"),
            "{rendered}"
        );
        assert!(
            rendered.contains("code #1: language=plain lines=2 preview=line one"),
            "{rendered}"
        );
    }

    #[test]
    fn interactive_footnotes_lists_references_and_definitions() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![MarkdownFootnote {
                label: "note".to_string(),
                text: "Footnote body".to_string(),
            }],
            footnote_references: vec!["note".to_string()],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_footnotes(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md footnotes — 1 references, 1 definitions"),
            "{rendered}"
        );
        assert!(rendered.contains("ref [^note]"), "{rendered}");
        assert!(
            rendered.contains("def [^note]: Footnote body"),
            "{rendered}"
        );
    }

    #[test]
    fn interactive_footnotes_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_interactive_footnotes(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md footnotes — 0 references, 0 definitions"),
            "{rendered}"
        );
        assert!(rendered.contains("<empty>"), "{rendered}");
    }

    #[test]
    fn interactive_definitions_lists_definition_entries() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![MarkdownDefinition {
                term: "Term".to_string(),
                definition: "Definition body".to_string(),
            }],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_definitions(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md definitions — 1 definitions"),
            "{rendered}"
        );
        assert!(rendered.contains("Term: Definition body"), "{rendered}");
    }

    #[test]
    fn interactive_math_lists_math_expressions() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![MarkdownMath {
                kind: MarkdownMathKind::Inline,
                source: "a+b".to_string(),
            }],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_math(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md math — 1 expressions"),
            "{rendered}"
        );
        assert!(rendered.contains("inline: a+b"), "{rendered}");
    }

    #[test]
    fn interactive_html_lists_html_fragments() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![kittui_affordances::MarkdownHtml {
                kind: kittui_affordances::MarkdownHtmlKind::Inline,
                source: "<kbd>x</kbd>".to_string(),
            }],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_interactive_html(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md html — 1 fragments"),
            "{rendered}"
        );
        assert!(rendered.contains("inline: <kbd>x</kbd>"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_reload_status() {
        let mut out = Vec::new();
        write_interactive_footer(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some("reloaded doc.md — 12 rows"),
            "doc.md",
            4,
            10,
            30,
            &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("source: doc.md"), "{rendered}");
        assert!(rendered.contains("offset 4/20"), "{rendered}");
        assert!(rendered.contains("viewport 10"), "{rendered}");
        assert!(rendered.contains("rows 30"), "{rendered}");
        assert!(rendered.contains("status: reloaded doc.md"), "{rendered}");
        assert!(rendered.contains("r reload"), "{rendered}");
        assert!(rendered.contains("c clear status"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("f footnotes"), "{rendered}");
        assert!(rendered.contains("d definitions"), "{rendered}");
        assert!(rendered.contains("m math"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_omits_status_when_cleared() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, false, false, false, false, false, None, "doc.md",
            0, 10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(!rendered.contains("status:"), "{rendered}");
        assert!(rendered.contains("source: doc.md"), "{rendered}");
        assert!(rendered.contains("c clear status"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_reload_error_status() {
        let mut out = Vec::new();
        write_interactive_footer(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some("reload failed: missing"),
            "doc.md",
            99,
            10,
            30,
            &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("offset 20/20"), "{rendered}");
        assert!(
            rendered.contains("status: reload failed: missing"),
            "{rendered}"
        );
        assert!(rendered.contains("h/? close help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_outline_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, true, false, false, false, false, false, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("o close outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_links_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, true, false, false, false, false, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("l close links"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_images_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, true, false, false, false, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("i close images"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_tables_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, true, false, false, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("t close tables"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_code_blocks_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, true, false, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("s close code"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("f footnotes"), "{rendered}");
        assert!(rendered.contains("d definitions"), "{rendered}");
        assert!(rendered.contains("m math"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_footnotes_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, false, true, false, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("f close footnotes"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("d definitions"), "{rendered}");
        assert!(rendered.contains("m math"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_definitions_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, false, false, true, false, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("d close definitions"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("f footnotes"), "{rendered}");
        assert!(rendered.contains("m math"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_math_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, false, false, false, true, false, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("m close math"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("f footnotes"), "{rendered}");
        assert!(rendered.contains("d definitions"), "{rendered}");
        assert!(rendered.contains("x html"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn interactive_footer_writes_html_controls() {
        let mut out = Vec::new();
        write_interactive_footer(
            false, false, false, false, false, false, false, false, false, true, None, "doc.md", 0,
            10, 30, &mut out,
        )
        .unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("x close html"), "{rendered}");
        assert!(rendered.contains("o outline"), "{rendered}");
        assert!(rendered.contains("l links"), "{rendered}");
        assert!(rendered.contains("i images"), "{rendered}");
        assert!(rendered.contains("t tables"), "{rendered}");
        assert!(rendered.contains("s code"), "{rendered}");
        assert!(rendered.contains("f footnotes"), "{rendered}");
        assert!(rendered.contains("d definitions"), "{rendered}");
        assert!(rendered.contains("m math"), "{rendered}");
        assert!(rendered.contains("h/? help"), "{rendered}");
    }

    #[test]
    fn reload_interactive_document_reads_latest_file() {
        let path = std::env::temp_dir().join(format!(
            "kittui-md-reload-{}-{}.md",
            std::process::id(),
            unique_test_suffix()
        ));
        std::fs::write(&path, "# First\n").unwrap();
        let first = reload_interactive_document(path.to_str().unwrap(), 80).unwrap();
        assert_eq!(first.outline[0].text, "First");
        std::fs::write(&path, "# Second\n").unwrap();
        let second = reload_interactive_document(path.to_str().unwrap(), 80).unwrap();
        assert_eq!(second.outline[0].text, "Second");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pager_reads_arrow_and_page_key_escape_sequences() {
        let cases = [
            (b"\x1b[A".as_slice(), PagerAction::Up),
            (b"\x1b[B".as_slice(), PagerAction::Down),
            (b"\x1b[5~".as_slice(), PagerAction::PageUp),
            (b"\x1b[6~".as_slice(), PagerAction::PageDown),
            (b"\x1b[H".as_slice(), PagerAction::Home),
            (b"\x1b[F".as_slice(), PagerAction::End),
            (b"\x1bOH".as_slice(), PagerAction::Home),
            (b"\x1bOF".as_slice(), PagerAction::End),
        ];
        for (bytes, action) in cases {
            let mut cursor = std::io::Cursor::new(bytes);
            assert_eq!(read_pager_action(&mut cursor).unwrap(), action);
        }
    }

    #[test]
    fn document_rows_reports_bottom_edge() {
        let doc = MarkdownDocument {
            components: vec![h1("One", 40), h1("Two", 40)],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        assert_eq!(document_rows(&doc, 80), 7);
    }

    #[test]
    fn rich_status_line_reports_offset_viewport_and_total_rows() {
        let doc = MarkdownDocument {
            components: vec![h1("One", 40), h1("Two", 40)],
            links: vec![],
            tables: vec![MarkdownTable::new(vec![vec!["A".into()]])],
            images: vec![MarkdownImage {
                alt: "logo".to_string(),
                url: "logo.png".to_string(),
                title: None,
            }],
            outline: vec![HeadingOutline {
                level: 1,
                text: "Title".to_string(),
                anchor: "title".to_string(),
            }],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Yaml,
                source: "title: Proof".to_string(),
            }],
        };
        let cfg = Config {
            mode: Mode::Rich,
            width: 80,
            offset_rows: 99,
            height_rows: Some(3),
            interactive: true,
            path: Some("proof.md".to_string()),
            mode_info_name: None,
            mode_search_query: None,
            mode_category_name: None,
        };
        let status = rich_status_line(&doc, &cfg, document_rows(&doc, 80));
        assert!(status.contains("offset=4/4 rows"), "{status}");
        assert!(status.contains("viewport=3"), "{status}");
        assert!(status.contains("total_rows=7"), "{status}");
        assert!(
            status.contains("1 headings, 1 heading anchors, 0 links, 1 images, 1 tables, 0 footnote refs, 0 footnotes, 0 definitions, 0 math, 0 html, 1 metadata blocks, 0 code blocks"),
            "{status}"
        );
    }

    #[test]
    fn html_mode_writes_kind_and_source() {
        let doc = render_markdown("hello <kbd>x</kbd>\n\n<div>block</div>", 80);
        let mut out = Vec::new();
        write_html(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md html — 3 fragments"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #1\n  kind=inline\n  source=<kbd>"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #2\n  kind=inline\n  source=</kbd>"),
            "{rendered}"
        );
        assert!(
            rendered.contains("html #3\n  kind=block\n  source=<div>block</div>"),
            "{rendered}"
        );
    }

    #[test]
    fn html_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_html(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md html — 0 fragments\n<empty>\n"
        );
    }

    #[test]
    fn html_json_mode_writes_html_records() {
        let doc = render_markdown("hello <kbd>x</kbd>\n\n<div>block</div>", 80);
        let mut out = Vec::new();
        write_html_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["html"][0]["index"], 0);
        assert_eq!(value["html"][0]["kind"], "inline");
        assert_eq!(value["html"][0]["source"], "<kbd>");
        assert_eq!(value["html"][2]["index"], 2);
        assert_eq!(value["html"][2]["kind"], "block");
        assert_eq!(value["html"][2]["source"], "<div>block</div>");
    }

    #[test]
    fn math_mode_writes_kind_and_source() {
        let doc = render_markdown("inline $x + y$\n\n$$\na^2\n$$", 80);
        let mut out = Vec::new();
        write_math(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md math — 2 expressions"),
            "{rendered}"
        );
        assert!(
            rendered.contains("math #1\n  kind=inline\n  source=x + y"),
            "{rendered}"
        );
        assert!(
            rendered.contains("math #2\n  kind=display\n  source=a^2"),
            "{rendered}"
        );
    }

    #[test]
    fn math_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_math(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md math — 0 expressions\n<empty>\n"
        );
    }

    #[test]
    fn math_json_mode_writes_math_records() {
        let doc = render_markdown("inline $x + y$\n\n$$\na^2\n$$", 80);
        let mut out = Vec::new();
        write_math_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["math"][0]["index"], 0);
        assert_eq!(value["math"][0]["kind"], "inline");
        assert_eq!(value["math"][0]["source"], "x + y");
        assert_eq!(value["math"][1]["index"], 1);
        assert_eq!(value["math"][1]["kind"], "display");
        assert_eq!(value["math"][1]["source"], "a^2");
    }

    #[test]
    fn definitions_mode_writes_terms_and_definitions() {
        let doc = render_markdown("Term\n: Definition text", 80);
        let mut out = Vec::new();
        write_definitions(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md definitions — 1 definitions"),
            "{rendered}"
        );
        assert!(rendered.contains("definition #1"), "{rendered}");
        assert!(rendered.contains("term=Term"), "{rendered}");
        assert!(
            rendered.contains("definition=Definition text"),
            "{rendered}"
        );
    }

    #[test]
    fn definitions_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_definitions(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md definitions — 0 definitions\n<empty>\n"
        );
    }

    #[test]
    fn definitions_json_mode_writes_definition_records() {
        let doc = render_markdown("Term\n: Definition text", 80);
        let mut out = Vec::new();
        write_definitions_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["term"], "Term");
        assert_eq!(value["definitions"][0]["definition"], "Definition text");
    }

    #[test]
    fn code_blocks_mode_writes_language_and_source() {
        let doc = render_markdown("```rust\nfn main() {}\n```\n\n```\nplain\n```", 80);
        let mut out = Vec::new();
        write_code_blocks(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md code blocks — 2 code blocks"),
            "{rendered}"
        );
        assert!(
            rendered.contains("code block #1\n  language=rust"),
            "{rendered}"
        );
        assert!(rendered.contains("fn main() {}"), "{rendered}");
        assert!(
            rendered.contains("code block #2\n  language=<plain>"),
            "{rendered}"
        );
        assert!(rendered.contains("plain"), "{rendered}");
    }

    #[test]
    fn code_blocks_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_code_blocks(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md code blocks — 0 code blocks\n<empty>\n"
        );
    }

    #[test]
    fn code_blocks_json_mode_writes_code_block_records() {
        let doc = render_markdown("```rust\nfn main() {}\n```\n\n```\nplain\n```", 80);
        let mut out = Vec::new();
        write_code_blocks_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["code_blocks"][0]["index"], 0);
        assert_eq!(value["code_blocks"][0]["language"], "rust");
        assert_eq!(value["code_blocks"][0]["text"], "fn main() {}");
        assert_eq!(value["code_blocks"][1]["index"], 1);
        assert_eq!(value["code_blocks"][1]["language"], serde_json::Value::Null);
        assert_eq!(value["code_blocks"][1]["text"], "plain");
    }

    #[test]
    fn metadata_blocks_mode_writes_kind_and_source() {
        let doc = MarkdownDocument {
            metadata_blocks: vec![MarkdownMetadataBlock {
                kind: MarkdownMetadataBlockKind::Yaml,
                source: "title: Proof".to_string(),
            }],
            ..MarkdownDocument::default()
        };
        let mut out = Vec::new();
        write_metadata_blocks(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md metadata blocks — 1 metadata blocks"),
            "{rendered}"
        );
        assert!(rendered.contains("metadata block #1"), "{rendered}");
        assert!(rendered.contains("kind=yaml"), "{rendered}");
        assert!(rendered.contains("title: Proof"), "{rendered}");
    }

    #[test]
    fn metadata_blocks_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_metadata_blocks(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md metadata blocks — 0 metadata blocks\n<empty>\n"
        );
    }

    #[test]
    fn metadata_blocks_json_mode_writes_metadata_block_records() {
        let doc = render_markdown("---\ntitle: Proof\n---\n\n# Body", 80);
        let mut out = Vec::new();
        write_metadata_blocks_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["metadata_blocks"][0]["index"], 0);
        assert_eq!(value["metadata_blocks"][0]["kind"], "yaml");
        assert_eq!(value["metadata_blocks"][0]["source"], "title: Proof");
    }

    #[test]
    fn links_mode_writes_label_url_and_title() {
        let doc = render_markdown("See [site](https://example.com \"Example title\")", 80);
        let mut out = Vec::new();
        write_links(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md links — 1 links"), "{rendered}");
        assert!(rendered.contains("link #1"), "{rendered}");
        assert!(rendered.contains("label=site"), "{rendered}");
        assert!(rendered.contains("url=https://example.com"), "{rendered}");
        assert!(rendered.contains("title=Example title"), "{rendered}");
    }

    #[test]
    fn links_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_links(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md links — 0 links\n<empty>\n"
        );
    }

    #[test]
    fn links_json_mode_writes_link_records() {
        let doc = render_markdown("See [site](https://example.com \"Example title\")", 80);
        let mut out = Vec::new();
        write_links_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["label"], "site");
        assert_eq!(value["links"][0]["url"], "https://example.com");
        assert_eq!(value["links"][0]["title"], "Example title");
    }

    #[test]
    fn footnotes_mode_writes_references_and_definitions() {
        let doc = render_markdown("see[^n]\n\n[^n]: note text", 80);
        let mut out = Vec::new();
        write_footnotes(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md footnotes — 2 entries"),
            "{rendered}"
        );
        assert!(rendered.contains("references:\n  [^n]"), "{rendered}");
        assert!(
            rendered.contains("definitions:\n  [^n] note text"),
            "{rendered}"
        );
    }

    #[test]
    fn footnotes_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_footnotes(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md footnotes — 0 entries\n<empty>\n"
        );
    }

    #[test]
    fn footnotes_json_mode_writes_references_and_definitions() {
        let doc = render_markdown("see[^n]\n\n[^n]: note text", 80);
        let mut out = Vec::new();
        write_footnotes_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["references"][0]["index"], 0);
        assert_eq!(value["references"][0]["label"], "n");
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["label"], "n");
        assert_eq!(value["definitions"][0]["text"], "note text");
    }

    #[test]
    fn images_mode_writes_alt_url_and_title() {
        let doc = render_markdown("![logo](logo.png \"Logo title\")", 80);
        let mut out = Vec::new();
        write_images(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md images — 1 images"),
            "{rendered}"
        );
        assert!(rendered.contains("image #1"), "{rendered}");
        assert!(rendered.contains("alt=logo"), "{rendered}");
        assert!(rendered.contains("url=logo.png"), "{rendered}");
        assert!(rendered.contains("title=Logo title"), "{rendered}");
    }

    #[test]
    fn images_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_images(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md images — 0 images\n<empty>\n"
        );
    }

    #[test]
    fn images_json_mode_writes_image_records() {
        let doc = render_markdown("![logo](logo.png \"Logo title\")", 80);
        let mut out = Vec::new();
        write_images_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["alt"], "logo");
        assert_eq!(value["images"][0]["url"], "logo.png");
        assert_eq!(value["images"][0]["title"], "Logo title");
    }

    #[test]
    fn tables_mode_reports_table_metrics_and_rows() {
        let doc = render_markdown("| a | b |\n|:---|---:|\n| 1 | 22 |", 80);
        let mut out = Vec::new();
        write_tables(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md tables — 1 tables"),
            "{rendered}"
        );
        assert!(rendered.contains("table #1"), "{rendered}");
        assert!(rendered.contains("column_widths=[1, 2]"), "{rendered}");
        assert!(
            rendered.contains("alignments=[\"left\", \"right\"]"),
            "{rendered}"
        );
        assert!(rendered.contains("| 1 | 22 |"), "{rendered}");
    }

    #[test]
    fn tables_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_tables(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md tables — 0 tables\n<empty>\n"
        );
    }

    #[test]
    fn tables_json_mode_writes_table_records() {
        let doc = render_markdown("| a | b |\n|:---|---:|\n| 1 | 22 |", 80);
        let mut out = Vec::new();
        write_tables_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["tables"][0]["index"], 0);
        assert_eq!(value["tables"][0]["rows"][1][1], "22");
        assert_eq!(value["tables"][0]["alignments"][0], "left");
        assert_eq!(value["tables"][0]["alignments"][1], "right");
        assert_eq!(
            value["tables"][0]["column_widths"],
            serde_json::json!([1, 2])
        );
        assert!(value["tables"][0]["footprint"]["cols"].as_u64().unwrap() >= 6);
    }

    #[test]
    fn stats_mode_reports_document_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, None, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md stats\n"), "{rendered}");
        assert!(rendered.contains("source.bytes="), "{rendered}");
        assert!(rendered.contains("source.lines=3"), "{rendered}");
        assert!(rendered.contains("source.path=<stdin>"), "{rendered}");
        assert!(rendered.contains("render.width_cells=80"), "{rendered}");
        assert!(rendered.contains("headings=1"), "{rendered}");
        assert!(rendered.contains("heading_anchors=1"), "{rendered}");
        assert!(rendered.contains("links=1"), "{rendered}");
        assert!(rendered.contains("images=1"), "{rendered}");
    }

    #[test]
    fn stats_mode_reports_source_path_when_present() {
        let source = "# Title";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, Some("docs/proof.md"), 72, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("source.path=docs/proof.md"), "{rendered}");
        assert!(rendered.contains("render.width_cells=72"), "{rendered}");
    }

    #[test]
    fn stats_json_mode_reports_source_render_and_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats_json(&doc, source, Some("docs/proof.md"), 72, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["source"]["bytes"], source.len());
        assert_eq!(value["source"]["lines"], 3);
        assert_eq!(value["source"]["path"], "docs/proof.md");
        assert_eq!(value["render"]["mode"], "stats-json");
        assert_eq!(value["render"]["width_cells"], 72);
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert!(value.get("components_detail").is_none());
    }

    #[test]
    fn counts_mode_reports_counts_without_source_provenance() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_counts(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.starts_with("kittui-md counts\n"), "{rendered}");
        assert!(rendered.contains("components="), "{rendered}");
        assert!(rendered.contains("headings=1"), "{rendered}");
        assert!(rendered.contains("links=1"), "{rendered}");
        assert!(rendered.contains("images=1"), "{rendered}");
        assert!(!rendered.contains("source.path="), "{rendered}");
        assert!(!rendered.contains("render.width_cells="), "{rendered}");
    }

    #[test]
    fn counts_json_mode_reports_machine_readable_counts() {
        let source = "# Title\n\nSee [site](https://example.com) and ![logo](logo.png).";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_counts_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["heading_anchors"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert!(value.get("source").is_none());
        assert!(value.get("components_detail").is_none());
    }

    #[test]
    fn references_mode_writes_links_images_and_footnotes() {
        let doc = render_markdown(
            "See [site](https://example.com \"Example title\") and ![logo](logo.png \"Logo title\")[^n].\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        write_references(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md references — 4 entries"),
            "{rendered}"
        );
        assert!(
            rendered.contains("links:\n  [site] https://example.com \"Example title\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("images:\n  [logo] logo.png \"Logo title\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnote references:\n  [^n]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnotes:\n  [^n] note text"),
            "{rendered}"
        );
    }

    #[test]
    fn references_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_references(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md references — 0 entries\n<empty>\n"
        );
    }

    #[test]
    fn references_json_mode_writes_combined_reference_records() {
        let doc = render_markdown(
            "See [site](https://example.com \"Example title\") and ![logo](logo.png \"Logo title\")[^n].\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        write_references_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["label"], "site");
        assert_eq!(value["links"][0]["title"], "Example title");
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["alt"], "logo");
        assert_eq!(value["images"][0]["title"], "Logo title");
        assert_eq!(value["footnote_references"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["text"], "note text");
    }

    #[test]
    fn metadata_json_mode_reports_stable_shape() {
        let doc = render_markdown(
            "# Title\n\nSee [site](https://example.com) and note[^n] plus $x + y$ and <kbd>x</kbd>.\n\n```rust\nfn main() {}\n```\n\n![logo](logo.png)\n\n| a | b | c |\n|:---|:---:|---:|\n| 1 | 2 | 3 |\n\nTerm\n: Definition text\n\n[^n]: note text",
            80,
        );
        let mut out = Vec::new();
        let source = "# Title\n\nSee [site](https://example.com) and note[^n] plus $x + y$ and <kbd>x</kbd>.\n\n```rust\nfn main() {}\n```\n\n![logo](logo.png)\n\n| a | b | c |\n|:---|:---:|---:|\n| 1 | 2 | 3 |\n\nTerm\n: Definition text\n\n[^n]: note text";
        write_metadata_json(&doc, source, 80, Some("proof.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["source"]["bytes"], source.len());
        assert_eq!(value["source"]["lines"], source.lines().count());
        assert_eq!(value["source"]["path"], "proof.md");
        assert_eq!(value["render"]["mode"], "metadata-json");
        assert_eq!(value["render"]["width_cells"], 80);
        assert_eq!(
            value["components"].as_u64().unwrap(),
            doc.components.len() as u64
        );
        assert_eq!(value["counts"]["components"], doc.components.len());
        assert_eq!(value["counts"]["headings"], 1);
        assert_eq!(value["counts"]["heading_anchors"], 1);
        assert_eq!(value["counts"]["links"], 1);
        assert_eq!(value["counts"]["images"], 1);
        assert_eq!(value["counts"]["tables"], 1);
        assert_eq!(value["counts"]["footnote_references"], 1);
        assert_eq!(value["counts"]["footnotes"], 1);
        assert_eq!(value["counts"]["definitions"], 1);
        assert_eq!(value["counts"]["math"], 1);
        assert_eq!(value["counts"]["html"], 2);
        assert_eq!(value["counts"]["metadata_blocks"], 0);
        assert_eq!(value["counts"]["code_blocks"], 1);
        assert_eq!(value["components_detail"][0]["index"], 0);
        assert_eq!(value["components_detail"][0]["kind"], "H1");
        assert_eq!(value["components_detail"][0]["text"], "Title");
        assert_eq!(value["components_detail"][0]["width_cells"], 80);
        assert!(
            value["components_detail"][0]["height_cells"]
                .as_u64()
                .unwrap()
                >= 1
        );
        assert_eq!(value["outline"][0]["index"], 0);
        assert_eq!(value["outline"][0]["level"], 1);
        assert_eq!(value["outline"][0]["text"], "Title");
        assert_eq!(value["outline"][0]["anchor"], "title");
        assert_eq!(value["links"][0]["index"], 0);
        assert_eq!(value["links"][0]["url"], "https://example.com");
        assert_eq!(value["links"][0]["title"], serde_json::Value::Null);
        assert_eq!(value["images"][0]["index"], 0);
        assert_eq!(value["images"][0]["url"], "logo.png");
        assert_eq!(value["images"][0]["title"], serde_json::Value::Null);
        assert_eq!(value["footnote_references"][0], "n");
        assert_eq!(value["footnotes"][0]["index"], 0);
        assert_eq!(value["footnotes"][0]["label"], "n");
        assert_eq!(value["footnotes"][0]["text"], "note text");
        assert_eq!(value["definitions"][0]["index"], 0);
        assert_eq!(value["definitions"][0]["term"], "Term");
        assert_eq!(value["definitions"][0]["definition"], "Definition text");
        assert_eq!(value["math"][0]["index"], 0);
        assert_eq!(value["math"][0]["kind"], "inline");
        assert_eq!(value["math"][0]["source"], "x + y");
        assert_eq!(value["html"][0]["index"], 0);
        assert_eq!(value["html"][0]["kind"], "inline");
        assert_eq!(value["html"][0]["source"], "<kbd>");
        assert_eq!(value["code_blocks"][0]["index"], 0);
        assert_eq!(value["code_blocks"][0]["language"], "rust");
        assert_eq!(value["code_blocks"][0]["text"], "fn main() {}");
        assert_eq!(value["tables"][0]["index"], 0);
        assert_eq!(value["tables"][0]["rows"][1][0], "1");
        assert_eq!(value["tables"][0]["alignments"][0], "left");
        assert_eq!(value["tables"][0]["alignments"][1], "center");
        assert_eq!(value["tables"][0]["alignments"][2], "right");
        assert_eq!(
            value["tables"][0]["column_widths"],
            serde_json::json!([1, 1, 1])
        );
        assert!(value["tables"][0]["footprint"]["cols"].as_u64().unwrap() >= 10);
        assert_eq!(value["tables"][0]["footprint"]["rows"], 5);
    }

    #[test]
    fn metadata_json_mode_reports_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("frontmatter.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["metadata_blocks"][0]["index"], 0);
        assert_eq!(value["metadata_blocks"][0]["kind"], "yaml");
        assert_eq!(value["metadata_blocks"][0]["source"], "title: Proof");
    }

    #[test]
    fn metadata_json_mode_reports_link_and_image_titles() {
        let source =
            "[site](https://example.com \"Example title\")\n\n![logo](logo.png \"Logo title\")";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("titles.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["links"][0]["title"], "Example title");
        assert_eq!(value["images"][0]["title"], "Logo title");
    }

    #[test]
    fn metadata_json_mode_reports_pluses_metadata_blocks() {
        let source = "+++\ntitle = \"Proof\"\n+++\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_metadata_json(&doc, source, 80, Some("frontmatter.md"), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["metadata_blocks"][0]["kind"], "pluses");
        assert_eq!(value["metadata_blocks"][0]["source"], "title = \"Proof\"");
    }

    #[test]
    fn stats_mode_counts_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_stats(&doc, source, None, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("metadata_blocks=1"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_metadata_blocks() {
        let source = "---\ntitle: Proof\n---\n\n# Body";
        let doc = render_markdown(source, 80);
        let mut out = Vec::new();
        write_plain(&doc, 80, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("metadata blocks:\n  yaml title: Proof"),
            "{rendered}"
        );
    }

    #[test]
    fn components_mode_writes_only_component_records() {
        let doc = render_markdown("# Title\n\nSee [site](https://example.com)", 40);
        let mut out = Vec::new();
        write_components(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.starts_with("kittui-md components — "),
            "{rendered}"
        );
        assert!(rendered.contains("[H1] Title"), "{rendered}");
        assert!(rendered.contains("[TextChip] site"), "{rendered}");
        assert!(!rendered.contains("links:"), "{rendered}");
        assert!(!rendered.contains("outline:"), "{rendered}");
    }

    #[test]
    fn components_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_components(&doc, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "kittui-md components — 0 components\n<empty>\n"
        );
    }

    #[test]
    fn components_json_mode_writes_component_records() {
        let doc = render_markdown("# Title\n\nSee [site](https://example.com)", 40);
        let mut out = Vec::new();
        write_components_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["components"][0]["index"], 0);
        assert_eq!(value["components"][0]["kind"], "H1");
        assert_eq!(value["components"][0]["text"], "Title");
        assert_eq!(value["components"][0]["width_cells"], 40);
        assert_eq!(value["components"][0]["height_cells"], 3);
        assert!(value["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| { component["kind"] == "TextChip" && component["text"] == "site" }));
    }

    #[test]
    fn modes_mode_lists_flags_aliases_and_descriptions() {
        let mut out = Vec::new();
        write_modes(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.starts_with("kittui-md modes"), "{rendered}");
        assert!(rendered.contains("--components (--widgets)"), "{rendered}");
        assert!(rendered.contains("--stats-json"), "{rendered}");
        assert!(rendered.contains("available output modes"), "{rendered}");
    }

    #[test]
    fn modes_json_mode_lists_flags_aliases_and_descriptions() {
        let mut out = Vec::new();
        write_modes_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        let modes = value["modes"].as_array().unwrap();
        assert!(modes.iter().any(|mode| {
            mode["flag"] == "--components"
                && mode["aliases"] == serde_json::json!(["--widgets"])
                && mode["category"] == "inspect"
        }));
        assert!(modes
            .iter()
            .any(|mode| { mode["flag"] == "--modes-json" && mode["category"] == "discovery" }));
        assert!(modes
            .iter()
            .any(|mode| mode["description"] == "emit full document metadata as JSON"));
    }

    #[test]
    fn schemas_json_mode_lists_json_output_shapes() {
        let mut out = Vec::new();
        write_schemas_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        let schemas = value["schemas"].as_array().unwrap();
        assert!(schemas.iter().any(|schema| {
            schema["mode"] == "--metadata-json"
                && schema["top_level_keys"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("components_detail"))
        }));
        assert!(schemas.iter().any(|schema| {
            schema["mode"] == "--stats-json"
                && schema["category"] == "json"
                && schema["top_level_keys"]
                    == serde_json::json!(["schema_version", "source", "render", "counts"])
        }));
        assert!(schemas.iter().any(|schema| {
            schema["mode"] == "--mode-info-json" && schema["category"] == "discovery"
        }));
    }

    #[test]
    fn mode_info_mode_describes_alias_and_schema() {
        let mut out = Vec::new();
        write_mode_info("widgets", &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("kittui-md mode info — --components"),
            "{rendered}"
        );
        assert!(rendered.contains("aliases: --widgets"), "{rendered}");
        assert!(
            rendered.contains("json_top_level_keys: <none>"),
            "{rendered}"
        );
    }

    #[test]
    fn mode_info_json_mode_describes_json_schema() {
        let mut out = Vec::new();
        write_mode_info_json("stats-json", &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["mode"]["flag"], "--stats-json");
        assert_eq!(value["mode"]["category"], "json");
        assert_eq!(
            value["mode"]["json_schema"]["top_level_keys"],
            serde_json::json!(["schema_version", "source", "render", "counts"])
        );
    }

    #[test]
    fn mode_info_rejects_unknown_names() {
        let err = write_mode_info("not-a-real-mode", &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("unknown mode info value"), "{err}");
    }

    #[test]
    fn mode_search_mode_finds_flags_aliases_and_descriptions() {
        let mut out = Vec::new();
        write_mode_search("widget", &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("mode search"), "{rendered}");
        assert!(rendered.contains("--components (--widgets)"), "{rendered}");
    }

    #[test]
    fn mode_search_mode_reports_empty_results() {
        let mut out = Vec::new();
        write_mode_search("zzzz-no-mode", &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("0 matches"), "{rendered}");
        assert!(rendered.contains("<empty>"), "{rendered}");
    }

    #[test]
    fn mode_search_json_mode_finds_flags_aliases_and_descriptions() {
        let mut out = Vec::new();
        write_mode_search_json("table", &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["query"], "table");
        let matches = value["matches"].as_array().unwrap();
        assert!(matches.iter().any(|mode| {
            mode["flag"] == "--tables"
                && mode["category"] == "inspect"
                && mode["json_schema"] == serde_json::Value::Null
        }));
        assert!(matches.iter().any(|mode| {
            mode["flag"] == "--tables-json"
                && mode["category"] == "json"
                && mode["json_schema"]["top_level_keys"]
                    == serde_json::json!(["schema_version", "tables"])
        }));
    }

    #[test]
    fn mode_category_mode_lists_modes_in_category() {
        let mut out = Vec::new();
        write_mode_category("inspect", &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("mode category"), "{rendered}");
        assert!(rendered.contains("--components (--widgets)"), "{rendered}");
        assert!(rendered.contains("--tables"), "{rendered}");
        assert!(!rendered.contains("--tables-json"), "{rendered}");
    }

    #[test]
    fn mode_category_json_mode_lists_modes_in_category() {
        let mut out = Vec::new();
        write_mode_category_json("json", &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["category"], "json");
        let modes = value["modes"].as_array().unwrap();
        assert!(modes
            .iter()
            .any(|mode| { mode["flag"] == "--tables-json" && mode["category"] == "json" }));
        assert!(!modes.iter().any(|mode| mode["flag"] == "--tables"));
    }

    #[test]
    fn mode_category_rejects_unknown_categories() {
        let err = write_mode_category("not-a-category", &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("unknown mode category"), "{err}");
    }

    #[test]
    fn mode_categories_mode_lists_category_counts() {
        let mut out = Vec::new();
        write_mode_categories(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md mode categories"), "{rendered}");
        assert!(rendered.contains("render="), "{rendered}");
        assert!(rendered.contains("inspect="), "{rendered}");
        assert!(rendered.contains("json="), "{rendered}");
    }

    #[test]
    fn mode_categories_json_mode_lists_category_counts() {
        let mut out = Vec::new();
        write_mode_categories_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        let categories = value["categories"].as_array().unwrap();
        assert!(categories.iter().any(|category| {
            category["name"] == "json" && category["count"].as_u64().unwrap() > 0
        }));
        assert!(categories.iter().any(|category| {
            category["name"] == "inspect" && category["count"].as_u64().unwrap() > 0
        }));
    }

    #[test]
    fn about_mode_reports_version_and_capabilities() {
        let mut out = Vec::new();
        write_about(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md about"), "{rendered}");
        assert!(rendered.contains("binary=kittui-md"), "{rendered}");
        assert!(rendered.contains("package_version="), "{rendered}");
        assert!(rendered.contains("default_mode=rich"), "{rendered}");
        assert!(rendered.contains("mode-discovery"), "{rendered}");
    }

    #[test]
    fn about_json_mode_reports_version_and_capabilities() {
        let mut out = Vec::new();
        write_about_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["binary"], "kittui-md");
        assert_eq!(value["default_mode"], "rich");
        assert!(value["capabilities"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("mode-discovery")));
    }

    #[test]
    fn capabilities_mode_lists_capabilities() {
        let mut out = Vec::new();
        write_capabilities(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md capabilities"), "{rendered}");
        assert!(
            rendered.contains("rich-kitty-graphics-rendering"),
            "{rendered}"
        );
        assert!(rendered.contains("mode-discovery"), "{rendered}");
    }

    #[test]
    fn capabilities_json_mode_lists_capabilities() {
        let mut out = Vec::new();
        write_capabilities_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["capabilities"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("machine-readable-json-outputs")));
    }

    #[test]
    fn version_mode_reports_package_version() {
        let mut out = Vec::new();
        write_version(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.starts_with("kittui-md "), "{rendered}");
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")), "{rendered}");
    }

    #[test]
    fn version_json_mode_reports_package_version() {
        let mut out = Vec::new();
        write_version_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["binary"], "kittui-md");
        assert_eq!(value["package_version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn input_formats_mode_lists_markdown_format() {
        let mut out = Vec::new();
        write_input_formats(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md input formats"), "{rendered}");
        assert!(rendered.contains("markdown"), "{rendered}");
        assert!(rendered.contains("md"), "{rendered}");
    }

    #[test]
    fn input_formats_json_mode_lists_markdown_format() {
        let mut out = Vec::new();
        write_input_formats_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["input_formats"][0]["name"], "markdown");
        assert!(value["input_formats"][0]["extensions"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("md")));
    }

    #[test]
    fn output_formats_mode_lists_output_families() {
        let mut out = Vec::new();
        write_output_formats(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md output formats"), "{rendered}");
        assert!(rendered.contains("rich-kitty-graphics"), "{rendered}");
        assert!(rendered.contains("json"), "{rendered}");
    }

    #[test]
    fn output_formats_json_mode_lists_output_families() {
        let mut out = Vec::new();
        write_output_formats_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["output_formats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|format| {
                format["name"] == "json"
                    && format["mode_categories"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("json"))
            }));
    }

    #[test]
    fn defaults_mode_lists_default_settings() {
        let mut out = Vec::new();
        write_defaults(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md defaults"), "{rendered}");
        assert!(rendered.contains("mode=rich"), "{rendered}");
        assert!(rendered.contains("width.max=200"), "{rendered}");
        assert!(rendered.contains("input=stdin-or-one-file"), "{rendered}");
    }

    #[test]
    fn defaults_json_mode_lists_default_settings() {
        let mut out = Vec::new();
        write_defaults_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["defaults"]["mode"], "rich");
        assert_eq!(value["defaults"]["width"]["max"], 200);
        assert_eq!(value["defaults"]["interactive"], false);
        assert_eq!(value["defaults"]["input"], "stdin-or-one-file");
    }

    #[test]
    fn examples_mode_lists_common_invocations() {
        let mut out = Vec::new();
        write_examples(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md examples"), "{rendered}");
        assert!(rendered.contains("rich-file"), "{rendered}");
        assert!(rendered.contains("--components-json"), "{rendered}");
    }

    #[test]
    fn examples_json_mode_lists_common_invocations() {
        let mut out = Vec::new();
        write_examples_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["examples"].as_array().unwrap().iter().any(|example| {
            example["name"] == "component-json"
                && example["argv"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("--components-json"))
        }));
    }

    #[test]
    fn limits_mode_lists_numeric_limits() {
        let mut out = Vec::new();
        write_limits(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md limits"), "{rendered}");
        assert!(rendered.contains("width.max=200"), "{rendered}");
        assert!(rendered.contains("height_rows.min=1"), "{rendered}");
    }

    #[test]
    fn limits_json_mode_lists_numeric_limits() {
        let mut out = Vec::new();
        write_limits_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["limits"]["width"]["max"], 200);
        assert_eq!(value["limits"]["offset_rows"]["min"], 0);
        assert_eq!(value["limits"]["height_rows"]["min"], 1);
    }

    #[test]
    fn keybindings_mode_lists_interactive_controls() {
        let mut out = Vec::new();
        write_keybindings(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md keybindings"), "{rendered}");
        assert!(rendered.contains("scroll-up: k, w, Up"), "{rendered}");
        assert!(rendered.contains("links: l"), "{rendered}");
        assert!(rendered.contains("images: i"), "{rendered}");
        assert!(rendered.contains("tables: t"), "{rendered}");
        assert!(rendered.contains("code-blocks: s"), "{rendered}");
        assert!(rendered.contains("footnotes: f"), "{rendered}");
        assert!(rendered.contains("definitions: d"), "{rendered}");
        assert!(rendered.contains("math: m"), "{rendered}");
        assert!(rendered.contains("html: x"), "{rendered}");
        assert!(rendered.contains("reload: r"), "{rendered}");
        assert!(rendered.contains("clear-status: c"), "{rendered}");
        assert!(rendered.contains("quit: q, Ctrl-C"), "{rendered}");
    }

    #[test]
    fn keybindings_json_mode_lists_interactive_controls() {
        let mut out = Vec::new();
        write_keybindings_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "page-down"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("Space"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "links"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("l"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "images"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("i"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "tables"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("t"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "code-blocks"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("s"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "footnotes"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("f"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "definitions"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("d"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "math"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("m"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "html"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("x"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "reload"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("r"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "clear-status"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("c"))
            }));
        assert!(value["keybindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|binding| {
                binding["action"] == "quit"
                    && binding["keys"]
                        .as_array()
                        .unwrap()
                        .contains(&serde_json::json!("Ctrl-C"))
            }));
    }

    #[test]
    fn exit_codes_mode_lists_exit_codes() {
        let mut out = Vec::new();
        write_exit_codes(&mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("kittui-md exit codes"), "{rendered}");
        assert!(rendered.contains("0 success"), "{rendered}");
        assert!(rendered.contains("1 error"), "{rendered}");
    }

    #[test]
    fn exit_codes_json_mode_lists_exit_codes() {
        let mut out = Vec::new();
        write_exit_codes_json(&mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert!(value["exit_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| { code["code"] == 0 && code["name"] == "success" }));
        assert!(value["exit_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| { code["code"] == 1 && code["name"] == "error" }));
    }

    #[test]
    fn outline_mode_writes_only_heading_outline() {
        let doc = MarkdownDocument {
            components: vec![h1("Title", 40), h2("Section", 40)],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                    anchor: "section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_outline(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "kittui-md outline — 2 headings\nTitle #title\n  Section #section\n"
        );
        assert!(!rendered.contains("[H1]"));
    }

    #[test]
    fn outline_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_outline(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "kittui-md outline — 0 headings\n<empty>\n");
    }

    #[test]
    fn outline_json_mode_writes_heading_outline() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_outline_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["outline"][0]["index"], 0);
        assert_eq!(value["outline"][0]["level"], 1);
        assert_eq!(value["outline"][0]["text"], "Title");
        assert_eq!(value["outline"][0]["anchor"], "title");
        assert_eq!(value["outline"][1]["index"], 1);
        assert_eq!(value["outline"][1]["level"], 2);
        assert_eq!(value["outline"][1]["anchor"], "section");
    }

    #[test]
    fn anchors_mode_writes_heading_anchors() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_anchors(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "kittui-md anchors — 2 headings\nh1 #title Title\nh2 #section Section\n"
        );
    }

    #[test]
    fn anchors_mode_reports_empty_documents() {
        let doc = MarkdownDocument::default();
        let mut out = Vec::new();
        write_anchors(&doc, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "kittui-md anchors — 0 headings\n<empty>\n");
    }

    #[test]
    fn anchors_json_mode_writes_heading_anchors() {
        let doc = render_markdown("# Title\n\n## Section", 80);
        let mut out = Vec::new();
        write_anchors_json(&doc, &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["anchors"][0]["index"], 0);
        assert_eq!(value["anchors"][0]["level"], 1);
        assert_eq!(value["anchors"][0]["anchor"], "title");
        assert_eq!(value["anchors"][0]["text"], "Title");
        assert_eq!(value["anchors"][1]["index"], 1);
        assert_eq!(value["anchors"][1]["anchor"], "section");
    }

    #[test]
    fn plain_component_indents_multiline_text() {
        let comp = textbox("code:rust\nfn main() {}", 40, Tone::Tool);
        let mut out = Vec::new();
        write_plain_component(&mut out, &comp).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered, "[TextBox] code:rust\n          fn main() {}\n");
    }

    #[test]
    fn plain_metadata_sections_include_links_and_images_with_titles() {
        let doc = render_markdown(
            "[site](https://example.com \"Example title\")\n\n![logo](logo.png \"Logo title\")",
            80,
        );
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("1 links, 1 images"), "{rendered}");
        assert!(
            rendered.contains("links:\n  [site] https://example.com \"Example title\""),
            "{rendered}"
        );
        assert!(
            rendered.contains("images:\n  [logo] logo.png \"Logo title\""),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_footnotes() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![MarkdownFootnote {
                label: "note".to_string(),
                text: "details".to_string(),
            }],
            footnote_references: vec!["note".to_string()],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("footnote references:\n  [^note]"),
            "{rendered}"
        );
        assert!(
            rendered.contains("footnotes:\n  [^note] details"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_definitions() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![MarkdownDefinition {
                term: "Term".to_string(),
                definition: "Definition text".to_string(),
            }],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("definitions:\n  Term — Definition text"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_metadata_sections_include_math() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![kittui_affordances::MarkdownMath {
                kind: kittui_affordances::MarkdownMathKind::Inline,
                source: "x + y".to_string(),
            }],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("math:\n  inline x + y"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_html() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![kittui_affordances::MarkdownHtml {
                kind: kittui_affordances::MarkdownHtmlKind::Inline,
                source: "<kbd>".to_string(),
            }],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("html:\n  inline <kbd>"), "{rendered}");
    }

    #[test]
    fn plain_metadata_sections_include_code_blocks() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![kittui_affordances::MarkdownCodeBlock {
                language: Some("rust".to_string()),
                text: "fn main() {}".to_string(),
            }],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("code blocks:\n  rust fn main() {}"),
            "{rendered}"
        );
    }

    #[test]
    fn rich_outline_lines_mirror_plain_indentation() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 3,
                    text: "Deep".to_string(),
                    anchor: "deep".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        assert_eq!(
            outline_lines(&doc),
            vec!["Title #title".to_string(), "    Deep #deep".to_string()]
        );
    }

    #[test]
    fn plain_metadata_sections_include_heading_outline() {
        let doc = MarkdownDocument {
            components: vec![],
            links: vec![],
            tables: vec![],
            images: vec![],
            outline: vec![
                HeadingOutline {
                    level: 1,
                    text: "Title".to_string(),
                    anchor: "title".to_string(),
                },
                HeadingOutline {
                    level: 2,
                    text: "Section".to_string(),
                    anchor: "section".to_string(),
                },
            ],
            footnotes: vec![],
            footnote_references: vec![],
            definitions: vec![],
            math: vec![],
            html: vec![],
            code_blocks: vec![],
            metadata_blocks: vec![],
        };
        let mut out = Vec::new();
        write_plain(&doc, 20, &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert!(
            rendered.contains("outline:\n  Title #title\n    Section #section"),
            "{rendered}"
        );
    }

    #[test]
    fn align_table_cell_text_uses_markdown_alignment() {
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Left),
            "x  "
        );
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Center),
            " x "
        );
        assert_eq!(
            align_table_cell_text("x", 3, MarkdownTableAlignment::Right),
            "  x"
        );
        assert_eq!(
            align_table_cell_text("abcd", 2, MarkdownTableAlignment::Right),
            "ab"
        );
    }

    #[test]
    fn wrap_text_lines_wraps_and_respects_row_limit() {
        assert_eq!(
            wrap_text_lines("one two three four", 9, 3),
            vec![
                "one two".to_string(),
                "three".to_string(),
                "four".to_string()
            ]
        );
        assert_eq!(
            wrap_text_lines("one two three four", 9, 2),
            vec!["one two".to_string(), "three".to_string()]
        );
        assert_eq!(
            wrap_text_lines("abcdefghij", 4, 2),
            vec!["abcd".to_string()]
        );
    }

    #[test]
    fn viewport_filters_by_offset_and_height() {
        let comps = vec![h1("One", 40), h1("Two", 40), h1("Three", 40)];
        let layout = layout_components(&comps, &[], 80);
        let visible = visible_components(&layout, 4, Some(3));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].component.text, "Two");
    }
}
