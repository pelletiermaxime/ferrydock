//! Local command recipes. Loading and mutations run on the feature's file worker.
use crate::model::Container;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FAVORITES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Service { project: String, service: String },
    Container { name: String },
}
impl Target {
    pub fn matches(&self, container: &Container) -> bool {
        match self {
            Self::Service { project, service } => {
                container.project.as_ref() == Some(project)
                    && container.service.as_ref() == Some(service)
            }
            Self::Container { name } => container.project.is_none() && &container.name == name,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Service { project, service } => format!("{project} / {service}"),
            Self::Container { name } => name.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Favorite {
    pub id: u64,
    pub endpoint: String,
    pub target: Target,
    pub label: String,
    pub command: String,
}
impl Favorite {
    pub fn validate(&self) -> Result<()> {
        if self.label.trim().is_empty() || self.label.len() > 120 {
            bail!("Enter a label of 1–120 bytes");
        }
        if self.command.trim().is_empty()
            || self.command.len() > 16 * 1024
            || self
                .command
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            bail!("Enter a command of 1–16384 bytes without terminal control characters");
        }
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub favorites: Vec<Favorite>,
}
pub enum Mutation {
    Save(Favorite),
    Delete(u64),
}
impl Store {
    pub fn path() -> Result<PathBuf> {
        Ok(crate::compose::Settings::path()?.with_file_name("favorites.json"))
    }
    pub fn load(path: &Path) -> Result<Self> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Favorites file exceeds 2 MiB");
        }
        let store: Self =
            serde_json::from_slice(&bytes).context("Cannot read saved favorite commands")?;
        if store.favorites.len() > MAX_FAVORITES {
            bail!("Too many favorite commands");
        }
        let mut ids = std::collections::HashSet::new();
        for favorite in &store.favorites {
            favorite.validate()?;
            if favorite.id == 0 || !ids.insert(favorite.id) {
                bail!("Invalid favorite command IDs");
            }
        }
        Ok(store)
    }
    pub fn mutate(path: &Path, mutation: Mutation) -> Result<Self> {
        // Preserve unreadable files and reload so unrelated entries survive.
        let mut store = Self::load(path)?;
        match mutation {
            Mutation::Save(mut favorite) => {
                favorite.validate()?;
                if favorite.id == 0 {
                    if store.favorites.len() >= MAX_FAVORITES {
                        bail!("Favorite command limit reached");
                    }
                    favorite.id = store
                        .favorites
                        .iter()
                        .map(|f| f.id)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .context("Favorite IDs exhausted")?;
                    store.favorites.push(favorite);
                } else {
                    let existing = store
                        .favorites
                        .iter_mut()
                        .find(|f| f.id == favorite.id)
                        .context("Favorite no longer exists")?;
                    if existing.endpoint != favorite.endpoint || existing.target != favorite.target
                    {
                        bail!("Favorite scope changed");
                    }
                    *existing = favorite;
                }
            }
            Mutation::Delete(id) => store.favorites.retain(|f| f.id != id),
        }
        let parent = path.parent().context("Favorites path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(&store)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Favorites file exceeds 2 MiB");
        }
        // Private atomic replacement: command recipes may contain credentials.
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        std::io::Write::write_all(&mut temporary, &bytes)?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recipe(endpoint: &str) -> Favorite {
        Favorite {
            id: 0,
            endpoint: endpoint.into(),
            target: Target::Service {
                project: "demo".into(),
                service: "api".into(),
            },
            label: "Tests".into(),
            command: "printf '%s\\n' \"$PWD\"; read answer".into(),
        }
    }
    #[test]
    fn persistence_edits_deletes_and_keeps_other_endpoints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("favorites.json");
        Store::mutate(&path, Mutation::Save(recipe("unix:///a"))).unwrap();
        let mut store = Store::mutate(&path, Mutation::Save(recipe("unix:///b"))).unwrap();
        let mut favorite = store.favorites.remove(0);
        favorite.label = "Updated".into();
        favorite.command = "echo one\necho two".into();
        let store = Store::mutate(&path, Mutation::Save(favorite)).unwrap();
        assert_eq!(store.favorites[0].command, "echo one\necho two");
        let store = Store::mutate(&path, Mutation::Delete(store.favorites[0].id)).unwrap();
        assert_eq!(store.favorites.len(), 1);
        assert_eq!(store.favorites[0].endpoint, "unix:///b");
    }
    #[test]
    fn malformed_file_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("favorites.json");
        std::fs::write(&path, b"broken").unwrap();
        assert!(Store::mutate(&path, Mutation::Save(recipe("unix:///a"))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        let mut favorite = recipe("a");
        favorite.command = "echo\u{1b}[0m".into();
        assert!(favorite.validate().is_err());
    }
}
