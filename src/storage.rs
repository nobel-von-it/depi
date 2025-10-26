use std::{collections::HashMap, env, fs, path::PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value as JValue;

pub struct AliasStorage {
    pub path: PathBuf,
    pub aliases: HashMap<String, String>,
}

impl AliasStorage {
    fn init_if_no_exist() -> Result<PathBuf> {
        let dir = get_storage_directory_by_os()?;
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }
        let aliases_path = dir.join("aliases.json");
        if !aliases_path.exists() {
            fs::File::create(&aliases_path)?;
        }
        Ok(aliases_path)
    }
    pub fn load() -> Result<Self> {
        let path = Self::init_if_no_exist()?;
        let mut aliases = HashMap::new();

        let content = fs::read_to_string(&path)?;
        if content.is_empty() {
            return Ok(Self { path, aliases });
        }

        let obj = serde_json::from_str::<JValue>(&content)?;
        if let JValue::Object(obj) = obj {
            for (k, v) in obj {
                if let JValue::String(v) = v {
                    aliases.insert(k, v);
                } else {
                    return Err(anyhow!(
                        "The alias storage is corrupted. Value is not String: {:?}",
                        v
                    ));
                }
            }
        }

        Ok(Self { path, aliases })
    }
    pub fn save(&self) -> Result<()> {
        let file = fs::File::options().write(true).open(&self.path)?;
        serde_json::to_writer(file, &self.aliases)?;
        Ok(())
    }

    pub fn get<S: AsRef<str>>(&self, k: S) -> Option<&String> {
        self.aliases.get(k.as_ref())
    }
    pub fn add<S: AsRef<str>>(&mut self, k: S, v: S) -> Option<String> {
        self.aliases
            .insert(k.as_ref().to_string(), v.as_ref().to_string())
    }
    pub fn rem<S: AsRef<str>>(&mut self, k: S) -> Option<String> {
        self.aliases.remove(k.as_ref())
    }
    pub fn list(&self) -> &HashMap<String, String> {
        &self.aliases
    }
}

fn get_storage_directory_by_os() -> Result<PathBuf> {
    match env::consts::OS {
        "linux" => {
            let full_path = format!("{}/.config/depi", env::var("HOME")?);
            Ok(PathBuf::from(full_path))
        }
        "windows" => {
            let full_path = format!("{}\\NERDINGS\\depi", env::var("APPDATA")?);
            Ok(PathBuf::from(full_path))
        }
        os => Err(anyhow!("OS {} is not supported", os)),
    }
}
