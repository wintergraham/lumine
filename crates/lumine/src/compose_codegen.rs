//! Build-tool-only native factory generation. No JSON dependency or runtime interpreter.
use super::{Kind, View};
use crate::{scene::Color, style::Styles};
use std::collections::BTreeMap;

pub struct CompiledSource {
    pub rust: String,
    /// Relative filenames referenced by include_bytes! in the generated module.
    pub assets: BTreeMap<String, Vec<u8>>,
}
fn color(c: Color) -> String {
    format!("lumine::scene::Color({}, {}, {}, {})", c.0, c.1, c.2, c.3)
}
fn family(f: &crate::text_layout::FontFamily) -> String {
    use crate::text_layout::FontFamily::*;
    match f {
        Named(s) => format!("lumine::text_layout::FontFamily::from({s:?})"),
        _ => format!("lumine::text_layout::FontFamily::{f:?}"),
    }
}
fn styles(s: &Styles) -> Result<String, String> {
    let mut out = "Styles::new()".to_owned();
    macro_rules! scalar { ($($field:ident => $method:ident),* $(,)?) => { $(if let Some(v) = s.$field { out += &format!(".{}({v:?})", stringify!($method)); })* }; }
    scalar!(absolute=>absolute_position, width=>w, height=>h, min_width=>min_w, max_width=>max_w,
        min_height=>min_h, max_height=>max_h, gap=>gap, flex_grow=>flex_grow, flex_shrink=>flex_shrink,
        text_wrap=>text_wrap, text_size=>text_size, font_weight=>font_weight, italic=>italic,
        radius=>rounded, border_width=>border, opacity=>opacity, blur=>blur, edge_fade=>edge_fade,
        isolated=>isolated, z_index=>z_index);
    // absolute() takes no argument; preserve explicit false through layout defaults.
    out = out.replace(".absolute_position(true)", ".absolute()");
    out = out.replace(".absolute_position(false)", ".relative()");
    if let Some(v) = s.width_percent {
        out += &format!(".w_percent({:?})", v * 100.);
    }
    if let Some(v) = s.height_percent {
        out += &format!(".h_percent({:?})", v * 100.);
    }
    if let Some(v) = s.layout {
        out += match v {
            crate::scene::Layout::Row => ".flex_row()",
            crate::scene::Layout::Column => ".flex_col()",
            crate::scene::Layout::Overlay => ".overlay()",
        };
    }
    if let Some(v) = s.clip {
        out += if v {
            ".overflow_hidden()"
        } else {
            ".overflow_visible()"
        };
    }
    if let Some(v) = s.object_fit {
        out += &format!(".object_fit(lumine::style::ObjectFit::{v:?})");
    }
    if let Some(v) = s.cursor {
        out += &format!(".cursor(lumine::cursor::Cursor::{v:?})");
    }
    if let Some(v) = s.align {
        out += &format!(".items_{}()", format!("{v:?}").to_lowercase());
    }
    if let Some(v) = s.justify {
        out += &format!(
            ".justify_{}()",
            match v {
                crate::scene::Justify::SpaceBetween => "between".into(),
                crate::scene::Justify::SpaceAround => "around".into(),
                crate::scene::Justify::SpaceEvenly => "evenly".into(),
                _ => format!("{v:?}").to_lowercase(),
            }
        );
    }
    for (value, mask, methods) in [
        (s.padding, s.padding_sides, ["pt", "pr", "pb", "pl"]),
        (s.margin, s.margin_sides, ["mt", "mr", "mb", "ml"]),
    ] {
        if let Some(v) = value {
            for (i, n) in [v.top, v.right, v.bottom, v.left].into_iter().enumerate() {
                if mask == 0 || mask & (1 << i) != 0 {
                    out += &format!(".{}({n:?})", methods[i]);
                }
            }
        }
    }
    for (v, method) in [
        (s.background, "bg"),
        (s.text_color, "text_color"),
        (s.border_color, "border_color"),
    ] {
        if let Some(v) = v {
            out += &format!(".{method}({})", color(v));
        }
    }
    if let Some(v) = &s.font_family {
        out += &format!(".font_family({})", family(v));
    }
    if let Some(v) = s.text_align {
        out += &format!(".text_align(lumine::text_layout::TextAlign::{v:?})");
    }
    if let Some(v) = s.line_height {
        out += &format!(
            ".line_height_rounded({:?})",
            v.pixels().unwrap_or(0.) + 2. * v.baseline_offset()
        );
        // Exact line heights have zero baseline correction.
        if v.baseline_offset() == 0. {
            out = out.replace(
                &format!(".line_height_rounded({:?})", v.pixels().unwrap_or(0.)),
                &format!(".line_height({:?})", v.pixels().unwrap_or(0.)),
            );
        }
    }
    if let Some(v) = s.letter_spacing {
        out += &format!(".letter_spacing({:?})", v.pixels());
    }
    if let Some(v) = s.text_overflow {
        out += &format!(".text_overflow(lumine::text_layout::TextOverflow::{v:?})");
    }
    if let Some(v) = &s.font_features {
        out += &format!(
            ".font_features(lumine::text_layout::FontFeatures::new({:?}))",
            v.settings()
        );
    }
    if let Some(v) = &s.font_fallbacks {
        out += &format!(
            ".font_fallbacks(vec![{}])",
            v.iter().map(family).collect::<Vec<_>>().join(",")
        );
    }
    if let Some(v) = s.line_clamp {
        out += &format!(".line_clamp({})", v.map_or(0, |n| n.get()));
    }
    if let Some(Some(v)) = s.border_edges {
        out += &format!(".border_edges(lumine::scene::{v:?})");
    }
    if s.border_edges == Some(None) && s.border_width.is_none() {
        return Err("border edge reset needs an explicit border width".into());
    }
    if let Some(Some(v)) = s.corners {
        out += &format!(".rounded_corners(lumine::decoration::{v:?})");
    }
    if s.corners == Some(None) && s.radius.is_none() {
        return Err("corner reset needs an explicit radius".into());
    }
    if let Some(v) = s.border_style {
        match v {
            crate::decoration::BorderStyle::Solid => {}
            crate::decoration::BorderStyle::Dashed { length, gap } => {
                out += &format!(
                    ".border_style(lumine::decoration::BorderStyle::Dashed {{ length: {length:?}, gap: {gap:?} }})"
                )
            }
        }
    }
    if let Some(v) = &s.shadows {
        out += &format!(
            ".shadows(vec![{}])",
            v.as_deref()
                .unwrap_or(&[])
                .iter()
                .map(shadow)
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    if let Some(v) = s.shadow {
        match v {
            Some(v) => out += &format!(".shadow({})", shadow(&v)),
            None => out += ".shadow_none()",
        }
    }
    if let Some(v) = s.transform {
        out += &format!(".translate({:?},{:?})", v.x, v.y);
    }
    if let Some(v) = s.scale {
        out += &format!(".scale({:?},{:?})", v[0], v[1]);
    }
    if let Some(v) = s.rotation {
        out += &format!(".rotate({v:?})");
    }
    if let Some(v) = s.transform_origin {
        out += &format!(".transform_origin({:?},{:?})", v[0], v[1]);
    }
    if let Some(v) = s.fade_edges {
        out += &format!(".fade_edges({:?},{:?})", v[0], v[1]);
    }
    if let Some(v) = &s.paint_background {
        match v {
            Some(crate::decoration::Background::Linear { angle, stops }) => {
                out += &format!(
                    ".bg_gradient({angle:?}, vec![{}])",
                    stops
                        .iter()
                        .map(|v| format!(
                            "lumine::canvas::GradientStop {{ offset: {:?}, color: {} }}",
                            v.offset,
                            color(v.color)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            None if s.background.is_some() => {}
            _ => return Err("unsupported brush background in native codegen".into()),
        }
    }
    if let Some(v) = &s.layout_options {
        // Debug for public layout fields is Rust syntax when enum variants are in scope.
        let mut expr = format!("{v:?}").replace(
            "display: Some(None)",
            "display: Some(lumine::layout::Display::None)",
        );
        for name in [
            "Start",
            "Center",
            "End",
            "Stretch",
            "SpaceBetween",
            "SpaceAround",
            "SpaceEvenly",
        ] {
            expr = expr.replace(
                &format!("align_content: Some({name})"),
                &format!("align_content: Some(lumine::layout::ContentAlign::{name})"),
            );
        }
        out += &format!(".layout_options({expr})");
    }
    Ok(out)
}
fn shadow(v: &crate::scene::BoxShadow) -> String {
    format!(
        "lumine::scene::BoxShadow {{ color: {}, offset: lumine::scene::{:?}, blur_radius: {:?}, spread: {:?} }}",
        color(v.color),
        v.offset,
        v.blur_radius,
        v.spread
    )
}
impl View {
    /// Freeze an unbound static view tree into native Rust constructors.
    /// Runtime widgets and behavior must be supplied through CompiledHost.
    /// Image/SVG sources are evaluated as static build-tool assets.
    pub fn compile_rust(self) -> Result<CompiledSource, String> {
        let mut result = CompiledSource {
            rust: String::from(
                "// Generated. Edit the design JSON, not this file.\n#[allow(unused_imports)]\nuse lumine::{compose::prelude::*, layout::{*, Length::*, Display::{Flex,Grid}, FlexWrap::*}, scene::{Align::*, Layout::*}};\n",
            ),
            assets: BTreeMap::new(),
        };
        let mut count = 0;
        let root = freeze(self, &mut result, &mut count)?;
        result.rust += &format!(
            "pub fn build(host: &impl lumine::compose::CompiledHost) -> View {{ node_{root}(host) }}\n"
        );
        Ok(result)
    }
}
fn freeze(mut v: View, out: &mut CompiledSource, count: &mut usize) -> Result<usize, String> {
    let index = *count;
    *count += 1;
    let id = v.id.clone().unwrap_or_default();
    if v.reactive_style.is_some()
        || v.disabled.is_some()
        || v.read_only.is_some()
        || v.click.is_some()
        || !v.events.is_empty()
        || v.on_editor.is_some()
        || v.animation.is_some()
        || v.drag_preview.is_some()
        || v.layout_motion.is_some()
        || !v.visibility.is_empty()
        || v.layout_target.is_some()
        || v.menu_trigger.is_some()
        || v.scroll_progress.is_some()
        || v.selection_style.is_some()
        || v.select_all_on_focus.is_some()
        || v.keyboard_navigation.is_some()
        || v.dismiss_backdrop.is_some()
        || v.trigger_id.is_some()
        || v.layout_id.is_some()
        || v.menu_checked.is_some()
        || !v.keymap.bindings.is_empty()
        || !v.keymap.contexts.is_empty()
    {
        return Err(format!(
            "{id}: static factory has runtime behavior; attach it through CompiledHost"
        ));
    }
    let base = match &mut v.kind {
        Kind::Container(layout) => match layout {
            crate::scene::Layout::Row => "row()".into(),
            crate::scene::Layout::Column => "column()".into(),
            crate::scene::Layout::Overlay => "overlay()".into(),
        },
        Kind::Text(t) => format!("text({t:?})"),
        Kind::Button => "button()".into(),
        Kind::Svg { label, source } => {
            let svg = source();
            let name = format!("asset_{index}.svg");
            out.assets.insert(name.clone(), svg.bytes().to_vec());
            let mut expr =
                format!("lumine::svg::SvgData::new(&include_bytes!({name:?})[..]).unwrap()");
            if let Some(tint) = svg.tint() {
                expr += &format!(".tinted({})", color(tint));
            }
            expr += &format!(".transformed(lumine::affine::{:?})", svg.transform());
            format!(
                "{{ static ASSET: std::sync::OnceLock<std::sync::Arc<lumine::svg::SvgData>> = std::sync::OnceLock::new(); svg({label:?}, ASSET.get_or_init(|| std::sync::Arc::new({expr})).clone()) }}"
            )
        }
        Kind::Image { label, source } => {
            let image = source();
            if image.procedural().is_some() || image.effect_chain().is_some() {
                return Err(format!(
                    "{id}: procedural images need a native component binding"
                ));
            }
            let name = format!("asset_{index}.rgba");
            out.assets.insert(name.clone(), image.pixels().to_vec());
            format!(
                "{{ static ASSET: std::sync::OnceLock<std::sync::Arc<lumine::image::ImageData>> = std::sync::OnceLock::new(); image({label:?}, ASSET.get_or_init(|| std::sync::Arc::new(lumine::image::ImageData::new({}, {}, include_bytes!({name:?}).to_vec()).unwrap().sampled(lumine::image::ImageSampling::{:?}).transformed(lumine::affine::{:?}))).clone()) }}",
                image.width(),
                image.height(),
                image.sampling(),
                image.transform()
            )
        }
        _ => {
            return Err(format!(
                "{id}: runtime view kind cannot be frozen; use a native component binding"
            ));
        }
    };
    let mut source = format!(
        "fn node_{index}(host: &impl lumine::compose::CompiledHost) -> View {{\nlet view = host.create({id:?}, || {base}).id({id:?}).style({});\n",
        styles(&v.styles)?
    );
    source += "let view = view";
    for (method, patch) in [
        ("hover", v.variants.hover),
        ("active", v.variants.active),
        ("focus", v.variants.focus),
        ("disabled_style", v.variants.disabled),
    ] {
        if let Some(patch) = patch {
            source += &format!(".{method}(|_| {})", styles(&patch)?);
        }
    }
    if let Some(focusable) = v.focusable {
        source += &format!(".focusable({focusable})");
    }
    if let Some(scrollbar) = v.scrollbar {
        source += &format!(".scrollbar({scrollbar})");
    }
    for child in v.children {
        let child = freeze(child, out, count)?;
        source += &format!(".child(node_{child}(host))");
    }
    source += &format!(";\nhost.decorate({id:?}, view)\n}}\n");
    out.rust += &source;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use crate::compose::prelude::*;
    #[test]
    fn codegen_rejects_runtime_behavior_and_emits_native_constructors() {
        assert!(button().on_click(|| {}).compile_rust().is_err());
        assert!(text_signal(|| "reactive".into()).compile_rust().is_err());
        let source = column()
            .id("root")
            .child(text("label").id("label"))
            .compile_rust()
            .unwrap();
        assert!(
            source
                .rust
                .contains("host.create(\"label\", || text(\"label\"))")
        );
        assert!(source.rust.contains("CompiledHost"));
        assert!(!source.rust.contains("serde_json"));
    }
}
