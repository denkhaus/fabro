use std::borrow::Cow;
use std::sync::LazyLock;

use anyhow::Context as _;
use fabro_dot::normalize_for_graphviz;

/// Dark mode CSS injected into SVG output (leading newline included for
/// insertion).
const DARK_MODE_STYLE: &str = r##"
<style>
  @media (prefers-color-scheme: dark) {
    text { fill: #e0e0e0 !important; }
    [stroke="#357f9e"] { stroke: #5bb8d8; }
    [stroke="#666666"] { stroke: #999999; }
    polygon[fill="#357f9e"] { fill: #5bb8d8; }
    polygon[fill="#666666"] { fill: #999999; }
  }
</style>"##;

/// DOT graph-level defaults injected after the graph opening `{`.
const DOT_STYLE_DEFAULTS: &str = r##"
    bgcolor="transparent"
    node [color="#357f9e", fontname="Helvetica", fontsize=12, fontcolor="#1a1a1a"]
    edge [color="#666666", fontname="Helvetica", fontsize=10, fontcolor="#666666"]
"##;

static RANKDIR_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"rankdir\s*=\s*\w+").expect("hardcoded regex should compile")
});
static WHITE_BG_POLYGON_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"<polygon\b[^>]*fill="white"[^>]*stroke="none"[^>]*/>|<polygon\b[^>]*stroke="none"[^>]*fill="white"[^>]*/>"#,
    )
    .expect("hardcoded regex should compile")
});

/// Rewrite `rankdir=...` in DOT source.
#[must_use]
pub fn apply_direction<'a>(source: &'a str, direction: &str) -> std::borrow::Cow<'a, str> {
    let replacement = format!("rankdir={direction}");
    RANKDIR_RE.replace(source, replacement.as_str())
}

/// Inject DOT graph-level style defaults, after normalizing Fabro DOT into
/// DOT Graphviz accepts.
#[must_use]
pub fn inject_dot_style_defaults(source: &str) -> String {
    let source = normalize_for_graphviz(source);
    inject_dot_style_defaults_raw(&source)
}

fn inject_dot_style_defaults_raw(source: &str) -> String {
    let Some(pos) = source.find('{') else {
        return source.to_string();
    };
    let (before, after) = source.split_at(pos + 1);
    format!("{before}{DOT_STYLE_DEFAULTS}{after}")
}

/// Post-process raw SVG output from Graphviz.
#[must_use]
pub fn postprocess_svg(raw: Vec<u8>) -> Vec<u8> {
    let mut svg = String::from_utf8(raw)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    svg = WHITE_BG_POLYGON_RE.replace_all(&svg, "").into_owned();

    if let Some(svg_close) = svg
        .find("<svg")
        .and_then(|start| svg[start..].find('>').map(|end| start + end))
    {
        svg.insert_str(svg_close + 1, DARK_MODE_STYLE);
    }

    svg.into_bytes()
}

/// DOT source prepared for Graphviz rendering.
pub struct RenderableDot<'a> {
    source: Cow<'a, str>,
}

impl<'a> RenderableDot<'a> {
    /// Prepare Fabro DOT for Graphviz by applying render styling and
    /// normalizing Fabro-specific syntax such as dotted attribute keys.
    #[must_use]
    pub fn from_fabro_source(source: &'a str) -> Self {
        Self {
            source: Cow::Owned(inject_dot_style_defaults(source)),
        }
    }

    /// Return the DOT source that can be handed to Graphviz.
    #[must_use]
    pub fn as_graphviz_source(&self) -> &str {
        &self.source
    }
}

/// Render prepared DOT source into raw SVG via the vendored Graphviz library.
///
/// This is the only `graphviz_sys` boundary in the workspace.
pub fn render_raw_svg(dot: &RenderableDot<'_>) -> anyhow::Result<Vec<u8>> {
    graphviz_sys::render_dot_to_svg(dot.as_graphviz_source())
        .map_err(anyhow::Error::msg)
        .context("Graphviz rendering failed")
}

/// Render prepared DOT source into post-processed SVG.
pub fn render_svg(dot: &RenderableDot<'_>) -> anyhow::Result<Vec<u8>> {
    let raw = render_raw_svg(dot)?;
    Ok(postprocess_svg(raw))
}

/// Render Fabro DOT source into post-processed SVG.
pub fn render_dot(source: &str) -> anyhow::Result<Vec<u8>> {
    let dot = RenderableDot::from_fabro_source(source);
    render_svg(&dot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_direction_rewrites_rankdir() {
        let source = "digraph { rankdir=LR a -> b }";
        let rewritten = apply_direction(source, "TB");
        assert!(rewritten.contains("rankdir=TB"));
    }

    #[test]
    fn inject_style_defaults_adds_graph_defaults() {
        let source = "digraph X { a -> b }";
        let styled = inject_dot_style_defaults(source);
        assert!(styled.contains("bgcolor=\"transparent\""));
        assert!(styled.contains("node [color=\"#357f9e\""));
    }

    #[test]
    fn postprocess_svg_removes_white_background() {
        let raw = br#"<svg><polygon fill="white" stroke="none" points="0,0"/><text>x</text></svg>"#
            .to_vec();
        let svg = String::from_utf8(postprocess_svg(raw)).unwrap();
        assert!(!svg.contains("fill=\"white\""));
        assert!(svg.contains("@media (prefers-color-scheme: dark)"));
    }

    #[test]
    fn render_dot_produces_svg() {
        let svg = render_dot("digraph { a -> b }").unwrap();
        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
    }

    #[test]
    fn render_dot_accepts_leading_comment_with_template_braces() {
        let svg = render_dot(
            r#"// The goal is rendered into prompts as {{ goal }}.
digraph G {
    start [shape=Mdiamond]
    exit [shape=Msquare]
    a [label="A"]
    start -> a -> exit
}"#,
        )
        .unwrap();

        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
    }

    #[test]
    fn render_dot_complex_graph() {
        let source = r#"digraph {
            subgraph cluster_0 {
                label = "process #1";
                a0 -> a1 -> a2 -> a3;
            }
            subgraph cluster_1 {
                label = "process #2";
                b0 -> b1 -> b2 -> b3;
            }
            start -> a0;
            start -> b0;
            a1 -> b3;
            b2 -> a3;
            a3 -> end;
            b3 -> end;
        }"#;
        let svg = render_dot(source).unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(svg_str.contains("<svg"));
        assert!(svg_str.contains("process #1"));
        assert!(svg_str.contains("process #2"));
    }

    #[test]
    fn render_dot_invalid_source_returns_error() {
        let result = render_dot("not valid dot {{{");
        assert!(result.is_err());
    }

    #[test]
    fn render_dot_accepts_fabro_dotted_attribute_keys() {
        let svg = render_dot(
            r#"digraph X {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                a [label="A", acp.command="codex"]
                start -> a -> exit
            }"#,
        )
        .unwrap();

        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
    }

    #[test]
    fn renderable_dot_normalizes_fabro_source_for_graphviz() {
        let dot = RenderableDot::from_fabro_source(
            r#"digraph X {
                a [label="A", acp.command="codex"]
            }"#,
        );

        assert!(
            dot.as_graphviz_source()
                .contains(r#""acp.command"="codex""#)
        );
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "unit test reads checked-in DOT compatibility fixtures synchronously"
    )]
    #[test]
    fn render_dot_compatibility_corpus_produces_svg() {
        let fixtures = dot_compatibility_fixtures();

        assert_eq!(
            fixtures.len(),
            3,
            "dot compatibility corpus should stay intentionally small"
        );

        for fixture in fixtures {
            let source = std::fs::read_to_string(&fixture)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture.display()));
            let dot = RenderableDot::from_fabro_source(&source);
            let svg = render_svg(&dot)
                .unwrap_or_else(|err| panic!("failed to render {}: {err:#}", fixture.display()));
            let svg = String::from_utf8(svg)
                .unwrap_or_else(|err| panic!("SVG was not UTF-8 for {}: {err}", fixture.display()));

            assert!(
                svg.contains("<svg"),
                "expected SVG output for {}, got: {}",
                fixture.display(),
                &svg[..svg.len().min(200)]
            );
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "unit test helper enumerates checked-in DOT compatibility fixtures synchronously"
    )]
    fn dot_compatibility_fixtures() -> Vec<std::path::PathBuf> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test/dot-compatibility");
        let mut fixtures = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|err| {
                        panic!("failed to read entry in {}: {err}", dir.display())
                    })
                    .path()
            })
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("fabro"))
            .collect::<Vec<_>>();
        fixtures.sort();
        fixtures
    }
}
