use anyhow::{Error, Result, anyhow};
use clap::{Parser, Subcommand};
use colored::*;
use futures::{Future, future};
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
    fn list_deps(deps: &HashMap<DType, Vec<Dep>>) {
        println!("{}", "DEPS LS".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        let mut mnl = 0;
        let mut mvl = 0;

        for (_, ds) in deps {
            mnl = mnl.max(ds.iter().map(|d| d.name.len()).max().unwrap());
            mvl = mnl.max(ds.iter().map(|d| d.version.len()).max().unwrap());
        }

        for (t, ds) in deps {
            println!("{}", t.to_string().to_uppercase().bold());
            for d in ds {
                if let Some(fs) = &d.features {
                    let fs = fs.join(", ");
                    println!(
                        "  - {:<mnl$} @ {:<mvl$} : {}",
                        d.name.bold().green(),
                        d.version.yellow(),
                        fs.red()
                    )
                } else {
                    println!(
                        "  - {:<mnl$} @ {:<mvl$}",
                        d.name.bold().green(),
                        d.version.yellow()
                    )
                }
            }
            println!("{}", "=".repeat(40).cyan());
            println!()
        }
    }
    fn remove_deps(removed: &[&str], remained: &[&str], all_removed: bool) {
        println!("{}", "DEPS TO REMOVE".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        // let mut mnl = (removed.iter().map(|d| d.len()).max().unwrap())
        //     .max(remained.iter().map(|d| d.len()).max().unwrap());

        for name in removed {
            println!("{}", name.red());
        }

        println!("{}", "DEPS TO REMAIN".bold().cyan());
        println!("{}", "=".repeat(40).cyan());

        for name in remained {
            println!("{}", name.bold().green())
        }

        println!("{}", "=".repeat(40).cyan());

        if all_removed {
            println!("{}", "all deps removed successfully".green());
        } else {
            println!("{}", "something went wrong (ignore or error)".red());
        }
    }
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

#[derive(Hash, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DType {
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
    fn to_cargo_field(&self) -> String {
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

#[derive(Debug, Parser)]
#[clap(about = "DEPendency Initialization manager", version)]
enum DepiCommand {
    /// Initialize a new project in current directory with provided dependencies
    Init {
        /// Dependencies string in format: <name>@<version>:<features>!<target>
        /// Multiple dependencies separated by '/'
        /// Example: serde@1.0:derive,json/anyhow@1.0!dev
        #[clap(short = 'D', long, required = true)]
        deps: String,

        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Create a new project in provided directory with provided dependencies
    New {
        /// Project and directory name
        #[clap(required = true)]
        name: String,

        /// Dependencies string in format: <name>@<version>:<features>!<target>
        /// Multiple dependencies separated by '/'
        /// Example: serde@1.0:derive,json/anyhow@1.0!dev
        #[clap(short = 'D', long, required = true)]
        deps: String,

        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Add new dependencies to existing project
    Add {
        /// Dependencies string in format: <name>@<version>:<features>!<target>
        /// Multiple dependencies separated by '/'
        /// Example: serde@1.0:derive,json/anyhow@1.0!dev
        #[clap(required = true)]
        deps: String,

        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Remove dependencies from existing project
    Remove {
        /// Dependency names to remove, separated by commas
        /// Example: serde,anyhow,tokio
        #[clap(required = true)]
        names: String,

        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// List all current dependencies
    List {
        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
    /// Update dependencies to latest versions
    Update {
        /// Enable verbose output
        #[clap(short, long)]
        verbose: bool,
    },
}

fn _start_update_futures_from_toml_table(
    deps: &Table,
) -> Result<(Vec<String>, Vec<impl Future<Output = Result<Dep, Error>>>)> {
    let mut oldvs = Vec::new();
    let mut ress = Vec::new();
    for (dk, dv) in deps {
        let d = Dep::from_toml(dk, dv.clone())?;
        oldvs.push(d.version.clone());
        ress.push(d.update_version());
    }

    Ok((oldvs, ress))
}

async fn _update_deps_by_table_and_type(
    deps: &Table,
    dtype: DType,
    verbose: bool,
) -> Result<(DType, Option<Table>)> {
    if deps.is_empty() {
        return Ok((dtype.clone(), None));
    }

    let mut ovs = Vec::new();
    let mut fdeps = Vec::new();
    for (k, v) in deps {
        let d = Dep::from_toml(k, v.clone())?;
        ovs.push(d.version.clone());
        fdeps.push(d.update_version());
    }

    let fdl = fdeps.len();
    let udeps = (future::join_all(fdeps).await)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if fdl != udeps.len() {
        return Err(anyhow!(
            "ERROR: with updating some deps {}",
            dtype.to_string()
        ));
    }

    let mnl = udeps.iter().map(|d| d.name.len()).max().unwrap();
    let mvl = ovs.iter().map(|v| v.len()).max().unwrap();

    let mut rest = Table::new();
    let mut latest = 0;
    for i in 0..fdl {
        if ovs[i] < udeps[i].version {
            println!(
                "{} {} -> {}",
                &udeps[i].name.bold().green(),
                &ovs[i].dimmed(),
                &udeps[i].version.bold().yellow()
            );
        } else {
            latest += 1;
        }

        let (name, attrs) = udeps[i].to_toml();
        rest.insert(name, attrs);
    }

    if fdl == latest {
        println!(
            "all deps in {} are {}",
            dtype.to_cargo_field().bold(),
            "latest".bold().green()
        );
    } else {
        println!(
            "{} dep(s) in {} are latest",
            latest.to_string().bold().green(),
            dtype.to_cargo_field().bold()
        );
    }

    println!("{}", "=".repeat(40).cyan());

    Ok((dtype.clone(), Some(rest)))
}

struct Cargo(PathBuf);

impl Cargo {
    async fn update_dep_type(
        content: &Table,
        verbose: bool,
        dtype: DType,
    ) -> Result<(DType, Option<Table>)> {
        if let Some(TValue::Table(deps)) = content.get(&dtype.to_cargo_field()) {
            return Ok(_update_deps_by_table_and_type(&deps, dtype, verbose).await?);
        }
        Ok((dtype.clone(), None))
    }
    async fn update_deps(&self, verbose: bool) -> Result<()> {
        println!("{}", "DEPS UP".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let content = fs::read_to_string(&self.0)?;
        if verbose {
            println!("INFO: start updating {} file", self.0.display())
        }
        let mut content = content.parse::<Table>()?;

        let mut fupddt = Vec::with_capacity(3);

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            fupddt.push(Self::update_dep_type(&content, verbose, dtype))
        }

        let tupds = (future::join_all(fupddt).await)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            if let Some(TValue::Table(ndeps)) = content.get_mut(&dtype.to_cargo_field()) {
                let (_, table) = tupds.iter().find(|(dt, td)| *dt == dtype).unwrap();
                *ndeps = table.clone().unwrap();
            }
        }

        fs::write(&self.0, toml::to_string(&content)?)?;

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
        println!("{}", "DEP(S) ADD".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let content = fs::read_to_string(&self.0)?;
        let mut content = content.parse::<Table>()?;

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
            return Err(anyhow!(
                "error fetching some dependencies: {}",
                fdl - fdeps.len()
            ));
        }

        let mut hmdeps = HashMap::new();
        for i in 0..fdl {
            let d = get_normal_dep(&pdeps[i], &fdeps[i])?;
            hmdeps
                .entry(DType::from(&pdeps[i].target))
                .and_modify(|tds: &mut Vec<Dep>| tds.push(d.clone()))
                .or_insert(vec![d]);
        }

        for (t, ds) in hmdeps {
            println!("{}", t.to_cargo_field().bold().green());

            let mnl = ds.iter().map(|d| d.name.len()).max().unwrap();
            let mvl = ds.iter().map(|d| d.version.len()).max().unwrap();

            match content.get_mut(&t.to_cargo_field()) {
                Some(TValue::Table(deps)) => {
                    for d in ds {
                        d.print_pretty(mnl, mvl, 2);
                        let (name, attrs) = d.to_toml();
                        deps.insert(name, attrs);
                    }
                }
                Some(_) => println!("tipatipitiritoutou"),
                None => {
                    let mut deps = Table::new();
                    for d in ds {
                        d.print_pretty(mnl, mvl, 2);
                        let (name, attrs) = d.to_toml();
                        deps.insert(name, attrs);
                    }
                    content.insert(t.to_cargo_field(), TValue::Table(deps));
                }
            }
        }

        fs::write(&self.0, toml::to_string(&content)?)?;

        println!("{}", "=".repeat(40).cyan());

        Ok(())
    }
    async fn remove_deps<S: AsRef<str>>(&self, names: S, verbose: bool) -> Result<()> {
        let mut content = fs::read_to_string(&self.0)?.parse::<Table>()?;

        let names = names.as_ref().trim().split(",").collect::<Vec<_>>();
        if let Some(TValue::Table(deps)) = content.get_mut("dependencies") {
            let needs_remove = names.len();

            let before_remove = deps.len();
            deps.retain(|k, v| !names.contains(&k));
            let after_remove = deps.len();

            let cur_deps = deps.iter().map(|(dk, _)| dk.as_str()).collect::<Vec<_>>();

            Printer::remove_deps(
                &names,
                &cur_deps,
                needs_remove == (before_remove - after_remove),
            );

            fs::write(&self.0, toml::to_string(&content)?)?;
        }

        Ok(())
    }
    async fn _get_deps_from_value(t: &Table) -> Vec<Dep> {
        t.iter()
            .map(|(dk, dv)| Dep::from_toml(dk, dv.clone()))
            .flatten()
            .collect()
    }
    async fn list(&self, verbose: bool) -> Result<()> {
        let content = fs::read_to_string(&self.0)?.parse::<Table>()?;
        let mut deps_by_type: HashMap<DType, Vec<Dep>> = HashMap::new();
        for (k, v) in content {
            match k.as_str() {
                "dependencies" => {
                    if let TValue::Table(v) = v {
                        deps_by_type.insert(DType::Normal, Self::_get_deps_from_value(&v).await);
                    }
                }
                "dev-dependencies" => {
                    if let TValue::Table(v) = v {
                        deps_by_type.insert(DType::Dev, Self::_get_deps_from_value(&v).await);
                    }
                }
                "build-dependencies" => {
                    if let TValue::Table(v) = v {
                        deps_by_type.insert(DType::Build, Self::_get_deps_from_value(&v).await);
                    }
                }
                _ => {}
            }
        }
        Printer::list_deps(&deps_by_type);
        Ok(())
    }
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
    fn print_pretty(&self, mnl: usize, mvl: usize, tabbing: usize) {
        if let Some(fs) = &self.features {
            println!(
                "{}{:<mnl$} {} {:<mvl$} {} {}",
                " ".repeat(tabbing),
                &self.name.bold(),
                "@".dimmed(),
                &self.version.yellow(),
                ":".dimmed(),
                fs.join(", ").red(),
            );
        } else {
            println!(
                "{}{:<mnl$} {} {:<mvl$}",
                " ".repeat(tabbing),
                &self.name.bold(),
                "@".dimmed(),
                &self.version.yellow()
            );
        }
    }
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
        DepiCommand::Remove { names, verbose } => {
            let cp = Cargo::from_cur()?;
            cp.remove_deps(names, verbose).await?;
        }
        DepiCommand::Update { verbose } => {
            let cp = Cargo::from_cur()?;
            cp.update_deps(verbose).await?;
        }
        DepiCommand::List { verbose } => {
            let cp = Cargo::from_cur()?;
            cp.list(verbose).await?;
        }
    }

    Ok(())
}
