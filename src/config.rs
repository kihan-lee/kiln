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
    docs: Option<Vec<String>>,
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

    fn matching<'a>(&'a self, dirs: &[PathBuf]) -> impl Iterator<Item = &'a (PathBuf, Entry)> {
        let dirs: Vec<PathBuf> = dirs
            .iter()
            .map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
            .collect();
        self.entries
            .iter()
            .filter(move |(base, _)| dirs.iter().any(|d| d.starts_with(base)))
    }

    pub fn docs(&self, dirs: &[PathBuf]) -> Option<Vec<String>> {
        self.matching(dirs)
            .filter(|(_, e)| e.docs.is_some())
            .max_by_key(|(base, _)| base.components().count())
            .and_then(|(_, e)| e.docs.clone())
    }

    pub fn setting(&self, dirs: &[PathBuf], key: &str) -> Option<bool> {
        let value = |e: &Entry| match key {
            "comments" => e.comments,
            "tests" => e.tests,
            _ => None,
        };
        self.matching(dirs)
            .filter(|(_, e)| value(e).is_some())
            .max_by_key(|(base, _)| base.components().count())
            .and_then(|(_, e)| value(e))
    }
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Custom {
    pub name: String,
    pub run: String,
    #[serde(default)]
    pub hook: bool,
    pub exts: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct Kiln {
    #[serde(default)]
    check: Vec<Custom>,
    docs: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct Holder {
    #[serde(default)]
    kiln: Kiln,
}

#[derive(Deserialize, Default)]
struct Package {
    #[serde(default)]
    metadata: Holder,
}

#[derive(Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    tool: Holder,
    #[serde(default)]
    package: Package,
}

fn read_kiln(dir: &Path) -> Result<Kiln, String> {
    let mut found = Kiln::default();
    for name in ["pyproject.toml", "Cargo.toml"] {
        let text = fs::read_to_string(dir.join(name)).unwrap_or_default();
        if !text.contains("kiln") {
            continue;
        }
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| format!("{name}: {}", e.message()))?;
        for kiln in [manifest.tool.kiln, manifest.package.metadata.kiln] {
            found.check.extend(kiln.check);
            found.docs = found.docs.or(kiln.docs);
        }
    }
    Ok(found)
}

pub fn custom_checks(dir: &Path) -> Result<Vec<Custom>, String> {
    read_kiln(dir).map(|k| k.check)
}

pub fn local_docs(dir: &Path) -> Option<Vec<String>> {
    read_kiln(dir).ok().and_then(|k| k.docs)
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
    fn reads_custom_checks_from_either_manifest() {
        let base = std::env::temp_dir().join(format!("kiln-custom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[[package.metadata.kiln.check]]\nname = \"tla\"\nrun = \"tlc a.tla\"\nhook = true\nexts = [\"tla\"]\n",
        )
        .unwrap();
        let found = super::custom_checks(&base).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].hook && found[0].run == "tlc a.tla");

        std::fs::write(
            base.join("pyproject.toml"),
            "[[tool.kiln.check]]\nname = \"e2e\"\nrun = \"x\"\nwhen = \"hook\"\n",
        )
        .unwrap();
        let error = super::custom_checks(&base).err().unwrap_or_default();
        assert!(error.contains("when"), "{error}");
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
