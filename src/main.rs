use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use futures::future;
use serde_json::{Value, from_str};
use std::collections::HashMap;

#[derive(Debug, Parser)]
struct DepiCommand {
    #[clap(required = true)]
    deps: String,
}

#[derive(PartialOrd, Ord, PartialEq, Eq, Clone, Debug, Default)]
struct OrdVersion(u32, u32, u32);
impl OrdVersion {
    fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        let mut res = Self::default();

        let mut s = s.as_ref();

        if let Some((left, _)) = s.split_once("-") {
            s = left.trim();
        }
        let start = s.chars().nth(0).unwrap();
        if !start.is_ascii_digit() {
            s = s.trim_start_matches(start);
        }

        let s = s.split(".").collect::<Vec<_>>();

        match s.len() {
            1 => res.0 = s[0].parse()?,
            2 => {
                res.0 = s[0].parse()?;
                res.1 = s[1].parse()?;
            }
            3 => {
                res.0 = s[0].parse()?;
                res.1 = s[1].parse()?;
                res.2 = s[2].parse()?;
            }
            _ => return Err(anyhow!("invalid parts {}", s.len())),
        }
        Ok(res)
    }
}
impl ToString for OrdVersion {
    fn to_string(&self) -> String {
        format!("{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone)]
struct CratesDep {
    name: String,
    versions: HashMap<String, Vec<String>>,
}

impl CratesDep {
    fn get_last_version(&self) -> String {
        self.versions
            .iter()
            .filter_map(|(v, _)| OrdVersion::parse(v).map_err(|e| eprintln!("{e:#?}")).ok())
            .max()
            .unwrap()
            .to_string()
    }
    fn has_version(&self, vs: &str) -> bool {
        self.versions.contains_key(vs)
    }
    fn get_features(&self, vs: &str) -> Option<&[String]> {
        self.versions.get(vs).map(|fs| fs.as_slice())
    }
}

async fn fetch_crates_dep<S: AsRef<str>>(name: S) -> Result<CratesDep> {
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

    let obj = from_str::<Value>(&body)?;

    if let Value::Object(obj) = obj {
        if let Some(Value::Array(arr)) = obj.get("versions") {
            for v in arr {
                if let Value::Object(vo) = v {
                    let num = if let Some(Value::String(num)) = vo.get("num") {
                        num.to_string()
                    } else {
                        continue;
                    };
                    let mut features = Vec::new();
                    if let Some(Value::Object(fo)) = vo.get("features") {
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

#[derive(Debug, Clone)]
struct PDep {
    name: String,
    version: String,
    features: String,
    target: String,
}

fn parse_dep<S: AsRef<str>>(s: S) -> Result<PDep> {
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

    let mut once_name = false;
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

#[derive(Debug, Clone)]
enum DTarget {
    Normal,
    Dev,
    Build,
    OS(String),
}

impl<S: AsRef<str>> From<S> for DTarget {
    fn from(s: S) -> Self {
        let s = s.as_ref();
        match s {
            "normal" => Self::Normal,
            "dev" => Self::Dev,
            "build" => Self::Build,
            os => Self::OS(os.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct Dep {
    name: String,
    version: String,
    features: Option<Vec<String>>,
    target: DTarget,
}

impl Dep {
    fn to_cargo_str(&self) -> String {
        match &self.features {
            Some(fs) => format!(
                "{} = {{ version = \"{}\", features = [{}] }}",
                &self.name,
                &self.version,
                fs.iter()
                    .map(|f| format!("\"{f}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => format!("{} = \"{}\"", &self.name, &self.version),
        }
    }
}

fn get_normal_dep(pdep: &PDep, fdep: &CratesDep) -> Result<Dep> {
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
    let target = if pdep.target.is_empty() {
        DTarget::Normal
    } else {
        DTarget::from(&pdep.target)
    };

    Ok(Dep {
        name,
        version,
        features,
        target,
    })
}

#[tokio::main]
async fn main() {
    let com = DepiCommand::parse();
    let deps = com
        .deps
        .split('/')
        .filter_map(|d| parse_dep(d).map_err(|e| println!("{e:#?}")).ok())
        .collect::<Vec<_>>();

    let futures = deps
        .iter()
        .map(|d| fetch_crates_dep(&d.name))
        .collect::<Vec<_>>();

    let ress = (future::join_all(futures).await)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    if deps.len() != ress.len() {
        eprintln!("bruh length {} != {}", deps.len(), ress.len());
        return;
    }

    for i in 0..deps.len() {
        if let Ok(dep) = get_normal_dep(&deps[i], &ress[i]) {
            println!("{}", dep.to_cargo_str());
        }
    }
}
