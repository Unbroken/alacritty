use std::cmp;
use std::fmt;

use crossfont::Size as FontSize;
use log::debug;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use alacritty_config_derive::{ConfigDeserialize, SerdeReplace};

use crate::config::ui_config::Delta;

/// Font rendering mode for anti-aliasing.
#[derive(ConfigDeserialize, Serialize, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub enum FontRendering {
    /// No anti-aliasing, grid-fitted pixel rendering.
    Aliased,
    /// Grayscale anti-aliasing.
    #[default]
    Grayscale,
    /// Subpixel (ClearType) rendering.
    Subpixel,
}

impl From<FontRendering> for crossfont::RenderingMode {
    fn from(rendering: FontRendering) -> crossfont::RenderingMode {
        match rendering {
            FontRendering::Aliased => crossfont::RenderingMode::Aliased,
            FontRendering::Grayscale => crossfont::RenderingMode::Grayscale,
            FontRendering::Subpixel => crossfont::RenderingMode::Subpixel,
        }
    }
}

/// Font config.
///
/// Defaults are provided at the level of this struct per platform, but not per
/// field in this struct. It might be nice in the future to have defaults for
/// each value independently. Alternatively, maybe erroring when the user
/// doesn't provide complete config is Ok.
#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// Extra spacing per character.
    pub offset: Delta<i8>,

    /// Glyph offset within character cell.
    pub glyph_offset: Delta<i8>,

    #[config(removed = "set the AppleFontSmoothing user default instead")]
    pub use_thin_strokes: bool,

    /// Font rendering mode (grayscale or subpixel).
    pub rendering: FontRendering,

    /// Whether to use grid fitting (hinting) for font rendering.
    pub grid_fitting: bool,

    /// Normal font face.
    normal: FontDescription,

    /// Bold font face.
    bold: SecondaryFontDescription,

    /// Italic font face.
    italic: SecondaryFontDescription,

    /// Bold italic font face.
    bold_italic: SecondaryFontDescription,

    /// Font size in points.
    size: Size,

    /// Whether to use the built-in font for box drawing characters.
    pub builtin_box_drawing: bool,

    /// DPI scale factor overrides.
    dpi_override: Vec<DpiOverride>,
}

impl Font {
    /// Get a font clone with a size modification.
    pub fn with_size(self, size: FontSize) -> Font {
        Font { size: Size(size), ..self }
    }

    #[inline]
    pub fn size(&self) -> FontSize {
        self.size.0
    }

    /// Get normal font description.
    pub fn normal(&self) -> &FontDescription {
        &self.normal
    }

    /// Get bold font description.
    pub fn bold(&self) -> FontDescription {
        self.bold.desc(&self.normal)
    }

    /// Get italic font description.
    pub fn italic(&self) -> FontDescription {
        self.italic.desc(&self.normal)
    }

    /// Get bold italic font description.
    pub fn bold_italic(&self) -> FontDescription {
        self.bold_italic.desc(&self.normal)
    }

    /// Resolve font configuration for a given DPI scale factor.
    ///
    /// If any `dpi_override` entries have `min_scale <= scale_factor`,
    /// the one with the highest `min_scale` is selected and its fields
    /// are merged on top of the base font config.
    pub fn resolve_for_scale(&self, scale_factor: f64) -> Font {
        let scale = scale_factor as f32;

        let best_override = self
            .dpi_override
            .iter()
            .filter(|o| o.min_scale.0 > 0.0 && scale >= o.min_scale.0)
            .max_by(|a, b| a.min_scale.cmp(&b.min_scale));

        let Some(ovr) = best_override else {
            debug!(
                "No DPI override matched for scale {scale}; using base font \"{}\"",
                self.normal.family,
            );
            return self.clone();
        };

        debug!(
            "DPI override matched: min_scale={} for scale {scale}",
            ovr.min_scale.0,
        );

        let mut resolved = self.clone();
        resolved.dpi_override = Vec::new();

        if let Some(ref family) = ovr.normal.family {
            resolved.normal.family = family.clone();
        }
        if ovr.normal.style.is_some() {
            resolved.normal.style = ovr.normal.style.clone();
        }

        if let Some(ref family) = ovr.bold.family {
            resolved.bold.family = Some(family.clone());
        }
        if ovr.bold.style.is_some() {
            resolved.bold.style = ovr.bold.style.clone();
        }

        if let Some(ref family) = ovr.italic.family {
            resolved.italic.family = Some(family.clone());
        }
        if ovr.italic.style.is_some() {
            resolved.italic.style = ovr.italic.style.clone();
        }

        if let Some(ref family) = ovr.bold_italic.family {
            resolved.bold_italic.family = Some(family.clone());
        }
        if ovr.bold_italic.style.is_some() {
            resolved.bold_italic.style = ovr.bold_italic.style.clone();
        }

        if let Some(ref size) = ovr.size {
            resolved.size = size.clone();
        }
        if let Some(rendering) = ovr.rendering {
            resolved.rendering = rendering;
        }
        if let Some(grid_fitting) = ovr.grid_fitting {
            resolved.grid_fitting = grid_fitting;
        }
        if let Some(offset) = ovr.offset {
            resolved.offset = offset;
        }
        if let Some(glyph_offset) = ovr.glyph_offset {
            resolved.glyph_offset = glyph_offset;
        }

        debug!("Resolved font for scale {scale}: \"{}\"", resolved.normal.family);

        resolved
    }
}

impl Default for Font {
    fn default() -> Font {
        Self {
            builtin_box_drawing: true,
            grid_fitting: false,
            glyph_offset: Default::default(),
            use_thin_strokes: Default::default(),
            rendering: Default::default(),
            bold_italic: Default::default(),
            italic: Default::default(),
            offset: Default::default(),
            normal: Default::default(),
            bold: Default::default(),
            size: Default::default(),
            dpi_override: Default::default(),
        }
    }
}

/// Description of the normal font.
#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct FontDescription {
    pub family: String,
    pub style: Option<String>,
}

impl Default for FontDescription {
    fn default() -> FontDescription {
        FontDescription {
            #[cfg(not(any(target_os = "macos", windows)))]
            family: "monospace".into(),
            #[cfg(target_os = "macos")]
            family: "Menlo".into(),
            #[cfg(windows)]
            family: "Consolas".into(),
            style: None,
        }
    }
}

/// Description of the italic and bold font.
#[derive(ConfigDeserialize, Serialize, Debug, Default, Clone, PartialEq, Eq)]
pub struct SecondaryFontDescription {
    family: Option<String>,
    style: Option<String>,
}

impl SecondaryFontDescription {
    pub fn desc(&self, fallback: &FontDescription) -> FontDescription {
        FontDescription {
            family: self.family.clone().unwrap_or_else(|| fallback.family.clone()),
            style: self.style.clone(),
        }
    }
}

#[derive(SerdeReplace, Debug, Clone, PartialEq, Eq)]
struct Size(FontSize);

impl Default for Size {
    fn default() -> Self {
        Self(FontSize::new(11.25))
    }
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NumVisitor;
        impl Visitor<'_> for NumVisitor {
            type Value = Size;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("f64 or i64")
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(Size(FontSize::new(value as f32)))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(Size(FontSize::new(value as f32)))
            }
        }

        deserializer.deserialize_any(NumVisitor)
    }
}

impl Serialize for Size {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f32(self.0.as_pt())
    }
}

/// Scale factor threshold for DPI overrides.
#[derive(SerdeReplace, Debug, Clone, PartialEq)]
struct ScaleThreshold(f32);

impl Eq for ScaleThreshold {}

impl Ord for ScaleThreshold {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for ScaleThreshold {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Default for ScaleThreshold {
    fn default() -> Self {
        Self(0.0)
    }
}

impl<'de> Deserialize<'de> for ScaleThreshold {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NumVisitor;
        impl Visitor<'_> for NumVisitor {
            type Value = ScaleThreshold;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("f64 or i64")
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(ScaleThreshold(value as f32))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ScaleThreshold(value as f32))
            }
        }

        deserializer.deserialize_any(NumVisitor)
    }
}

impl Serialize for ScaleThreshold {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f32(self.0)
    }
}

/// Font description override with all-optional fields for merging.
#[derive(ConfigDeserialize, Serialize, Debug, Default, Clone, PartialEq, Eq)]
struct FontDescriptionOverride {
    family: Option<String>,
    style: Option<String>,
}

/// DPI scale factor override for font configuration.
#[derive(ConfigDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
struct DpiOverride {
    /// Minimum scale factor for this override to activate.
    min_scale: ScaleThreshold,

    /// Override for the normal font face.
    normal: FontDescriptionOverride,

    /// Override for the bold font face.
    bold: FontDescriptionOverride,

    /// Override for the italic font face.
    italic: FontDescriptionOverride,

    /// Override for the bold italic font face.
    bold_italic: FontDescriptionOverride,

    /// Override for font size in points.
    size: Option<Size>,

    /// Override for font rendering mode.
    rendering: Option<FontRendering>,

    /// Override for grid fitting.
    grid_fitting: Option<bool>,

    /// Override for extra spacing per character.
    offset: Option<Delta<i8>>,

    /// Override for glyph offset within character cell.
    glyph_offset: Option<Delta<i8>>,
}

impl Default for DpiOverride {
    fn default() -> Self {
        Self {
            min_scale: Default::default(),
            normal: Default::default(),
            bold: Default::default(),
            italic: Default::default(),
            bold_italic: Default::default(),
            size: None,
            rendering: None,
            grid_fitting: None,
            offset: None,
            glyph_offset: None,
        }
    }
}
