//! The tool's voice, in one place.
//!
//! Every check's message lives in `data/messages.toml`, three tones each.
//! Keeping them together means the voice can be reviewed in one sitting;
//! scattering them through the checkers is how a tool with personality drifts
//! into inconsistency.

use crate::check::CheckId;
use crate::finding::Finding;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Tone {
    /// Usable at work without explaining yourself.
    Professional,
    /// Deadpan and specific. The default.
    #[default]
    Dry,
    /// Blunter. Still true.
    Brutal,
}

impl Tone {
    fn key(self) -> &'static str {
        match self {
            Tone::Professional => "professional",
            Tone::Dry => "dry",
            Tone::Brutal => "brutal",
        }
    }

    pub const ALL: &'static [Tone] = &[Tone::Professional, Tone::Dry, Tone::Brutal];
}

impl FromStr for Tone {
    type Err = MessageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "professional" => Ok(Tone::Professional),
            "dry" => Ok(Tone::Dry),
            "brutal" => Ok(Tone::Brutal),
            other => Err(MessageError::UnknownTone(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("unknown tone '{0}', expected professional, dry or brutal")]
    UnknownTone(String),
    #[error("unknown check code '{0}'")]
    UnknownCheck(String),
    #[error("check {check} has no '{tone}' message")]
    MissingTone { check: String, tone: String },
    #[error("message for {check} needs a value for '{placeholder}' but none was supplied")]
    MissingArgument { check: String, placeholder: String },
    #[error("could not parse the message table: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug)]
pub struct MessageTable {
    templates: HashMap<(CheckId, Tone), String>,
}

const EMBEDDED: &str = include_str!("../../../data/messages.toml");

impl MessageTable {
    /// The table compiled into the binary, so the tool has a voice without
    /// needing its data directory alongside it.
    pub fn embedded() -> &'static MessageTable {
        static TABLE: OnceLock<MessageTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            MessageTable::load(EMBEDDED).expect("the embedded message table must be valid")
        })
    }

    pub fn load(source: &str) -> Result<Self, MessageError> {
        let raw: HashMap<String, HashMap<String, String>> = toml::from_str(source)?;
        let mut templates = HashMap::new();

        for (code, tones) in raw {
            let check = CheckId::from_code(&code)
                .ok_or_else(|| MessageError::UnknownCheck(code.clone()))?;

            for &tone in Tone::ALL {
                let template = tones
                    .get(tone.key())
                    .ok_or_else(|| MessageError::MissingTone {
                        check: code.clone(),
                        tone: tone.key().to_string(),
                    })?;
                templates.insert((check, tone), template.clone());
            }
        }

        Ok(Self { templates })
    }

    pub fn template(&self, check: CheckId, tone: Tone) -> Option<&str> {
        self.templates.get(&(check, tone)).map(String::as_str)
    }

    /// # Panics
    /// If a placeholder has no matching argument. Use [`MessageTable::try_render`]
    /// where that is recoverable; inside the engine it is a bug in the check.
    pub fn render(&self, finding: &Finding, tone: Tone) -> String {
        self.try_render(finding, tone)
            .expect("message rendering failed")
    }

    pub fn try_render(&self, finding: &Finding, tone: Tone) -> Result<String, MessageError> {
        let template =
            self.template(finding.check, tone)
                .ok_or_else(|| MessageError::MissingTone {
                    check: finding.check.code().to_string(),
                    tone: tone.key().to_string(),
                })?;

        let mut out = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let close = after
                .find('}')
                .ok_or_else(|| MessageError::MissingArgument {
                    check: finding.check.code().to_string(),
                    placeholder: after.to_string(),
                })?;
            let placeholder = &after[..close];

            let value = finding
                .args
                .iter()
                .find(|(key, _)| key == placeholder)
                .map(|(_, value)| value.as_str())
                .ok_or_else(|| MessageError::MissingArgument {
                    check: finding.check.code().to_string(),
                    placeholder: placeholder.to_string(),
                })?;

            out.push_str(value);
            rest = &after[close + 1..];
        }
        out.push_str(rest);

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{FileId, Id};
    use crate::span::Span;

    fn finding(check: CheckId, args: &[(&str, &str)]) -> Finding {
        let mut f = Finding::new(check, FileId::from_index(0), Span::new(0, 1));
        for (key, value) in args {
            f = f.with_arg(*key, *value);
        }
        f
    }

    #[test]
    fn every_check_has_every_tone() {
        // The guard that stops a new check shipping with a missing voice.
        let table = MessageTable::embedded();
        for &check in CheckId::ALL {
            for &tone in Tone::ALL {
                assert!(
                    table.template(check, tone).is_some(),
                    "{} has no {tone:?} message",
                    check.code()
                );
            }
        }
    }

    #[test]
    fn no_template_is_empty() {
        let table = MessageTable::embedded();
        for &check in CheckId::ALL {
            for &tone in Tone::ALL {
                assert!(!table.template(check, tone).unwrap().trim().is_empty());
            }
        }
    }

    #[test]
    fn placeholders_are_interpolated() {
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data"), ("n", "4"), ("k", "3")]);
        let rendered = table.render(&f, Tone::Dry);
        assert_eq!(
            rendered,
            "4 variables called 'data' in this file. none are related."
        );
    }

    #[test]
    fn each_tone_renders_differently() {
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data"), ("n", "4"), ("k", "3")]);
        let professional = table.render(&f, Tone::Professional);
        let dry = table.render(&f, Tone::Dry);
        let brutal = table.render(&f, Tone::Brutal);
        assert_ne!(professional, dry);
        assert_ne!(dry, brutal);
        assert_ne!(professional, brutal);
    }

    #[test]
    fn a_missing_argument_is_an_error_not_a_silent_blank() {
        // A message reading "variables called  in this file" would ship
        // unnoticed. Failing loudly in tests is the point.
        let table = MessageTable::embedded();
        let f = finding(CheckId::C3f, &[("name", "data")]); // n missing
        let err = table
            .try_render(&f, Tone::Dry)
            .expect_err("expected an error");
        assert!(
            err.to_string().contains('n'),
            "error should name the missing placeholder"
        );
    }

    #[test]
    fn loading_rejects_a_check_with_a_missing_tone() {
        let toml = r#"
            [C1]
            professional = "a"
            dry = "b"
        "#;
        let err = MessageTable::load(toml).expect_err("expected an error");
        assert!(err.to_string().contains("brutal"));
    }

    #[test]
    fn loading_rejects_an_unknown_check_code() {
        let toml = r#"
            [C99]
            professional = "a"
            dry = "b"
            brutal = "c"
        "#;
        let err = MessageTable::load(toml).expect_err("expected an error");
        assert!(err.to_string().contains("C99"));
    }

    #[test]
    fn tone_parses_from_its_name() {
        assert_eq!("dry".parse::<Tone>().unwrap(), Tone::Dry);
        assert_eq!("professional".parse::<Tone>().unwrap(), Tone::Professional);
        assert_eq!("brutal".parse::<Tone>().unwrap(), Tone::Brutal);
        assert!("sarcastic".parse::<Tone>().is_err());
    }

    #[test]
    fn the_default_tone_is_dry() {
        assert_eq!(Tone::default(), Tone::Dry);
    }

    #[test]
    fn every_embedded_template_renders_with_its_checks_arguments() {
        // Catches a template referring to a placeholder no checker supplies,
        // which would otherwise only surface the first time that check fired
        // on real code.
        let table = MessageTable::embedded();
        let args: &[(&str, &str)] = &[
            ("name", "x"),
            ("ty", "str"),
            ("n", "4"),
            ("k", "3"),
            ("lines", "80"),
            ("target", "self.cache"),
            ("detail", "it returns None"),
            ("released", "1"),
            ("total", "3"),
            ("name_number", "plural"),
            ("value_number", "singular"),
        ];
        for &check in CheckId::ALL {
            for &tone in Tone::ALL {
                let f = finding(check, args);
                table
                    .try_render(&f, tone)
                    .unwrap_or_else(|e| panic!("{} {tone:?}: {e}", check.code()));
            }
        }
    }
}
