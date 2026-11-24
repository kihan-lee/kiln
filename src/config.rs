use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    project: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    comments: Option<bool>,
    tests: Option<bool>,
}

#[derive(Default)]
pub struct Config {
    entries: Vec<(PathBuf, Entry)>,
    pub source: Option<PathBuf>,
    pub error: Option<String>,
}

fn expand(raw: &str) -> PathBuf {
    let home = env::var("HOME").unwrap_or_default();
    let path = match raw.strip_prefix("~/") {
        Some(rest) => Path::new(&home).join(rest),
        None => PathBuf::from(raw),
    };
    fs::canonicalize(&path).unwrap_or(path)
}

impl Config {
    pub fn path() -> PathBuf {
        env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env::var("HOME").unwrap_or_default()).join(".config"))
            .join("kiln")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let Ok(text) = fs::read_to_string(&path) else {
            return Config::default();
        };
        Self::parse(&text, path)
    }

    fn parse(text: &str, path: PathBuf) -> Self {
        match toml::from_str::<File>(text) {
            Ok(file) => Config {
                entries: file
                    .project
                    .into_iter()
                    .map(|e| (expand(&e.path), e))
                    .collect(),
                source: Some(path),
                error: None,
            },
            Err(e) => Config {
                error: Some(format!("{}: {}", path.display(), e.message())),
                source: Some(path),
                ..Config::default()
            },
        }
    }

    pub fn setting(&self, dirs: &[PathBuf], key: &str) -> Option<bool> {
        let dirs: Vec<PathBuf> = dirs
            .iter()
            .map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
            .collect();
        let value = |e: &Entry| match key {
            "comments" => e.comments,
            "tests" => e.tests,
            _ => None,
        };
        self.entries
            .iter()
            .filter(|(base, e)| value(e).is_some() && dirs.iter().any(|d| d.starts_with(base)))
            .max_by_key(|(base, _)| base.components().count())
            .and_then(|(_, e)| value(e))
    }
}

pub fn local(dir: &Path, key: &str) -> Option<bool> {
    ["pyproject.toml", "Cargo.toml"].iter().find_map(|name| {
        fs::read_to_string(dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .skip_while(|l| !(l.starts_with('[') && l.ends_with("kiln]")))
            .skip(1)
            .take_while(|l| !l.starts_with('['))
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| v.trim().parse().ok())
    })
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::path::PathBuf;

    #[test]
    fn most_specific_project_entry_wins() {
        let config = Config::parse(
            "[[project]]\npath = \"/srv\"\ncomments = true\n\n[[project]]\npath = \"/srv/app\"\ncomments = false\n",
            PathBuf::from("c.toml"),
        );
        let at = |p: &str| config.setting(&[PathBuf::from(p)], "comments");
        assert_eq!(at("/srv/app/pkg"), Some(false));
        assert_eq!(at("/srv/other"), Some(true));
        assert_eq!(at("/elsewhere"), None);
    }

    #[test]
    fn deeper_entry_without_the_key_does_not_hide_a_shallower_one() {
        let config = Config::parse(
            "[[project]]\npath = \"/srv\"\ncomments = false\n\n[[project]]\npath = \"/srv/app\"\ntests = true\n",
            PathBuf::from("c.toml"),
        );
        let app = [PathBuf::from("/srv/app")];
        assert_eq!(config.setting(&app, "comments"), Some(false));
        assert_eq!(config.setting(&app, "tests"), Some(true));
        assert_eq!(config.setting(&[PathBuf::from("/srv/web")], "tests"), None);
    }

    #[test]
    fn reports_unknown_keys() {
        let config = Config::parse(
            "[[project]]\npath = \"/srv\"\ncoments = false\n",
            PathBuf::from("c.toml"),
        );
        assert!(config.error.is_some_and(|e| e.contains("coments")));
    }
}
