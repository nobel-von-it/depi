use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use colored::*;
use futures::future;
use serde_json::{Value as JValue, from_str};
use toml::{Table, Value as TValue, value::Array};

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

const MAIN: &str = r#"
fn main() {
    println!("Hello Depi!");
}
"#;

struct Printer;
impl Printer {
    fn added_many(added: &[(String, String, bool)]) {
        println!("{}", "DEPS ADD".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        let mnl = added.iter().map(|(n, _, _)| n.len()).max().unwrap();
        let mvl = added
            .iter()
            .map(|(_, v, _)| v.to_string().len())
            .max()
            .unwrap();
        for (name, version, is_features) in added {
            println!(
                "{:<mnl$} {:<mvl$} {}",
                name.bold(),
                version.yellow().bold(),
                if *is_features {
                    "with features".dimmed()
                } else {
                    "without features".dimmed()
                }
            );
        }

        println!("{}", "=".repeat(40).cyan());
    }
    fn not_added<S: AsRef<str>>(name: S) {
        println!("{}", "DEP ADD".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        println!("{} already in the Cargo file", name.as_ref().bold().red());

        println!("{}", "=".repeat(40).cyan());
    }
    fn added<S: AsRef<str>>(name: S) {
        println!("{}", "DEPS ADD".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        println!(
            "{} succfully add to the Cargo file",
            name.as_ref().bold().green()
        );

        println!("{}", "=".repeat(40).cyan());
    }
    fn print_updates(updates: &[(&str, &str, &str)]) {
        println!("{}", "DEPS UP".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        let mnl = updates.iter().map(|(n, _, _)| n.len()).max().unwrap();
        let mfl = updates
            .iter()
            .map(|(_, f, _)| f.to_string().len())
            .max()
            .unwrap();
        for (name, from, to) in updates {
            println!(
                "  {:<mnl$} {:<mfl$} -> {}",
                name.bold(),
                from.dimmed(),
                to.yellow().bold()
            );
        }

        println!("{}", "=".repeat(40).cyan());

        println!(
            "{}",
            format!("updated {} dep(s)", updates.len()).green().bold()
        )
    }
    fn print_init_deps(deps: &[(String, String, bool)]) {
        println!("{}", "INIT DEPS IN CARGO".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        let mnl = deps.iter().map(|(n, _, _)| n.len()).max().unwrap();
        let mvl = deps
            .iter()
            .map(|(_, v, _)| v.to_string().len())
            .max()
            .unwrap();
        for (name, version, is_features) in deps {
            println!(
                "{:<mnl$} {:<mvl$} {}",
                name.bold(),
                version.yellow().bold(),
                if *is_features {
                    "with features".dimmed()
                } else {
                    "without features".dimmed()
                }
            );
        }

        println!("{}", "=".repeat(40).cyan());
    }
}

#[derive(Debug, Parser)]
enum DepiCommand {
    Init {
        #[clap(short = 'D', long, required = true)]
        deps: String,
        #[clap(short, long)]
        verbose: bool,
    },
    New {
        #[clap(required = true)]
        name: String,
        #[clap(short = 'D', long, required = true)]
        deps: String,
        #[clap(short, long)]
        verbose: bool,
    },
    Add {
        #[clap(required = true)]
        deps: String,
        #[clap(short, long)]
        verbose: bool,
    },
    Remove {
        #[clap(required = true)]
        names: String,
        #[clap(short, long)]
        verbose: bool,
    },
    List {
        #[clap(short, long)]
        verbose: bool,
    },
    Update {
        #[clap(short, long)]
        verbose: bool,
    },
}

struct Cargo(PathBuf);

impl Cargo {
    async fn update_deps(&self, verbose: bool) -> Result<()> {
        let content = fs::read_to_string(&self.0)?;
        if verbose {
            println!("INFO: start updating {} file", self.0.display())
        }
        let content = content.parse::<Table>()?;

        let mut newc = Table::new();
        let mut dfutures = Vec::new();
        let mut old_versions = Vec::new();
        for (k, v) in &content {
            match k.as_str() {
                "dependencies" => {
                    if let TValue::Table(deps) = v {
                        for (dk, dv) in deps {
                            let d = Dep::from_toml(dk, dv.clone())?;
                            old_versions.push(d.version.clone());
                            dfutures.push(d.update_version());
                        }
                    }
                }
                k => {
                    newc.insert(
                        k.to_string(),
                        content.get(k).ok_or(anyhow!("invalid cargo"))?.clone(),
                    );
                }
            }
        }

        let dfl = dfutures.len();
        let udeps = (future::join_all(dfutures).await)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        if dfl != udeps.len() {
            return Err(anyhow!("error with updating some deps"));
        }

        let mut updates = Vec::new();
        let mut deps = Table::new();
        for i in 0..dfl {
            if &old_versions[i] < &udeps[i].version {
                updates.push((
                    udeps[i].name.as_str(),
                    old_versions[i].as_str(),
                    udeps[i].version.as_str(),
                ));
            }
            let (name, attrs) = udeps[i].to_toml();
            deps.insert(name, attrs);
        }

        if updates.len() == 0 {
            println!("no need to be updated");
            return Ok(());
        }

        Printer::print_updates(&updates);

        newc.insert("dependencies".to_string(), TValue::Table(deps));

        fs::write(&self.0, toml::to_string(&newc)?)?;

        Ok(())
    }
    async fn init_project<S: AsRef<str>>(deps: S, verbose: bool) -> Result<String> {
        let mut newc = Table::new();

        let mut project = Table::new();
        let project_name = fs::canonicalize(env::current_dir().unwrap_or(".".into()))?
            .file_name()
            .ok_or(anyhow!("failed file_name()"))?
            .to_str()
            .ok_or(anyhow!("failed to_str()"))?
            .to_string();
        project.insert("name".to_string(), TValue::String(project_name));
        project.insert("version".to_string(), TValue::String("0.1.0".to_string()));
        project.insert("edition".to_string(), TValue::String("2024".to_string()));

        newc.insert("project".to_string(), TValue::Table(project));

        let pdeps = parse_deps(deps)?;
        let mut fdeps = Vec::new();
        for pd in &pdeps {
            fdeps.push(fetch_crates_dep(&pd.name));
        }

        let fdl = fdeps.len();
        let fdeps = (future::join_all(fdeps).await)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        if fdl != fdeps.len() {
            return Err(anyhow!("error with fetching some deps"));
        }

        let mut depst = Table::new();
        let mut depsafp = Vec::new();
        for i in 0..fdl {
            let d = get_normal_dep(&pdeps[i], &fdeps[i])?;

            depsafp.push((d.name.clone(), d.version.clone(), d.features.is_some()));
            let (name, attrs) = d.to_toml();
            depst.insert(name, attrs);
        }

        Printer::print_init_deps(&depsafp);

        newc.insert("dependencies".to_string(), TValue::Table(depst));
        let newc = toml::to_string(&newc)?;

        Ok(newc)
    }
    async fn append_deps<S: AsRef<str>>(&self, deps: S, verbose: bool) -> Result<()> {
        let content = fs::read_to_string(&self.0)?;
        let mut content = content.parse::<Table>()?;

        if let Some(TValue::Table(tdeps)) = content.get_mut("dependencies") {
            let mut pdeps = parse_deps(deps)?;
            let mut fdeps = Vec::new();
            for pd in &pdeps {
                fdeps.push(fetch_crates_dep(&pd.name));
            }

            let fdl = fdeps.len();
            let fdeps = (future::join_all(fdeps).await)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

            if fdl != fdeps.len() {
                return Err(anyhow!("error with fetching some deps"));
            }

            let mut adeps = Vec::new();
            for i in 0..fdl {
                let d = get_normal_dep(&pdeps[i], &fdeps[i])?;
                let (name, attrs) = d.to_toml();
                adeps.push((d.name.clone(), d.version.clone(), d.features.is_some()));
                tdeps.insert(name, attrs);
            }

            Printer::added_many(&adeps);
        }

        Ok(())
    }
    // async fn append_dep(&self, dep: &Dep) -> Result<()> {
    //     let content = fs::read_to_string(&self.0)?;
    //     let content = content.parse::<Table>()?;
    //
    //     let mut newc = Table::new();
    //     let mut dfutures = Vec::new();
    //     let mut old_versions = Vec::new();
    //     for (k, v) in &content {
    //         match k.as_str() {
    //             "dependencies" => {
    //                 if let TValue::Table(deps) = v {
    //                     let (name, attrs) = dep.to_toml();
    //                     if let Some(d) = deps.insert(name, attrs) {
    //                         Printer::not_added(&dep.name);
    //                     } else {
    //                         Printer::added(&dep.name);
    //                     }
    //                     // for (dk, dv) in deps {
    //                     //     let d = Dep::from_toml(dk, dv.clone())?;
    //                     //     old_versions.push(d.version.clone());
    //                     //     dfutures.push(d.update_version());
    //                     // }
    //                 }
    //             }
    //             k => {
    //                 newc.insert(
    //                     k.to_string(),
    //                     content.get(k).ok_or(anyhow!("invalid cargo"))?.clone(),
    //                 );
    //             }
    //         }
    //     }
    //
    //     let dfl = dfutures.len();
    //     let udeps = (future::join_all(dfutures).await)
    //         .into_iter()
    //         .flatten()
    //         .collect::<Vec<_>>();
    //
    //     Ok(())
    // }

    fn from_cur() -> Result<Self> {
        let cf = Self::find_cargo_file(Path::new("."))?;
        Ok(Self(cf))
    }
    fn find_cargo_file<P: AsRef<Path>>(path: P) -> Result<PathBuf> {
        let path = path.as_ref();
        let files = fs::read_dir(path)?;
        for file in files.into_iter().flatten() {
            if file.path().display().to_string().contains("Cargo.toml") {
                return Ok(file.path());
            }
        }
        if let Some(parent) = path.parent() {
            return Self::find_cargo_file(parent);
        }
        Err(anyhow!("cargo not found"))
    }
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

    let obj = from_str::<JValue>(&body)?;

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

#[derive(Debug, Clone)]
struct PDep {
    name: String,
    version: String,
    features: String,
    target: String,
}

fn parse_deps<S: AsRef<str>>(s: S) -> Result<Vec<PDep>> {
    s.as_ref().trim().split("/").map(|d| parse_dep(d)).collect()
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
    fn from_toml<S: AsRef<str>>(name: S, attrs: TValue) -> Result<Self> {
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
    fn to_toml(&self) -> (String, TValue) {
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
    async fn update_version(self) -> Result<Self> {
        let fd = fetch_crates_dep(&self.name).await?;
        let mut d = self;
        d.version = fd.get_last_version();
        Ok(d)
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

    Ok(Dep {
        name,
        version,
        features,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let com = DepiCommand::parse();
    match com {
        DepiCommand::Update { verbose } => {
            let cp = Cargo::from_cur()?;
            cp.update_deps(verbose).await?;
        }
        DepiCommand::Init { deps, verbose } => {
            // let cp = Cargo::from_cur()?;
            let cs = Cargo::init_project(deps, verbose).await?;

            let mut f = fs::File::create("Cargo.toml")?;
            f.write_all(cs.as_bytes())?;

            fs::create_dir("src")?;
            let mut f = fs::File::create("src/main.rs")?;
            f.write_all(MAIN.as_bytes())?;

            let gout = process::Command::new("git").arg("init").output()?.stdout;
            println!("{}", String::from_utf8(gout)?.bold());
        }
        DepiCommand::New {
            name,
            deps,
            verbose,
        } => {
            let cs = Cargo::init_project(deps, verbose).await?;

            let name = PathBuf::from(name);
            fs::create_dir(&name)?;

            let mut f = fs::File::create(&name.join("Cargo.toml"))?;
            f.write_all(cs.as_bytes())?;

            let src = name.join("src");
            fs::create_dir(&src)?;
            let mut f = fs::File::create(&src.join("main.rs"))?;
            f.write_all(MAIN.as_bytes())?;

            let gout = process::Command::new("git")
                .current_dir(name)
                .arg("init")
                .output()?
                .stdout;
            println!("{}", String::from_utf8(gout)?.bold());
        }
        DepiCommand::Add { deps, verbose } => {
            let cp = Cargo::from_cur()?;
            cp.append_deps(deps, verbose).await?;
        }
        _ => {}
    }

    Ok(())
}
