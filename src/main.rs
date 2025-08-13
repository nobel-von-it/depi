use std::{fs};
use std::collections::HashMap;
use std::cmp::{Ord};
use std::fmt;
use std::path::{Path, PathBuf};
use std::env;
use std::io::Write;

use clap::{Parser, Subcommand};
use miniserde::{json::{self, Object, Value}};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Default)]
struct OrdVersion(u32, u32, u32);

impl OrdVersion {
    fn parse<S: AsRef<str>>(s: S) -> Option<OrdVersion> {
        let ss = s.as_ref().split(".").collect::<Vec<_>>();
        if ss.len() != 3 {
            return None;
        }
        Some(Self(ss[0].parse().ok()?, ss[1].parse().ok()?, ss[2].parse().ok()?))
    }
}

impl fmt::Display for OrdVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug)]
struct CratesIoDependency {
    name: String,
    versions: HashMap<String, Vec<String>>,
}

impl CratesIoDependency {
    fn from_creates_api(name: &str) -> Option<Self> {
        let mut creates_io_dep_versions = HashMap::new();

        let url = format!("https://crates.io/api/v1/crates/{name}");
        let mut res = ureq::get(&url).header("User-Agent", "depi/0.1.0").call().ok()?;
        let body = res.body_mut().read_to_string().ok()?;

        let obj: Object = json::from_str(&body).ok()?;
        if let Some(Value::Array(versions)) = obj.get("versions") {
            for v in versions {
                if let Value::Object(v_obj) = v {
                    let num = if let Some(Value::String(num)) = v_obj.get("num") {
                        num.to_string()
                    } else {
                        continue;
                    };

                    let mut features = Vec::new();
                    if let Some(Value::Object(features_obj)) = v_obj.get("features") {
                        for (f, _) in features_obj {
                            features.push(f.to_string());
                        }
                    } else {
                        continue;
                    }

                    creates_io_dep_versions.insert(num, features);
                }
            }
        }

        Some(Self {
            name: name.to_string(),
            versions: creates_io_dep_versions,
        })
    }
    fn get_last_version(&self) -> String {
        // let mut max
        // for (v, f) in self.versions {
        // }
        self.versions.iter().filter_map(|(v, _)| OrdVersion::parse(v)).max().unwrap().0.to_string()
    }
}

#[derive(Debug, Clone)]
enum DepType {
    Dev,
    Normal,
}

impl<S: AsRef<str>> From<S> for DepType {
    fn from(s: S) -> Self {
        match s.as_ref() {
            "dev" | "d" => Self::Dev,
            _ => Self::Normal
        }
    }
}


#[derive(Debug, Clone)]
struct CargoDependency {
    name: String,
    version: String,
    features: Option<Vec<String>>,

    // TODO: replace with enum
    type_: DepType,
}

impl fmt::Display for CargoDependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        writeln!(f, "  * {}:", &self.name)?;
        writeln!(f, "    * {}", &self.version)?;
        if let Some(features) = &self.features {
            writeln!(f, "    * features:")?;
            for feat in features.iter() {
                writeln!(f, "      * {feat}")?;
            }
        }
        Ok(())
    }
}

impl CargoDependency {
    fn to_cargo_dependency(&self) -> String {
        if self.features.is_none() {
            return format!("{} = \"{}\"", self.name, self.version);
        }

        let features = self
            .features
            .as_ref()
            .unwrap()
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<String>>()
            .join(", ");
        format!(
            "{} = {{ version = \"{}\", features = [{}] }}",
            self.name, self.version, features
        )
    }
    fn from_cargo<S: AsRef<str>>(s: S, type_: S) -> Option<Self> {

        let s = s.as_ref().trim().trim_matches('"');
        let type_ = DepType::from(type_);
        let (name, attrs) = s.split_once("=").map(|(a, b)| (a.trim(), b.trim()))?;
        let name = name.to_string();

        if !attrs.contains("{") {
            let version = trim_str(attrs, "\"'").to_string();
            return Some(Self {
                name,
                version,

                features: None,

                type_
            })
        }

        let attrs_body = trim_sides(attrs, '{', '}');

        let mut version = None;
        let mut features = None;

        for pair in attrs_body.split(',') {
            let pair = pair.trim();
            if pair.is_empty() { continue; }

            let (k, v) = pair.split_once('=').map(|(k, v)| (k.trim(), v.trim()))?;
            
            match k {
                "version" => {
                    version = Some(trim_str(v, "\"'").to_string());
                },
                "features" => {
                    let feature_list = trim_str(trim_sides(v, '[', ']'), "\"'")
                        .split(',')
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty())
                        .collect::<Vec<_>>();
                    features = Some(feature_list);
                },
                _ => {}
            }
        }

        let version = version.unwrap_or("0.1.0".to_string());

        Some(Self {
            name,
            version,
            features,
            type_,
        })
    }
    fn from_provided_str(s: &str) -> Option<Self> {
        // TODO: implement many variants of creation of CargoDependency
        // now is only based
        let (name, version) = s.split_once('@').unwrap_or((s, "0.1.0"));
        Some(Self {
            name: name.to_string(),
            version: version.to_string(),
            features: None,
            type_: DepType::Normal,
        })
    }
}

fn trim_sides<'a>(s: &'a str, left: char, right: char) -> &'a str {
    s.trim_start_matches(left).trim_end_matches(right)
}
fn trim_str<'a>(s: &'a str, t: &'a str) -> &'a str {
    s.trim_matches(|c| t.contains(c))
}

#[derive(Debug, Parser)]
struct DepiCommand {
    #[clap(short, long)]
    verbose: bool,
    #[clap(short, long)]
    quiet: bool,
    #[clap(subcommand)]
    command: DepiCommandVariant,
}

#[derive(Debug, Subcommand)]
enum DepiCommandVariant {
    Init {
        #[clap(default_value = ".")]
        path: String,
        #[clap(short = 'D', long = "deps")]
        dependencies: Option<String>,
        #[clap(long = "dev-deps")]
        dev_dependencies: Option<String>,

        #[clap(short, long, default_value = "2024")]
        edition: String,
        #[clap(short, long, default_value = "false")]
        bin: bool,
        #[clap(short, long, default_value = "false")]
        lib: bool,
        #[clap(short = 'g', long = "git", default_value = "false")]
        add_git: bool,
        // By default is the current directory
        #[clap(short = 'N', long = "name")]
        project_name: Option<String>,
    },
    New {
        path: String,
        #[clap(short = 'D', long = "deps")]
        dependencies: Option<String>,
        #[clap(long = "dev-deps")]
        dev_dependencies: Option<String>,

        #[clap(short, long, default_value = "2024")]
        edition: String,
        #[clap(short, long, default_value = "false")]
        bin: bool,
        #[clap(short, long, default_value = "false")]
        lib: bool,
        #[clap(short = 'g', long = "git", default_value = "false")]
        add_git: bool,
        // By default is the provided path
        #[clap(short = 'N', long = "name")]
        project_name: Option<String>,
    },
    Add {
        #[clap(short, long)]
        path: Option<String>,

        #[clap(required = true)]
        dep_name: String,
        #[clap(short = 'v', long = "version")]
        dep_version: Option<String>,
        // Comma separated features
        #[clap(short = 'F', long = "features")]
        dep_features: Option<String>,
        // TODO: replace with enum
        #[clap(short = 't', long = "type", default_value = "normal")]
        dep_type: DepType,

        #[clap(long = "trust")]
        trust_me: bool,
    },
    Check {
        #[clap(required = true)]
        name: String,
        #[clap(short, long)]
        version: Option<String>,
        #[clap(short = 'F', long)]
        features: Option<String>,
    },
    Get {
        #[clap(required = true)]
        name: String,
        #[clap(short = 'c', long = "count")]
        version_count: Option<usize>, 
        #[clap(short, long)]
        full: bool,
    },
    List,
    Update,
    Store {
        #[clap(subcommand)]
        command: DepiStoreCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DepiStoreCommand {
    Add {
        #[clap(required = true)]
        dep_name: String,
        #[clap(short = 'v', long = "version")]
        dep_version: Option<String>,
        // Comma separated features
        #[clap(short = 'F', long = "features")]
        dep_features: Option<String>,
        // TODO: replace with enum
        #[clap(short = 't', long = "type", default_value = "normal")]
        dep_type: String,

        #[clap(short, long, default_value = "true")]
        check_api: bool,
    },
    Remove {
        #[clap(required = true)]
        dep_name: String,
    },
    List,
    Update,
}

impl DepiCommand {
    fn execute(&mut self) {
        match &self.command {
            DepiCommandVariant::Init {
                path,
                dependencies,
                dev_dependencies,
                edition,
                bin,
                lib,
                add_git,
                project_name,
            } => {
                let project_name = if let Some(name) = project_name {
                    name.to_string()
                } else if let Some(name) = get_project_name_for_init(path) {
                    name
                } else {
                    // TODO: rewrite to DepiResult or something like that
                    panic!("Could not get project name");
                };
                let deps = if let Some(dependencies) = dependencies.as_ref() {
                    dependencies
                        .split(',')
                        .map(|dep| CargoDependency::from_provided_str(dep).unwrap())
                        .collect::<Vec<CargoDependency>>()
                } else {
                    vec![]
                };
                let dev_deps = if let Some(dev_dependencies) = dev_dependencies.as_ref() {
                    dev_dependencies
                        .split(',')
                        .map(|dep| CargoDependency::from_provided_str(dep).unwrap())
                        .collect::<Vec<CargoDependency>>()
                } else {
                    vec![]
                };

                let cargo_toml_file_content = construct_cargo_file(
                    &project_name,
                    "0.1.0",
                    edition,
                    deps.as_slice(),
                    dev_deps.as_slice(),
                );

                println!("{cargo_toml_file_content}");
                // fs::write(path + "/Cargo.toml", cargo_toml_file_content).unwrap();
            },
            DepiCommandVariant::Add {
                path,
                dep_name,
                dep_version,
                dep_features,
                dep_type,
                trust_me,
            } => {
                // &Option<String> != &str
                let cargo_content = read_cargo_file(path.as_deref().unwrap_or(".")).unwrap();
                let dep_name = dep_name.to_string();

                let mut lines = cargo_content.lines().map(|l| l.to_string()).collect::<Vec<_>>();

                let mut dep_name_list = Vec::new();
                let mut in_dep = false;
                let mut start_deps = 0;
                for (i, line) in lines.iter().enumerate() {

                    if line.is_empty() || line.starts_with("#") {
                        continue;
                    }

                    if line.eq(&"[dependencies]") {
                        start_deps = i + 1;
                        in_dep = true;
                        continue;
                    } else if line.eq(&"[dev-dependencies]") {
                        in_dep = false;
                        continue;
                    }


                    if in_dep {
                        let (name, _) = line.split_once("=").map(|(v, i)| (v.trim(), i)).unwrap();
                        dep_name_list.push(name.to_string());
                    }
                }

                let mut index = 0;
                for i in 0..dep_name_list.len() {
                    if i == 0 {
                        if dep_name < dep_name_list[i] {
                            index = 0;
                            break;
                        }
                    } else if dep_name_list[i-1] < dep_name && dep_name_list[i] > dep_name {
                        index = i;
                        break;
                    } else if i == dep_name_list.len()-1 {
                        index = dep_name_list.len();
                        break;
                    }
                }

                lines.insert(start_deps + index, format!("{} = blublu", &dep_name));
                // println!("{start_deps} {index}");
                // println!("{lines:#?}");

                let mut cargo_file = open_cargo_file_rw(path.as_deref().unwrap_or(".")).unwrap();
                cargo_file.set_len(0).unwrap();
                for line in lines {
                    cargo_file.write_all(line.as_bytes()).unwrap();
                    cargo_file.write_all(b"\n").unwrap();
                }
            },
            DepiCommandVariant::Check {
                name,
                // 1
                // 1.2
                // 0.0.0
                version,
                features,
            } => {
                let crates_io_dep = CratesIoDependency::from_creates_api(name).unwrap_or_else(|| {
                    eprintln!("ERROR: CratesIoDependency::from_creates_api with value {}", &name);
                    std::process::exit(1);
                });

                if let Some(version) = version {
                    if !crates_io_dep.versions.contains_key(version) {
                        eprintln!("ERROR: in crate {} provided version {} is incorrect", &name, &version);
                        std::process::exit(1);
                    }
                }

                if let Some(features) = features {
                    let req_feats = features.split(",").map(|f| f.trim()).filter(|f| !f.is_empty()).collect::<Vec<_>>();
                    if req_feats.is_empty() {
                        eprintln!("ERROR: no valid features provided");
                        std::process::exit(1);
                    }

                    let comp_vers = crates_io_dep.versions.iter().filter(|(_, avail_feats)| {
                        req_feats.iter().all(|f| avail_feats.contains(&f.to_string()))
                    }).map(|(v, _)| v).collect::<Vec<_>>();
                    if comp_vers.is_empty() {
                        eprintln!("ERROR: in crate {} none of the versions support all features: {:?}", &name, req_feats);
                        std::process::exit(1);
                    }

                    let max_ver = comp_vers.iter().filter_map(OrdVersion::parse).max().unwrap();

                    println!("INFO: provided features contains in {max_ver}");
                    println!();
                }

                println!("CHECK SUCCESSFUL!");
            }
            DepiCommandVariant::Get { name, version_count, full } => {
                let crates_io_dep = CratesIoDependency::from_creates_api(name).unwrap_or_else(|| {
                    eprintln!("ERROR: CratesIoDependency::from_creates_api with value {}", &name);
                    std::process::exit(1);
                });

                let mut versions = crates_io_dep.versions.into_iter().collect::<Vec<(_, _)>>();
                versions.sort_by(|(v1, _), (v2, _)| OrdVersion::parse(v2).unwrap_or_default().cmp(&OrdVersion::parse(v1).unwrap_or_default()));
                if let Some(take) = version_count {
                    versions.truncate(*take);
                }

                println!("DEPENDENCY: ");
                println!(" * name: {}", &crates_io_dep.name);
                println!(" * versions: [");
                for (v, fs) in versions {
                    if *full {
                        println!("  * {}: [{}]", v, fs.into_iter().filter(|f| !f.starts_with("_")).collect::<Vec<_>>().join(", "));
                    } else {
                        println!("  * {}: [...{} features]", v, fs.len());
                    }
                }
                println!(" ]")
            }
            DepiCommandVariant::List => {
                let cargo_content = read_cargo_file(env::current_dir().unwrap()).unwrap();
                let mut is_dep_block = false;
                let mut is_dev_dep_block = false;

                let mut deps = Vec::new();
                let mut dev_deps = Vec::new();

                for line in cargo_content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with("#") {
                        continue;
                    }

                    if line.eq("[dependencies]") {
                        is_dep_block = true;
                        is_dev_dep_block = false;
                        continue;
                    } else if line.eq("[dev-dependencies]") {
                        is_dev_dep_block = true;
                        is_dep_block = false;
                        continue;
                    } else if line.starts_with("[") && line.ends_with("]") {
                        is_dev_dep_block = false;
                        is_dep_block = false;
                        continue;
                    }

                    if is_dep_block {
                        deps.push(line);
                    } else if is_dev_dep_block {
                        dev_deps.push(line);
                    }
                }

                
                println!("DEPENDENCIES:");
                for dep in deps {
                    if let Some(cargo_dep) = CargoDependency::from_cargo(dep, "normal") {
                        println!("{}", &cargo_dep)
                    }
                }
                println!();

                println!("DEV-DEPENDENCIES:");
                for ddep in dev_deps {
                    if let Some(cargo_dep) = CargoDependency::from_cargo(ddep, "dev") {
                        println!("{}", &cargo_dep)
                    }
                }
                println!();
            }
            _ => (),
        }
    }
}

fn open_cargo_file_rw<P: AsRef<Path>>(path: P) -> Option<fs::File> {
    let cargo_path = find_cargo_file(path)?;
    fs::File::options().read(true).write(true).create(true).open(cargo_path).ok()
}

fn read_cargo_file<P: AsRef<Path>>(path: P) -> Option<String> {
    let cargo_path = find_cargo_file(path)?;
    fs::read_to_string(cargo_path).ok()
}

fn find_cargo_file<P: AsRef<Path>>(path: P) -> Option<PathBuf> {
    let path = path.as_ref();
    let files = fs::read_dir(path).ok()?;
    for file in files.into_iter().flatten() {
        if file.path().display().to_string().contains("Cargo.toml") {
            return Some(file.path());
        }
    }
    if let Some(parent) = path.parent() {
        return find_cargo_file(parent);
    }
    None
}

fn construct_cargo_file(
    name: &str,
    version: &str,
    edition: &str,
    deps: &[CargoDependency],
    dev_deps: &[CargoDependency],
) -> String {
    let mut sb = String::new();
    sb.push_str("[package]\n");
    sb.push_str(&format!("name = \"{name}\"\n"));
    sb.push_str(&format!("version = \"{version}\"\n"));
    sb.push_str(&format!("edition = \"{edition}\"\n"));
    sb.push('\n');
    sb.push_str("[dependencies]\n");
    for dep in deps {
        sb.push_str(&dep.to_cargo_dependency());
        sb.push('\n');
    }
    sb.push('\n');
    sb.push_str("[dev-dependencies]\n");
    for dep in dev_deps {
        sb.push_str(&dep.to_cargo_dependency());
    }
    sb
}

fn get_project_name_for_init(path: &str) -> Option<String> {
    Some(
        fs::canonicalize(path)
            .ok()?
            .file_name()?
            .to_str()?
            .to_string(),
    )
}

fn main() {
    let mut command = DepiCommand::parse();
    // let cur_dir = std::env::current_dir().unwrap();
    // println!("{:?}", find_cargo_file(cur_dir));
    command.execute();
}
