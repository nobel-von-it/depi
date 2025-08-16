use std::cmp::Ord;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use miniserde::json::{self, Object, Value};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Default)]
struct OrdVersion(u32, u32, u32);

impl OrdVersion {
    fn parse<S: AsRef<str>>(s: S) -> Option<OrdVersion> {
        let mut result = OrdVersion::default();

        //    ^1.2.3-rc3
        // ...(1.2.3)...

        let mut ss = s.as_ref();

        if ss.contains("-") {
            let (left, _) = ss.split_once("-")?;
            ss = left.trim();
        }
        let start = ss.chars().nth(0)?;
        if !start.is_ascii_digit() {
            ss = ss.trim_start_matches(start);
        }

        let ss = ss.split(".").collect::<Vec<_>>();

        match ss.len() {
            1 => result.0 = ss[0].parse().ok()?,
            2 => {
                result.0 = ss[0].parse().ok()?;
                result.1 = ss[1].parse().ok()?;
            }
            3 => {
                result.0 = ss[0].parse().ok()?;
                result.1 = ss[1].parse().ok()?;
                result.2 = ss[2].parse().ok()?;
            }
            _ => return None,
        }
        Some(result)
    }
}

#[cfg(test)]
mod ord_version_test {
    use crate::OrdVersion;

    fn parse_is_some(s: &str, major: u32, minor: u32, patch: u32) {
        let ov = OrdVersion::parse(s);
        assert!(ov.is_some());
        let ov = ov.unwrap();

        assert_eq!(ov.0, major);
        assert_eq!(ov.1, minor);
        assert_eq!(ov.2, patch);
    }
    fn parse_is_none(s: &str) {
        let ov = OrdVersion::parse(s);
        assert!(ov.is_none());
    }
    #[test]
    fn parse_one_test() {
        parse_is_some("1", 1, 0, 0);
    }
    #[test]
    fn parse_two_test() {
        parse_is_some("1.2", 1, 2, 0);
    }
    #[test]
    fn parse_three_test() {
        parse_is_some("1.2.3", 1, 2, 3);
    }
    #[test]
    fn parse_with_neglect_data_one_test() {
        parse_is_some("=1", 1, 0, 0);
        parse_is_some("^1", 1, 0, 0);
        parse_is_some("~1", 1, 0, 0);

        parse_is_some("1-rc2", 1, 0, 0);
        parse_is_some("1-buld.1", 1, 0, 0);
        parse_is_some("1-blubli", 1, 0, 0);

        parse_is_some("=1-rc2", 1, 0, 0);
        parse_is_some("^1-buld.1", 1, 0, 0);
        parse_is_some("~1-blubli", 1, 0, 0);
    }
    #[test]
    fn parse_with_neglect_data_two_test() {
        parse_is_some("=1.2", 1, 2, 0);
        parse_is_some("^1.2", 1, 2, 0);
        parse_is_some("~1.2", 1, 2, 0);

        parse_is_some("1.2-rc2", 1, 2, 0);
        parse_is_some("1.2-buld.1", 1, 2, 0);
        parse_is_some("1.2-blubli", 1, 2, 0);

        parse_is_some("=1.2-rc2", 1, 2, 0);
        parse_is_some("^1.2-buld.1", 1, 2, 0);
        parse_is_some("~1.2-blubli", 1, 2, 0);
    }
    #[test]
    fn parse_with_incorrect_data_test() {
        parse_is_none("slkdjflsdkfj");
        parse_is_none("...-lbub");
        parse_is_none("1.-1.-2");
        parse_is_none("");
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
    fn from_crates_api(name: &str) -> Option<Self> {
        let mut creates_io_dep_versions = HashMap::new();

        let url = format!("https://crates.io/api/v1/crates/{name}");
        let mut res = ureq::get(&url)
            .header("User-Agent", "depi/0.1.0")
            .call()
            .ok()?;
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
        self.versions
            .iter()
            .filter_map(|(v, _)| OrdVersion::parse(v))
            .max()
            .unwrap()
            .to_string()
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
            _ => Self::Normal,
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
    fn create_with_add_fields_checked(
        dep_name: &str,
        dep_version: Option<&str>,
        dep_features: Option<&str>,
        dep_type: DepType,
    ) -> Option<Self> {
        let crates_io_dep = CratesIoDependency::from_crates_api(dep_name)?;

        let cargo_version = match dep_version {
            Some(dep_version_provided) => {
                if !crates_io_dep.versions.contains_key(dep_version_provided) {
                    eprintln!("Invalid dependency version provided");
                    return None;
                }
                dep_version_provided.to_string()
            }
            None => crates_io_dep.get_last_version(),
        };

        let mut cargo_features = if let Some(dep_features_provided) = dep_features {
            // no way
            let crates_features = crates_io_dep.versions.get(&cargo_version).unwrap();

            let requested_features = dep_features_provided
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>();

            if requested_features
                .iter()
                .all(|f| crates_features.contains(&f.to_string()))
            {
                Some(
                    requested_features
                        .into_iter()
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                )
            } else {
                eprintln!("Invalid dependency features provided");
                return None;
            }
        } else {
            None
        };

        Some(Self {
            name: dep_name.to_string(),
            version: cargo_version,
            features: cargo_features,
            type_: dep_type,
        })
    }
    fn create_with_add_fields_unchecked(
        dep_name: &str,
        dep_version: &str,
        dep_features: Option<&str>,
        dep_type: DepType,
    ) -> Self {
        let name = dep_name.to_string();
        let version = dep_version.to_string();
        let features = dep_features.map(|features| {
            features
                .split(",")
                .map(|f| f.trim().to_string())
                .collect::<Vec<_>>()
        });

        Self {
            name,
            version,
            features,
            type_: dep_type,
        }
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

                type_,
            });
        }

        let attrs_body = trim_sides(attrs, '{', '}');

        let mut version = None;
        let mut features = None;

        for pair in attrs_body.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }

            let (k, v) = pair.split_once('=').map(|(k, v)| (k.trim(), v.trim()))?;

            match k {
                "version" => {
                    version = Some(trim_str(v, "\"'").to_string());
                }
                "features" => {
                    let feature_list = trim_str(trim_sides(v, '[', ']'), "\"'")
                        .split(',')
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty())
                        .collect::<Vec<_>>();
                    features = Some(feature_list);
                }
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

fn trim_sides(s: &str, left: char, right: char) -> &str {
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
    Remove {
        #[clap(required = true)]
        dep_name: String,
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
            }
            DepiCommandVariant::Add {
                path,
                dep_name,
                // None -> latest
                dep_version,
                // None -> default_features
                dep_features,
                // dev -> Dev, other -> Normal
                dep_type,
                trust_me,
            } => {
                // &Option<String> != &str
                let path = path.as_deref().unwrap_or(".");

                let cargo_dependency = if !trust_me {
                    CargoDependency::create_with_add_fields_checked(
                        dep_name.as_str(),
                        dep_version.as_deref(),
                        dep_features.as_deref(),
                        dep_type.clone(),
                    )
                    .unwrap()
                } else {
                    let version = if let Some(v) = dep_version {
                        v
                    } else {
                        eprintln!("Cannot trust, version required");
                        return;
                    };

                    CargoDependency::create_with_add_fields_unchecked(
                        dep_name.as_str(),
                        version,
                        dep_features.as_deref(),
                        dep_type.clone(),
                    )
                };

                println!("{}", cargo_dependency.to_cargo_dependency());

                append_dep_to_cargo_file(path, cargo_dependency).unwrap();
            }
            DepiCommandVariant::Remove { dep_name } => {
                remove_dep_from_cargo_file(
                    env::current_dir().unwrap_or(".".into()),
                    dep_name.as_str(),
                )
                .unwrap();
            }
            DepiCommandVariant::Check {
                name,
                // 1
                // 1.2
                // 0.0.0
                version,
                features,
            } => {
                let crates_io_dep =
                    CratesIoDependency::from_crates_api(name).unwrap_or_else(|| {
                        eprintln!(
                            "ERROR: CratesIoDependency::from_creates_api with value {}",
                            &name
                        );
                        std::process::exit(1);
                    });

                if let Some(version) = version {
                    let version = OrdVersion::parse(version).unwrap();
                    if !crates_io_dep.versions.contains_key(&version.to_string()) {
                        eprintln!(
                            "ERROR: in crate {} provided version {} is incorrect",
                            &name, &version
                        );
                        std::process::exit(1);
                    }
                }

                if let Some(features) = features {
                    let req_feats = features
                        .split(",")
                        .map(|f| f.trim())
                        .filter(|f| !f.is_empty())
                        .collect::<Vec<_>>();
                    if req_feats.is_empty() {
                        eprintln!("ERROR: no valid features provided");
                        std::process::exit(1);
                    }

                    let comp_vers = crates_io_dep
                        .versions
                        .iter()
                        .filter(|(_, avail_feats)| {
                            req_feats
                                .iter()
                                .all(|f| avail_feats.contains(&f.to_string()))
                        })
                        .map(|(v, _)| v)
                        .collect::<Vec<_>>();
                    if comp_vers.is_empty() {
                        eprintln!(
                            "ERROR: in crate {} none of the versions support all features: {:?}",
                            &name, req_feats
                        );
                        std::process::exit(1);
                    }

                    let max_ver = comp_vers
                        .iter()
                        .filter_map(OrdVersion::parse)
                        .max()
                        .unwrap();

                    println!("INFO: provided features contains in {max_ver}");
                    println!();
                }

                println!("CHECK SUCCESSFUL!");
            }
            DepiCommandVariant::Get {
                name,
                version_count,
                full,
            } => {
                let crates_io_dep =
                    CratesIoDependency::from_crates_api(name).unwrap_or_else(|| {
                        eprintln!(
                            "ERROR: CratesIoDependency::from_creates_api with value {}",
                            &name
                        );
                        std::process::exit(1);
                    });

                let mut versions = crates_io_dep.versions.into_iter().collect::<Vec<(_, _)>>();
                versions.sort_by(|(v1, _), (v2, _)| {
                    OrdVersion::parse(v2)
                        .unwrap_or_default()
                        .cmp(&OrdVersion::parse(v1).unwrap_or_default())
                });
                if let Some(take) = version_count {
                    versions.truncate(*take);
                }

                println!("DEPENDENCY: ");
                println!(" * name: {}", &crates_io_dep.name);
                println!(" * versions: [");
                for (v, fs) in versions {
                    if *full {
                        println!(
                            "  * {}: [{}]",
                            v,
                            fs.into_iter()
                                .filter(|f| !f.starts_with("_"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    } else {
                        println!("  * {}: [...{} features]", v, fs.len());
                    }
                }
                println!(" ]")
            }
            DepiCommandVariant::List => {
                let (deps, dev_deps) =
                    get_dev_and_deps_lines_from_cargo(env::current_dir().unwrap_or(".".into()))
                        .unwrap();

                if !deps.is_empty() {
                    println!("DEPENDENCIES:");
                    for dep in deps {
                        if let Some(cargo_dep) = CargoDependency::from_cargo(dep.as_str(), "normal")
                        {
                            println!("{}", &cargo_dep)
                        }
                    }
                    println!();
                }

                if !dev_deps.is_empty() {
                    println!("DEV-DEPENDENCIES:");
                    for ddep in dev_deps {
                        if let Some(cargo_dep) = CargoDependency::from_cargo(ddep.as_str(), "dev") {
                            println!("{}", &cargo_dep)
                        }
                    }
                    println!();
                }
            }
            _ => (),
        }
    }
}

fn remove_dep_from_cargo_file<P: AsRef<Path>>(path: P, dep_name: &str) -> Option<()> {
    let cargo_path = find_cargo_file(path)?;
    let cargo_content = read_cargo_file(&cargo_path)?;

    let mut new_content = cargo_content
        .lines()
        .filter(|line| !line.contains(dep_name) || line.trim_start().starts_with("#"))
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    if cargo_content == new_content {
        eprintln!("{} not in Cargo.toml", dep_name);
        return None;
    }

    fs::write(cargo_path, new_content).ok()?;

    Some(())
}
fn append_dep_to_cargo_file<P: AsRef<Path>>(path: P, cd: CargoDependency) -> Option<()> {
    let cargo_path = find_cargo_file(path)?;
    let cargo_content = read_cargo_file(&cargo_path)?;

    let section_name = match cd.type_ {
        DepType::Normal => "[dependencies]",
        DepType::Dev => "[dev-dependencies]",
    };

    let mut inserted = false;
    let mut in_section = false;

    let mut new_content = Vec::new();

    for line in cargo_content.lines() {
        let linet = line.trim();

        if linet.starts_with("[") && linet.ends_with("]") {
            in_section = linet == section_name;
            new_content.push(line.to_string());
            continue;
        }

        if in_section {
            if let Some((name, _)) = linet.split_once("=") {
                let name = name.trim();

                if !inserted && *name > *cd.name {
                    new_content.push(cd.to_cargo_dependency());
                    inserted = true;
                }
            }
        }

        new_content.push(line.to_string());
    }

    if !inserted && in_section {
        new_content.push(cd.to_cargo_dependency());
        inserted = true;
    }

    if !inserted {
        new_content.push(String::new());
        new_content.push(section_name.to_string());
        new_content.push(cd.to_cargo_dependency());
    }

    fs::write(cargo_path, new_content.join("\n")).ok()?;

    Some(())
}
fn get_dev_and_deps_lines_from_cargo<P: AsRef<Path>>(
    path: P,
) -> Option<(Vec<String>, Vec<String>)> {
    let path = find_cargo_file(path)?;
    let content = read_cargo_file(path)?;

    get_dev_and_deps_lines_from_cargo_content(content)
}

fn get_dev_and_deps_lines_from_cargo_content<S: AsRef<str>>(
    content: S,
) -> Option<(Vec<String>, Vec<String>)> {
    let cargo_content = content.as_ref();

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
            deps.push(line.to_string());
        } else if is_dev_dep_block {
            dev_deps.push(line.to_string());
        }
    }

    Some((deps, dev_deps))
}

fn read_cargo_file<P: AsRef<Path>>(path: P) -> Option<String> {
    fs::read_to_string(path).ok()
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
