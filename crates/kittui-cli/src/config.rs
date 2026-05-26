use std::{env, ffi::OsString, fmt, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use kittui::RendererKind;

#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    Tty,
    Yaml,
    Env,
    Flag,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Tty => f.write_str("tty"),
            Self::Yaml => f.write_str("yaml"),
            Self::Env => f.write_str("env"),
            Self::Flag => f.write_str("flag"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> Resolved<T> {
    fn new(value: T, source: ConfigSource) -> Self {
        Self { value, source }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RendererArg {
    Cpu,
    Gpu,
    Auto,
}

impl From<RendererArg> for RendererKind {
    fn from(value: RendererArg) -> Self {
        match value {
            RendererArg::Cpu => RendererKind::Cpu,
            RendererArg::Gpu => RendererKind::Gpu,
            RendererArg::Auto => RendererKind::Auto,
        }
    }
}

impl FromStr for RendererArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "cpu" => Ok(Self::Cpu),
            "gpu" => Ok(Self::Gpu),
            "auto" => Ok(Self::Auto),
            other => Err(format!(
                "invalid renderer {other:?}; expected cpu, gpu, or auto"
            )),
        }
    }
}

impl fmt::Display for RendererArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cpu => f.write_str("cpu"),
            Self::Gpu => f.write_str("gpu"),
            Self::Auto => f.write_str("auto"),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub cache_dir: Option<PathBuf>,
    pub renderer: Option<RendererArg>,
    pub terminal_cols: Option<u16>,
    pub terminal_rows: Option<u16>,
    pub json: Option<bool>,
    #[serde(rename = "box")]
    pub box_: BoxConfig,
    pub gradient: GradientConfig,
    pub glow: GlowConfig,
    pub cache: CacheConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoxConfig {
    pub x: Option<u16>,
    pub y: Option<u16>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub radius: Option<f32>,
    pub border: Option<f32>,
    pub animate: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GradientConfig {
    pub width: Option<String>,
    pub height: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub direction: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GlowConfig {
    pub width: Option<String>,
    pub height: Option<String>,
    pub color: Option<String>,
    pub intensity: Option<f32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    pub budget: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct EnvConfig {
    pub cache_dir: Option<PathBuf>,
    pub renderer: Option<RendererArg>,
    pub terminal_cols: Option<u16>,
    pub terminal_rows: Option<u16>,
    pub json: Option<bool>,
    pub box_: BoxConfig,
    pub gradient: GradientConfig,
    pub glow: GlowConfig,
    pub cache: CacheConfig,
}

#[derive(Clone, Debug)]
pub struct GlobalConfig {
    pub cache_dir: Resolved<PathBuf>,
    pub renderer: Resolved<RendererArg>,
    pub terminal_cols: Resolved<u16>,
    pub terminal_rows: Resolved<u16>,
    pub json: Resolved<bool>,
}

impl GlobalConfig {
    pub fn source_json(&self) -> serde_json::Value {
        serde_json::json!({
            "cache_dir": self.cache_dir.source,
            "renderer": self.renderer.source,
            "terminal_cols": self.terminal_cols.source,
            "terminal_rows": self.terminal_rows.source,
            "json": self.json.source,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ConfigLayers {
    yaml: FileConfig,
    env: EnvConfig,
}

impl ConfigLayers {
    pub fn load() -> Result<Self> {
        Ok(Self {
            yaml: load_yaml_config()?,
            env: EnvConfig::from_process_env()?,
        })
    }

    #[cfg(test)]
    pub fn from_parts(yaml: FileConfig, env: EnvConfig) -> Self {
        Self { yaml, env }
    }

    pub fn resolve_global(&self, flags: GlobalFlagValues) -> GlobalConfig {
        GlobalConfig {
            cache_dir: resolve(
                kittui::scene::default_cache_dir(),
                self.yaml.cache_dir.clone(),
                self.env.cache_dir.clone(),
                flags.cache_dir,
            ),
            renderer: resolve(
                RendererArg::Auto,
                self.yaml.renderer,
                self.env.renderer,
                flags.renderer,
            ),
            terminal_cols: resolve(
                80,
                self.yaml.terminal_cols,
                self.env.terminal_cols,
                flags.terminal_cols,
            ),
            terminal_rows: resolve(
                24,
                self.yaml.terminal_rows,
                self.env.terminal_rows,
                flags.terminal_rows,
            ),
            json: resolve(
                false,
                self.yaml.json,
                self.env.json,
                flags.json.then_some(true),
            ),
        }
    }

    pub fn resolve_box(&self, flags: BoxFlagValues) -> ResolvedBoxConfig {
        ResolvedBoxConfig {
            x: resolve(0, self.yaml.box_.x, self.env.box_.x, flags.x),
            y: resolve(0, self.yaml.box_.y, self.env.box_.y, flags.y),
            width: resolve(
                "40".to_string(),
                self.yaml.box_.width.clone(),
                self.env.box_.width.clone(),
                flags.width,
            ),
            height: resolve(
                "8".to_string(),
                self.yaml.box_.height.clone(),
                self.env.box_.height.clone(),
                flags.height,
            ),
            fg: resolve(
                "#00d8ff".to_string(),
                self.yaml.box_.fg.clone(),
                self.env.box_.fg.clone(),
                flags.fg,
            ),
            bg: resolve(
                "#08111fcc".to_string(),
                self.yaml.box_.bg.clone(),
                self.env.box_.bg.clone(),
                flags.bg,
            ),
            radius: resolve(
                6.0,
                self.yaml.box_.radius,
                self.env.box_.radius,
                flags.radius,
            ),
            border: resolve(
                1.5,
                self.yaml.box_.border,
                self.env.box_.border,
                flags.border,
            ),
            animate: resolve_opt(
                self.yaml.box_.animate.clone(),
                self.env.box_.animate.clone(),
                flags.animate,
            ),
        }
    }

    pub fn resolve_gradient(&self, flags: GradientFlagValues) -> ResolvedGradientConfig {
        ResolvedGradientConfig {
            width: resolve(
                "100%".to_string(),
                self.yaml.gradient.width.clone(),
                self.env.gradient.width.clone(),
                flags.width,
            ),
            height: resolve(
                "1".to_string(),
                self.yaml.gradient.height.clone(),
                self.env.gradient.height.clone(),
                flags.height,
            ),
            left: resolve(
                "#00d8ff".to_string(),
                self.yaml.gradient.left.clone(),
                self.env.gradient.left.clone(),
                flags.left,
            ),
            right: resolve(
                "#b48cff".to_string(),
                self.yaml.gradient.right.clone(),
                self.env.gradient.right.clone(),
                flags.right,
            ),
            direction: resolve(
                "horizontal".to_string(),
                self.yaml.gradient.direction.clone(),
                self.env.gradient.direction.clone(),
                flags.direction,
            ),
        }
    }

    pub fn resolve_glow(&self, flags: GlowFlagValues) -> ResolvedGlowConfig {
        ResolvedGlowConfig {
            width: resolve(
                "40".to_string(),
                self.yaml.glow.width.clone(),
                self.env.glow.width.clone(),
                flags.width,
            ),
            height: resolve(
                "8".to_string(),
                self.yaml.glow.height.clone(),
                self.env.glow.height.clone(),
                flags.height,
            ),
            color: resolve(
                "#00d8ff".to_string(),
                self.yaml.glow.color.clone(),
                self.env.glow.color.clone(),
                flags.color,
            ),
            intensity: resolve(
                0.6,
                self.yaml.glow.intensity,
                self.env.glow.intensity,
                flags.intensity,
            ),
        }
    }

    pub fn resolve_cache_budget(&self, flag: Option<u64>) -> Resolved<Option<u64>> {
        resolve_opt(self.yaml.cache.budget, self.env.cache.budget, flag)
    }
}

fn resolve<T>(default: T, yaml: Option<T>, env: Option<T>, flag: Option<T>) -> Resolved<T> {
    if let Some(value) = flag {
        Resolved::new(value, ConfigSource::Flag)
    } else if let Some(value) = env {
        Resolved::new(value, ConfigSource::Env)
    } else if let Some(value) = yaml {
        Resolved::new(value, ConfigSource::Yaml)
    } else {
        Resolved::new(default, ConfigSource::Default)
    }
}

fn resolve_opt<T>(yaml: Option<T>, env: Option<T>, flag: Option<T>) -> Resolved<Option<T>> {
    if flag.is_some() {
        Resolved::new(flag, ConfigSource::Flag)
    } else if env.is_some() {
        Resolved::new(env, ConfigSource::Env)
    } else if yaml.is_some() {
        Resolved::new(yaml, ConfigSource::Yaml)
    } else {
        Resolved::new(None, ConfigSource::Default)
    }
}

#[derive(Clone, Debug, Default)]
pub struct GlobalFlagValues {
    pub cache_dir: Option<PathBuf>,
    pub renderer: Option<RendererArg>,
    pub terminal_cols: Option<u16>,
    pub terminal_rows: Option<u16>,
    pub json: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BoxFlagValues {
    pub x: Option<u16>,
    pub y: Option<u16>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub radius: Option<f32>,
    pub border: Option<f32>,
    pub animate: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GradientFlagValues {
    pub width: Option<String>,
    pub height: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub direction: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GlowFlagValues {
    pub width: Option<String>,
    pub height: Option<String>,
    pub color: Option<String>,
    pub intensity: Option<f32>,
}

pub type ResolvedBoxConfig = BoxFlagValuesResolved;
pub type ResolvedGradientConfig = GradientFlagValuesResolved;
pub type ResolvedGlowConfig = GlowFlagValuesResolved;

#[derive(Clone, Debug)]
pub struct BoxFlagValuesResolved {
    pub x: Resolved<u16>,
    pub y: Resolved<u16>,
    pub width: Resolved<String>,
    pub height: Resolved<String>,
    pub fg: Resolved<String>,
    pub bg: Resolved<String>,
    pub radius: Resolved<f32>,
    pub border: Resolved<f32>,
    pub animate: Resolved<Option<String>>,
}

impl BoxFlagValuesResolved {
    pub fn source_json(&self) -> serde_json::Value {
        serde_json::json!({
            "x": self.x.source,
            "y": self.y.source,
            "width": self.width.source,
            "height": self.height.source,
            "fg": self.fg.source,
            "bg": self.bg.source,
            "radius": self.radius.source,
            "border": self.border.source,
            "animate": self.animate.source,
        })
    }
}

#[derive(Clone, Debug)]
pub struct GradientFlagValuesResolved {
    pub width: Resolved<String>,
    pub height: Resolved<String>,
    pub left: Resolved<String>,
    pub right: Resolved<String>,
    pub direction: Resolved<String>,
}

impl GradientFlagValuesResolved {
    pub fn source_json(&self) -> serde_json::Value {
        serde_json::json!({
            "width": self.width.source,
            "height": self.height.source,
            "left": self.left.source,
            "right": self.right.source,
            "direction": self.direction.source,
        })
    }
}

#[derive(Clone, Debug)]
pub struct GlowFlagValuesResolved {
    pub width: Resolved<String>,
    pub height: Resolved<String>,
    pub color: Resolved<String>,
    pub intensity: Resolved<f32>,
}

impl GlowFlagValuesResolved {
    pub fn source_json(&self) -> serde_json::Value {
        serde_json::json!({
            "width": self.width.source,
            "height": self.height.source,
            "color": self.color.source,
            "intensity": self.intensity.source,
        })
    }
}

impl EnvConfig {
    pub fn from_process_env() -> Result<Self> {
        Self::from_iter(env::vars_os())
    }

    pub fn from_iter<I, K, V>(vars: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let entries: Vec<(OsString, OsString)> = vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let get = |name: &str| -> Option<String> {
            entries
                .iter()
                .find_map(|(k, v)| (k == name).then(|| v.to_string_lossy().to_string()))
        };
        let terminal_cols = match parse_opt(&get, "KITTUI_TERMINAL_COLS")? {
            Some(value) => Some(value),
            None => parse_opt(&get, "COLUMNS")?,
        };
        let terminal_rows = match parse_opt(&get, "KITTUI_TERMINAL_ROWS")? {
            Some(value) => Some(value),
            None => parse_opt(&get, "LINES")?,
        };
        Ok(Self {
            cache_dir: get_path(&get, "KITTUI_CACHE_DIR"),
            renderer: parse_opt(&get, "KITTUI_RENDERER")?,
            terminal_cols,
            terminal_rows,
            json: parse_opt_bool(&get, "KITTUI_JSON")?,
            box_: BoxConfig {
                x: parse_opt(&get, "KITTUI_BOX_X")?,
                y: parse_opt(&get, "KITTUI_BOX_Y")?,
                width: get("KITTUI_BOX_WIDTH"),
                height: get("KITTUI_BOX_HEIGHT"),
                fg: get("KITTUI_BOX_FG"),
                bg: get("KITTUI_BOX_BG"),
                radius: parse_opt(&get, "KITTUI_BOX_RADIUS")?,
                border: parse_opt(&get, "KITTUI_BOX_BORDER")?,
                animate: get("KITTUI_BOX_ANIMATE"),
            },
            gradient: GradientConfig {
                width: get("KITTUI_GRADIENT_WIDTH"),
                height: get("KITTUI_GRADIENT_HEIGHT"),
                left: get("KITTUI_GRADIENT_LEFT"),
                right: get("KITTUI_GRADIENT_RIGHT"),
                direction: get("KITTUI_GRADIENT_DIRECTION"),
            },
            glow: GlowConfig {
                width: get("KITTUI_GLOW_WIDTH"),
                height: get("KITTUI_GLOW_HEIGHT"),
                color: get("KITTUI_GLOW_COLOR"),
                intensity: parse_opt(&get, "KITTUI_GLOW_INTENSITY")?,
            },
            cache: CacheConfig {
                budget: parse_opt(&get, "KITTUI_CACHE_BUDGET")?,
            },
        })
    }
}

fn get_path(get: &impl Fn(&str) -> Option<String>, name: &str) -> Option<PathBuf> {
    get(name).map(PathBuf::from)
}

fn parse_opt<T>(get: &impl Fn(&str) -> Option<String>, name: &str) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    get(name)
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|err| anyhow!("invalid value for {name}: {err}"))
        })
        .transpose()
}

fn parse_opt_bool(get: &impl Fn(&str) -> Option<String>, name: &str) -> Result<Option<bool>> {
    get(name)
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(anyhow!(
                "invalid value for {name}: {value:?}; expected true/false"
            )),
        })
        .transpose()
}

fn load_yaml_config() -> Result<FileConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read kittui config at {}", path.display()))?;
    serde_yaml::from_slice(&bytes)
        .with_context(|| format!("failed to parse kittui config at {}", path.display()))
}

fn config_path() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("kittui/config.yaml");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".config/kittui/config.yaml");
    }
    PathBuf::from("kittui/config.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_is_flag_then_env_then_yaml_then_default() {
        let yaml = FileConfig {
            cache_dir: Some(PathBuf::from("yaml-cache")),
            terminal_cols: Some(100),
            ..FileConfig::default()
        };
        let env = EnvConfig::from_iter([
            ("KITTUI_CACHE_DIR", "env-cache"),
            ("KITTUI_TERMINAL_COLS", "120"),
        ])
        .unwrap();
        let layers = ConfigLayers::from_parts(yaml, env);
        let resolved = layers.resolve_global(GlobalFlagValues {
            cache_dir: Some(PathBuf::from("flag-cache")),
            ..GlobalFlagValues::default()
        });
        assert_eq!(resolved.cache_dir.value, PathBuf::from("flag-cache"));
        assert_eq!(resolved.cache_dir.source, ConfigSource::Flag);
        assert_eq!(resolved.terminal_cols.value, 120);
        assert_eq!(resolved.terminal_cols.source, ConfigSource::Env);
        assert_eq!(resolved.terminal_rows.value, 24);
        assert_eq!(resolved.terminal_rows.source, ConfigSource::Default);
    }

    #[test]
    fn env_parser_rejects_malformed_numbers() {
        let err = EnvConfig::from_iter([("KITTUI_GLOW_INTENSITY", "bright")]).unwrap_err();
        assert!(err.to_string().contains("KITTUI_GLOW_INTENSITY"));
    }

    #[test]
    fn yaml_rejects_unknown_fields() {
        let err = serde_yaml::from_str::<FileConfig>("unknown: true\n").unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }
}
