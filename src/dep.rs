use anyhow::{Result, anyhow};
use toml::{Table, Value as TValue, value::Array};

#[derive(Hash, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DType {
    Normal,
    Dev,
    Build,
    OS(String),
}

impl<S: AsRef<str>> From<S> for DType {
    fn from(s: S) -> Self {
        let s = s.as_ref().to_lowercase();
        match s.as_str() {
            "dev" => Self::Dev,
            "build" => Self::Build,
            "normal" => Self::Normal,
            ss if ss.trim().len() == 0 => Self::Normal,
            os => Self::OS(os.to_string()),
        }
    }
}

impl DType {
    pub fn to_cargo_field(&self) -> String {
        match self {
            DType::Normal => "dependencies".to_string(),
            DType::Dev => "dev-dependencies".to_string(),
            DType::Build => "build-dependencies".to_string(),
            DType::OS(os) => format!("target.'cfg({})'.dependencies", os),
        }
    }
}

impl ToString for DType {
    fn to_string(&self) -> String {
        (match self {
            DType::Normal => "normal",
            DType::Dev => "dev",
            DType::Build => "build",
            DType::OS(os) => os,
        })
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub version: String,
    pub features: Option<Vec<String>>,
}

impl Dep {
    pub fn from_toml<S: AsRef<str>>(name: S, attrs: TValue) -> Result<Self> {
        let name = name.as_ref();
        match attrs {
            TValue::String(version) => Ok(Self {
                name: name.to_string(),
                version: version.to_string(),
                features: None,
            }),
            TValue::Table(body) => {
                let version = if let Some(TValue::String(version)) = body.get("version") {
                    version.to_string()
                } else {
                    return Err(anyhow!("parse error: version"));
                };

                let mut fs = Vec::new();
                if let Some(TValue::Array(afs)) = body.get("features") {
                    for f in afs {
                        if let TValue::String(f) = f {
                            fs.push(f.to_string())
                        }
                    }
                } else {
                    return Err(anyhow!("parse error: features"));
                }

                Ok(Self {
                    name: name.to_string(),
                    version,
                    features: Some(fs),
                })
            }
            _ => Err(anyhow!("parse error: incorrect attrs type")),
        }
    }
    pub fn to_toml(&self) -> (String, TValue) {
        if let Some(fs) = &self.features {
            let mut body = Table::new();

            let mut afs = Array::new();
            for f in fs {
                afs.push(TValue::String(f.to_string()))
            }

            body.insert(
                "version".to_string(),
                TValue::String(self.version.to_string()),
            );
            body.insert("features".to_string(), TValue::Array(afs));

            (self.name.to_string(), TValue::Table(body))
        } else {
            (
                self.name.to_string(),
                TValue::String(self.version.to_string()),
            )
        }
    }
    pub async fn update_version(self) -> Result<Self> {
        let fd = api::fetch_crates_dep(&self.name).await?;
        let mut d = self;
        d.version = fd.get_last_version();
        Ok(d)
    }
}

pub fn normalize(pdep: &parse::PDep, fdep: &api::CratesDep) -> Result<Dep> {
    let name = fdep.name.to_string();
    let version = if pdep.version.is_empty() {
        fdep.get_last_version()
    } else if fdep.has_version(&pdep.version) {
        pdep.version.to_string()
    } else {
        return Err(anyhow!("invalid version"));
    };
    let features = if pdep.features.is_empty() {
        None
    } else if let Some(ffeatures) = fdep.get_features(&version) {
        let mut rfeatures = Vec::new();

        let pfeatures = pdep.features.split(',').collect::<Vec<_>>();
        let mut valid_features = true;
        let mut invalid_feat = String::new();
        for pfeat in pfeatures {
            if !ffeatures.contains(&pfeat.to_owned()) {
                valid_features = false;
                invalid_feat = pfeat.to_string();
                break;
            }
            rfeatures.push(pfeat.to_string());
        }
        if !valid_features {
            return Err(anyhow!("invalid features: {}", invalid_feat));
        }

        Some(rfeatures)
    } else {
        return Err(anyhow!("invalid features, strange"));
    };

    Ok(Dep {
        name,
        version,
        features,
    })
}

pub mod api {
    use crate::utils;

    use anyhow::Result;
    use serde_json::Value as JValue;
    use std::collections::HashMap;

    #[derive(Debug, Clone)]
    pub struct CratesDep {
        pub name: String,
        pub versions: HashMap<String, Vec<String>>,
    }

    impl CratesDep {
        pub fn get_last_version(&self) -> String {
            self.versions
                .iter()
                .filter_map(|(v, _)| utils::ver::OrdVersion::parse(v).ok())
                .max()
                .unwrap()
                .to_string()
        }
        pub fn has_version(&self, vs: &str) -> bool {
            self.versions.contains_key(vs)
        }
        pub fn get_features(&self, vs: &str) -> Option<&[String]> {
            self.versions.get(vs).map(|fs| fs.as_slice())
        }
    }

    pub async fn fetch_crates_dep<S: AsRef<str>>(name: S) -> Result<CratesDep> {
        let mut vhm = HashMap::new();

        let url = format!("https://crates.io/api/v1/crates/{}", name.as_ref());
        let cli = reqwest::Client::new();
        let body = cli
            .get(&url)
            .header("User-Agent", "depi/0.1.0")
            .send()
            .await?
            .text()
            .await?;

        let obj = serde_json::from_str::<JValue>(&body)?;

        if let JValue::Object(obj) = obj {
            if let Some(JValue::Array(arr)) = obj.get("versions") {
                for v in arr {
                    if let JValue::Object(vo) = v {
                        let num = if let Some(JValue::String(num)) = vo.get("num") {
                            num.to_string()
                        } else {
                            continue;
                        };
                        let mut features = Vec::new();
                        if let Some(JValue::Object(fo)) = vo.get("features") {
                            for (f, _) in fo {
                                features.push(f.to_string())
                            }
                        } else {
                            continue;
                        }

                        vhm.insert(num, features);
                    }
                }
            }
        }

        Ok(CratesDep {
            name: name.as_ref().to_string(),
            versions: vhm,
        })
    }
}

pub mod parse {
    use std::collections::HashMap;

    use anyhow::{Result, anyhow};

    #[derive(Debug, Clone)]
    pub struct PDep {
        pub name: String,
        pub version: String,
        pub features: String,
        pub target: String,
    }

    pub fn parse_deps<S: AsRef<str>>(s: S, aliases: &HashMap<String, String>) -> Result<Vec<PDep>> {
        let mut res = Vec::new();
        for d in s.as_ref().trim().split("/") {
            if let Some(alias_deps) = aliases.get(d) {
                for ad in alias_deps.split("/") {
                    res.push(parse_dep(ad));
                }
            } else {
                res.push(parse_dep(d));
            }
        }
        res.into_iter().collect()
    }

    pub fn parse_dep<S: AsRef<str>>(s: S) -> Result<PDep> {
        enum DPState {
            Name,
            Version,
            Features,
            Target,
        }

        let s = s.as_ref().trim();
        if s.is_empty() {
            return Err(anyhow!("provided empty string"));
        }

        let mut name = String::new();
        let mut version = String::new();
        let mut features = String::new();
        let mut target = String::new();

        let mut once_version = false;
        let mut once_features = false;
        let mut once_target = false;

        let mut state = DPState::Name;

        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '@' if matches!(state, DPState::Name)
                    && chars.peek().is_some()
                    && chars.peek().unwrap().is_alphanumeric()
                    && !once_version =>
                {
                    once_version = true;
                    state = DPState::Version;
                }
                ':' if (matches!(state, DPState::Name) || matches!(state, DPState::Version))
                    && chars.peek().is_some()
                    && chars.peek().unwrap().is_alphanumeric()
                    && !once_features =>
                {
                    once_features = true;
                    state = DPState::Features;
                }
                '!' if !matches!(state, DPState::Target)
                    && chars.peek().is_some()
                    && chars.peek().unwrap().is_alphanumeric()
                    && !once_target =>
                {
                    once_target = true;
                    state = DPState::Target;
                }

                c if (c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
                    && matches!(state, DPState::Version) =>
                {
                    version.push(c)
                }
                c if (c.is_alphanumeric() || c == ',' || c == '-' || c == '_')
                    && matches!(state, DPState::Features) =>
                {
                    features.push(c)
                }
                c if (c.is_alphanumeric() || c == '-' || c == '_')
                    && matches!(state, DPState::Name) =>
                {
                    name.push(c)
                }
                c if (c.is_alphanumeric() || c == '-' || c == '_')
                    && matches!(state, DPState::Target) =>
                {
                    target.push(c)
                }
                c if c.is_alphanumeric() => match state {
                    DPState::Name => name.push(c),
                    DPState::Version => version.push(c),
                    DPState::Features => features.push(c),
                    DPState::Target => target.push(c),
                },
                _ => {
                    return Err(anyhow!(
                        "parse error\ncurstate:\n  {}\n  {}\n  {}\n  {}\n  {}\n",
                        name,
                        version,
                        features,
                        target,
                        match state {
                            DPState::Name => "name",
                            DPState::Version => "version",
                            DPState::Features => "features",
                            DPState::Target => "target",
                        }
                    ));
                }
            }
        }
        Ok(PDep {
            name,
            version,
            features,
            target,
        })
    }
}
