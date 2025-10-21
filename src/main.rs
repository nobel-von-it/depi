use anyhow::{Result, anyhow};
use clap::Parser;
use colored::*;
use futures::future;
use log::info;
use serde_json::{Value as JValue, from_str};
use toml::{Table, Value as TValue, value::Array};

use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone)]
enum ColorType {
    One(DColor),
    Random,
}

impl ColorType {
    fn get_dcolor(&self) -> DColor {
        match self {
            Self::One(dc) => *dc,
            Self::Random => DColor::get_random(),
        }
    }
}

impl Default for ColorType {
    fn default() -> Self {
        Self::One(DColor::default())
    }
}

impl<S: AsRef<str>> From<S> for ColorType {
    fn from(s: S) -> Self {
        match s.as_ref().to_lowercase().as_str() {
            "rand" | "random" => Self::Random,
            _ => Self::One(DColor::from(s)),
        }
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
        #[clap(short = 'D', long)]
        deps: Option<String>,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    /// Create a new project in provided directory with provided dependencies
    New {
        /// Project and directory name
        #[clap(required = true)]
        name: String,

        /// Dependencies string in format: <name>@<version>:<features>!<target>
        /// Multiple dependencies separated by '/'
        /// Example: serde@1.0:derive,json/anyhow@1.0!dev
        #[clap(short = 'D', long)]
        deps: Option<String>,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    /// Add new dependencies to existing project
    Add {
        /// Dependencies string in format: <name>@<version>:<features>!<target>
        /// Multiple dependencies separated by '/'
        /// Example: serde@1.0:derive,json/anyhow@1.0!dev
        #[clap(required = true)]
        deps: String,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    /// Remove dependencies from existing project
    Remove {
        /// Dependency names to remove, separated by commas
        /// Example: serde,anyhow,tokio
        #[clap(required = true)]
        names: String,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    /// List all current dependencies
    List {
        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    /// Update dependencies to latest versions
    Update {
        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
}

struct Cargo(PathBuf);

impl Cargo {
    fn update_dep_type(deps: &Table) -> Result<(Vec<Dep>, Vec<String>)> {
        let mut fds = Vec::new();
        let mut vds = Vec::new();
        info!("init feature and and old version vector");
        for (k, v) in deps {
            let d = Dep::from_toml(k, v.clone())?;
            vds.push(d.version.clone());
            fds.push(d);
        }
        info!("prepared {} deps to update", fds.len());
        Ok((fds, vds))
    }
    async fn update_deps(&self, ct: ColorType) -> Result<()> {
        println!("{}", "DEPS UP".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        info!("parsing Cargo.toml file...");
        let content = fs::read_to_string(&self.0)?;
        let mut content = content.parse::<Table>()?;
        info!("parsed successfully");

        let mut futures = Vec::new();

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(TValue::Table(deps)) = content.get(dtcf.as_str()) {
                info!("fetching {} field", &dtcf);
                futures.push(async move {
                    let (fds, vds) = Self::update_dep_type(&deps)?;
                    let ufds = fds
                        .into_iter()
                        .map(|d| d.update_version())
                        .collect::<Vec<_>>();
                    let uds = (future::join_all(ufds).await)
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>();

                    Ok::<_, anyhow::Error>((dtype, uds, vds))
                });
            }
        }
        info!("started {} update futures", futures.len());
        let frs = (future::join_all(futures).await)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        info!("awaited {} futures", frs.len());

        let mut mnl = 0;
        let mut mvl = 0;

        info!("perform max name and version");
        for fr in &frs {
            let (_, uds, _) = fr;

            for ud in uds {
                if mnl < ud.name.len() {
                    mnl = ud.name.len();
                }
                if mvl < ud.version.len() {
                    mvl = ud.version.len();
                }
            }
        }
        info!("got {}/{}", mnl, mvl);

        let mut real_updated = 0;
        for fr in frs {
            let (dtype, uds, vds) = fr;

            let dtcf = dtype.to_cargo_field();

            if let Some(TValue::Table(deps)) = content.get_mut(&dtcf) {
                info!("updating {} field", &dtcf);
                let mut ndeps = Table::new();
                for ud in &uds {
                    let (name, attrs) = ud.to_toml();
                    ndeps.insert(name, attrs);
                }
                *deps = ndeps;
            }

            let mut changed = 0;
            for i in 0..uds.len() {
                if uds[i].version > vds[i] {
                    changed += 1;
                }
            }
            info!("updated {} deps in {}", changed, &dtcf);

            if changed > 0 {
                real_updated += 1;
                println!("{}", dtype.to_cargo_field().green());
                if uds.len() != vds.len() {
                    return Err(anyhow!(
                        "damn error in fetching and updating deps: {}",
                        vds.len() - uds.len()
                    ));
                }
                for i in 0..uds.len() {
                    if uds[i].version > vds[i] {
                        uds[i].print_updated(&vds[i], mnl, mvl, 2, ct.get_dcolor());
                        println!(
                            "  {:<mnl$} {:<mvl$} {} {}",
                            uds[i].name.bold(),
                            vds[i].yellow(),
                            "->".dimmed(),
                            uds[i].version.green()
                        );
                    }
                }
            }
        }

        info!("real update {} fields", real_updated);
        info!("skip saving");
        if real_updated > 0 {
            info!("saving changes...");
            fs::write(&self.0, toml::to_string(&content)?)?;
        }

        println!("{}", "=".repeat(40).cyan());
        Ok(())
    }
    async fn init_project<S: AsRef<str>>(
        name: Option<S>,
        deps: Option<S>,
        ct: ColorType,
    ) -> Result<String> {
        println!("{}", "INIT".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let mut newc = Table::new();

        let mut project = Table::new();
        let project_name = if let Some(name) = name {
            let name = name.as_ref().to_string();
            name
        } else {
            fs::canonicalize(env::current_dir().unwrap_or(".".into()))?
                .file_name()
                .ok_or(anyhow!("failed file_name()"))?
                .to_str()
                .ok_or(anyhow!("failed to_str()"))?
                .to_string()
        };

        project.insert("name".to_string(), TValue::String(project_name));
        project.insert("version".to_string(), TValue::String("0.1.0".to_string()));
        project.insert("edition".to_string(), TValue::String("2024".to_string()));

        newc.insert("package".to_string(), TValue::Table(project));

        if let Some(deps) = deps {
            let pdeps = parse_deps(deps.as_ref())?;
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

            let mut hmdeps = HashMap::new();

            let mut mnl = 0;
            let mut mvl = 0;

            for i in 0..fdl {
                let d = get_normal_dep(&pdeps[i], &fdeps[i])?;
                if mnl < d.name.len() {
                    mnl = d.name.len();
                }
                if mvl < d.version.len() {
                    mvl = d.version.len();
                }
                hmdeps
                    .entry(DType::from(&pdeps[i].target))
                    .and_modify(|tds: &mut Vec<Dep>| tds.push(d.clone()))
                    .or_insert(vec![d]);
            }

            for (t, ds) in hmdeps {
                println!("{}", t.to_cargo_field().bold().green());

                let mut tdeps = Table::new();
                for d in ds {
                    d.print_pretty(mnl, mvl, 2, ct.get_dcolor());
                    let (name, attrs) = d.to_toml();
                    tdeps.insert(name, attrs);
                }

                newc.insert(t.to_cargo_field(), TValue::Table(tdeps));
            }
        }

        println!("{}", "=".repeat(40).cyan());

        Ok(toml::to_string(&newc)?)
    }
    async fn append_deps<S: AsRef<str>>(&self, deps: S, ct: ColorType) -> Result<()> {
        println!("{}", "DEP(S) ADD".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let content = fs::read_to_string(&self.0)?;
        let mut content = content.parse::<Table>()?;

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
            return Err(anyhow!(
                "error fetching some dependencies: {}",
                fdl - fdeps.len()
            ));
        }

        let mut mnl = 0;
        let mut mvl = 0;

        let mut hmdeps = HashMap::new();
        for i in 0..fdl {
            let d = get_normal_dep(&pdeps[i], &fdeps[i])?;
            if mnl < d.name.len() {
                mnl = d.name.len();
            }
            if mvl < d.version.len() {
                mvl = d.version.len();
            }

            hmdeps
                .entry(DType::from(&pdeps[i].target))
                .and_modify(|tds: &mut Vec<Dep>| tds.push(d.clone()))
                .or_insert(vec![d]);
        }

        for (t, ds) in hmdeps {
            println!("{}", t.to_cargo_field().bold().green());

            match content.get_mut(&t.to_cargo_field()) {
                Some(TValue::Table(deps)) => {
                    for d in ds {
                        d.print_pretty(mnl, mvl, 2, ct.get_dcolor());
                        let (name, attrs) = d.to_toml();
                        deps.insert(name, attrs);
                    }
                }
                Some(_) => println!("tipatipitiritoutou"),
                None => {
                    let mut deps = Table::new();
                    for d in ds {
                        d.print_pretty(mnl, mvl, 2, ct.get_dcolor());
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
    async fn remove_deps<S: AsRef<str>>(&self, names: S, ct: ColorType) -> Result<()> {
        println!("{}", "DEP(S) REM".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let mut content = fs::read_to_string(&self.0)?.parse::<Table>()?;
        let names = names.as_ref().trim().split(",").collect::<HashSet<_>>();

        let mut mnl = 0;
        let mut mvl = 0;

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(TValue::Table(deps)) = content.get(&dtcf) {
                for (k, v) in deps.iter() {
                    if names.contains(&k.as_str()) {
                        let d = Dep::from_toml(k, v.clone())?;
                        if d.name.len() > mnl {
                            mnl = d.name.len();
                        }
                        if d.version.len() > mvl {
                            mvl = d.version.len();
                        }
                    }
                }
            }
        }

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(TValue::Table(deps)) = content.get_mut(&dtcf) {
                let mut removed_deps = Vec::new();
                for (k, v) in deps.iter() {
                    if names.contains(&k.as_str()) {
                        let d = Dep::from_toml(k, v.clone())?;
                        removed_deps.push(d);
                    }
                }

                if removed_deps.is_empty() {
                    continue;
                }

                println!("{}", dtcf.bold().red());
                for d in removed_deps {
                    d.print_pretty(mnl, mvl, 2, ct.get_dcolor());
                }
                deps.retain(|k, _| !names.contains(&k));
                if deps.is_empty() {
                    content.remove(&dtcf);
                }
            }
        }

        println!("{}", "=".repeat(40).cyan());

        fs::write(&self.0, toml::to_string(&content)?)?;
        Ok(())
    }
    async fn _get_deps_from_value(t: &Table) -> Vec<Dep> {
        t.iter()
            .map(|(dk, dv)| Dep::from_toml(dk, dv.clone()))
            .flatten()
            .collect()
    }
    async fn list(&self, ct: ColorType) -> Result<()> {
        println!("{}", "DEPS LIST".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let content = fs::read_to_string(&self.0)?.parse::<Table>()?;

        let mut mnl = 0;
        let mut mvl = 0;

        let mut hmdeps = HashMap::new();

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(TValue::Table(deps)) = content.get(&dtcf) {
                for (n, ats) in deps {
                    let d = Dep::from_toml(n, ats.clone())?;
                    if mnl < d.name.len() {
                        mnl = d.name.len();
                    }
                    if mvl < d.version.len() {
                        mvl = d.version.len();
                    }

                    hmdeps
                        .entry(dtype.clone())
                        .and_modify(|tds: &mut Vec<Dep>| tds.push(d.clone()))
                        .or_insert(vec![d]);
                }
            }
        }

        for (t, ds) in hmdeps {
            println!("{}", t.to_cargo_field().bold().green());
            for d in ds {
                d.print_pretty(mnl, mvl, 2, ct.get_dcolor());
            }
        }

        println!("{}", "=".repeat(40).cyan());
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

        if let Some((_, right)) = s.split_once("-") {
            return Err(anyhow!("version with suffix {right} is not supported"));
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
            .filter_map(|(v, _)| OrdVersion::parse(v).ok())
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

enum DepsSnippet {
    Axum,
}

impl<S: AsRef<str>> From<S> for DepsSnippet {
    fn from(value: S) -> Self {
        match value
            .as_ref()
            .strip_prefix(SNIPPET_PREFIX)
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "axum" => Self::Axum,
            _ => panic!("incorrect snippet"),
        }
    }
}

impl DepsSnippet {
    fn get_deps(&self) -> Vec<Result<PDep>> {
        match self {
            Self::Axum => {
                vec![parse_dep("tokio:full"), parse_dep("axum")]
            }
        }
    }
}

const SNIPPET_PREFIX: &str = "s+";

fn parse_deps<S: AsRef<str>>(s: S) -> Result<Vec<PDep>> {
    let mut res = Vec::new();
    for d in s.as_ref().trim().split("/") {
        if d.starts_with(SNIPPET_PREFIX) {
            for pd in DepsSnippet::from(d).get_deps() {
                res.push(pd);
            }
        } else {
            res.push(parse_dep(d));
        }
    }
    res.into_iter().collect()
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

#[derive(Default, Clone, Copy, Debug)]
enum DColor {
    #[default]
    WithoutColor,
    GOIDA,
    Osetia,
    Poland,
}

impl DColor {
    fn get_random() -> Self {
        match rand::random_range(0..3) {
            0 => Self::GOIDA,
            1 => Self::Osetia,
            2 => Self::Poland,
            _ => unreachable!(),
        }
    }
}

impl<S: AsRef<str>> From<S> for DColor {
    fn from(s: S) -> Self {
        match s.as_ref().to_lowercase().as_str() {
            "rus" | "goool" | "goida" => Self::GOIDA,
            "osetia" | "auto" => Self::Osetia,
            "poland" => Self::Poland,
            _ => Self::WithoutColor,
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
    fn print_updated<S: AsRef<str>>(
        &self,
        oldv: S,
        mnl: usize,
        mvl: usize,
        tabbing: usize,
        dct: DColor,
    ) {
        match dct {
            DColor::WithoutColor => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    self.name.bold(),
                    oldv.as_ref(),
                    "->".dimmed(),
                    self.version.bold()
                )
            }
            DColor::GOIDA => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    self.name.bold(),
                    oldv.as_ref().blue(),
                    "->".dimmed(),
                    self.version.bold().red()
                )
            }
            DColor::Osetia => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    self.name.bold(),
                    oldv.as_ref().yellow(),
                    "->".dimmed(),
                    self.version.bold().red()
                )
            }
            DColor::Poland => {
                let oldv = oldv.as_ref();
                let oll = oldv.len() / 2;
                let oldvl = &oldv[0..oll];
                let oldvr = &oldv[oll..oldv.len()];

                let nmvl = mvl - oldvl.len();
                println!(
                    "{}{:<mnl$} {}{:<nmvl$} {} {}",
                    " ".repeat(tabbing),
                    self.name.bold(),
                    oldvl,
                    oldvr.red(),
                    "->".dimmed(),
                    self.version.bold().red()
                )
            }
        }
    }
    fn print_pretty(&self, mnl: usize, mvl: usize, tabbing: usize, dct: DColor) {
        match dct {
            DColor::WithoutColor => {
                if let Some(fs) = &self.features {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$} {} {}",
                        " ".repeat(tabbing),
                        &self.name.bold(),
                        "@".dimmed(),
                        &self.version,
                        ":".dimmed(),
                        fs.join(", "),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &self.name,
                        "@".dimmed(),
                        &self.version
                    );
                }
            }
            DColor::GOIDA => {
                if let Some(fs) = &self.features {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$} {} {}",
                        " ".repeat(tabbing),
                        &self.name.bold(),
                        "@".dimmed(),
                        &self.version.blue(),
                        ":".dimmed(),
                        fs.join(", ").red(),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &self.name.bold(),
                        "@".dimmed(),
                        &self.version.blue()
                    );
                }
            }
            DColor::Osetia => {
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
            DColor::Poland => {
                if let Some(fs) = &self.features {
                    let dvr = &self.version;
                    let dvh = dvr.len() / 2;
                    let dvrl = &dvr[0..dvh];
                    let dvrr = &dvr[dvh..dvr.len()];

                    let nmvl = mvl - dvrl.len();
                    println!(
                        "{}{:<mnl$} {} {}{:<nmvl$} {} {}",
                        " ".repeat(tabbing),
                        &self.name.bold(),
                        "@".dimmed(),
                        dvrl,
                        dvrr.red(),
                        ":".dimmed(),
                        fs.join(", ").red(),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &self.name.bold(),
                        "@".dimmed(),
                        &self.version.red()
                    );
                }
            }
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
    env_logger::init();

    let com = DepiCommand::parse();
    match com {
        DepiCommand::Init { deps, color } => {
            // let cp = Cargo::from_cur()?;
            let cs = Cargo::init_project(None, deps.as_deref(), color).await?;

            let mut f = fs::File::create("Cargo.toml")?;
            f.write_all(cs.as_bytes())?;

            fs::create_dir("src")?;
            let mut f = fs::File::create("src/main.rs")?;
            f.write_all(MAIN.as_bytes())?;

            let gout = process::Command::new("git").arg("init").output()?.stdout;
            println!("{}", String::from_utf8(gout)?.bold());
        }
        DepiCommand::New { name, deps, color } => {
            let cs = Cargo::init_project(Some(name.as_ref()), deps.as_deref(), color).await?;

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
        DepiCommand::Add { deps, color } => {
            let cp = Cargo::from_cur()?;
            cp.append_deps(deps, color).await?;
        }
        DepiCommand::Remove { names, color } => {
            let cp = Cargo::from_cur()?;
            cp.remove_deps(names, color).await?;
        }
        DepiCommand::Update { color } => {
            let cp = Cargo::from_cur()?;
            cp.update_deps(color).await?;
        }
        DepiCommand::List { color } => {
            let cp = Cargo::from_cur()?;
            cp.list(color).await?;
        }
    }

    Ok(())
}
