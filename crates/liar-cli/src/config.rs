//! liar.toml.

use liar_core::check::CheckId;
use liar_core::messages::Tone;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not parse the configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("unknown check code '{0}'")]
    UnknownCheck(String),
    #[error("unknown tone '{0}', expected professional, dry or brutal")]
    UnknownTone(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The file as written. Separate from `Config` so `deny_unknown_fields` catches
/// typos and so string fields can be validated into real types.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawConfig {
    tone: Option<String>,
    select: Option<Vec<String>>,
    ignore: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    scope_threshold: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    pub tone: Tone,
    pub enabled: Vec<CheckId>,
    pub exclude: Vec<String>,
    /// C3e: how many lines a scope must span before an uninformative name in
    /// it is worth mentioning. Short names in short scopes are good style.
    pub scope_threshold: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tone: Tone::Dry,
            enabled: CheckId::ALL.to_vec(),
            exclude: Vec::new(),
            scope_threshold: 20,
        }
    }
}

impl Config {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(source)?;
        let defaults = Config::default();

        let tone = match raw.tone {
            Some(name) => name.parse().map_err(|_| ConfigError::UnknownTone(name))?,
            None => defaults.tone,
        };

        let parse_codes = |codes: Vec<String>| -> Result<Vec<CheckId>, ConfigError> {
            codes
                .into_iter()
                .map(|code| CheckId::from_code(&code).ok_or(ConfigError::UnknownCheck(code)))
                .collect()
        };

        let selected = match raw.select {
            Some(codes) => parse_codes(codes)?,
            None => defaults.enabled,
        };
        let ignored = match raw.ignore {
            Some(codes) => parse_codes(codes)?,
            None => Vec::new(),
        };

        let enabled = selected
            .into_iter()
            .filter(|c| !ignored.contains(c))
            .collect();

        Ok(Self {
            tone,
            enabled,
            exclude: raw.exclude.unwrap_or(defaults.exclude),
            scope_threshold: raw.scope_threshold.unwrap_or(defaults.scope_threshold),
        })
    }

    /// Loads from `path`, or returns defaults when `path` is `None`.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            None => Ok(Config::default()),
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
                Config::from_toml(&text)
            }
        }
    }

    pub fn is_enabled(&self, check: CheckId) -> bool {
        self.enabled.contains(&check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_dry_tone_and_every_check_enabled() {
        let config = Config::default();
        assert_eq!(config.tone, Tone::Dry);
        for &check in CheckId::ALL {
            assert!(
                config.is_enabled(check),
                "{} should be enabled",
                check.code()
            );
        }
    }

    #[test]
    fn an_empty_config_equals_the_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn tone_is_read_from_the_file() {
        let config = Config::from_toml(r#"tone = "brutal""#).unwrap();
        assert_eq!(config.tone, Tone::Brutal);
    }

    #[test]
    fn an_invalid_tone_is_rejected() {
        let err = Config::from_toml(r#"tone = "sarcastic""#).expect_err("expected an error");
        assert!(err.to_string().contains("sarcastic"));
    }

    #[test]
    fn select_restricts_the_enabled_checks() {
        let config = Config::from_toml(r#"select = ["C1", "C4"]"#).unwrap();
        assert!(config.is_enabled(CheckId::C1));
        assert!(config.is_enabled(CheckId::C4));
        assert!(!config.is_enabled(CheckId::C2));
    }

    #[test]
    fn ignore_removes_checks_from_the_default_set() {
        let config = Config::from_toml(r#"ignore = ["C5"]"#).unwrap();
        assert!(!config.is_enabled(CheckId::C5));
        assert!(config.is_enabled(CheckId::C1));
    }

    #[test]
    fn ignore_applies_after_select() {
        let config = Config::from_toml(
            r#"
            select = ["C1", "C4"]
            ignore = ["C4"]
            "#,
        )
        .unwrap();
        assert!(config.is_enabled(CheckId::C1));
        assert!(!config.is_enabled(CheckId::C4));
    }

    #[test]
    fn an_unknown_check_code_is_rejected() {
        let err = Config::from_toml(r#"select = ["C99"]"#).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        // A silently ignored typo is a setting the user believes is applied.
        let err = Config::from_toml(r#"toen = "dry""#).expect_err("expected an error");
        assert!(err.to_string().contains("toen"));
    }

    #[test]
    fn the_scope_threshold_is_configurable() {
        let config = Config::from_toml("scope-threshold = 40").unwrap();
        assert_eq!(config.scope_threshold, 40);
        assert_eq!(Config::default().scope_threshold, 20);
    }

    #[test]
    fn exclude_globs_are_read() {
        let config = Config::from_toml(r#"exclude = ["tests/**", "build/*"]"#).unwrap();
        assert_eq!(
            config.exclude,
            vec!["tests/**".to_string(), "build/*".to_string()]
        );
    }
}
