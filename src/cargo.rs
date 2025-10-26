use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::{fs, path::PathBuf};

use anyhow::{Result, anyhow};
use colored::Colorize;
use futures::future;
use log::info;
use toml::Table;
use toml::Value as TValue;

use crate::storage;
use crate::{
    dep::{self, DType, Dep},
    utils::{self, ColorType},
};

pub struct Cargo(pub PathBuf);

impl Cargo {
    pub fn update_dep_type(deps: &Table) -> Result<(Vec<Dep>, Vec<String>)> {
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
    pub async fn update_deps(&self, ct: ColorType) -> Result<()> {
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
                        utils::style::print_colored_ref_dep_version_update(
                            &uds[i],
                            &vds[i],
                            mnl,
                            mvl,
                            2,
                            ct.get_dcolor(),
                        );
                        // println!(
                        //     "  {:<mnl$} {:<mvl$} {} {}",
                        //     uds[i].name.bold(),
                        //     vds[i].yellow(),
                        //     "->".dimmed(),
                        //     uds[i].version.green()
                        // );
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
    pub async fn init_project<S: AsRef<str>>(
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
            utils::funcs::current_absolute()?
        };

        project.insert("name".to_string(), TValue::String(project_name));
        project.insert("version".to_string(), TValue::String("0.1.0".to_string()));
        project.insert("edition".to_string(), TValue::String("2024".to_string()));

        newc.insert("package".to_string(), TValue::Table(project));

        if let Some(deps) = deps {
            let a_s = storage::AliasStorage::load()?;
            let pdeps = dep::parse::parse_deps(deps.as_ref(), a_s.list())?;
            let mut fdeps = Vec::new();
            for pd in &pdeps {
                fdeps.push(dep::api::fetch_crates_dep(&pd.name));
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
                let d = dep::normalize(&pdeps[i], &fdeps[i])?;
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
                    utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 2, ct.get_dcolor());
                    let (name, attrs) = d.to_toml();
                    tdeps.insert(name, attrs);
                }

                newc.insert(t.to_cargo_field(), TValue::Table(tdeps));
            }
        }

        println!("{}", "=".repeat(40).cyan());

        Ok(toml::to_string(&newc)?)
    }
    pub async fn append_deps<S: AsRef<str>>(&self, deps: S, ct: ColorType) -> Result<()> {
        println!("{}", "DEP(S) ADD".bold().on_cyan());
        println!("{}", "=".repeat(40).cyan());

        let content = fs::read_to_string(&self.0)?;
        let mut content = content.parse::<Table>()?;

        let a_s = storage::AliasStorage::load()?;
        let pdeps = dep::parse::parse_deps(deps.as_ref(), a_s.list())?;
        let mut fdeps = Vec::new();
        for pd in &pdeps {
            fdeps.push(dep::api::fetch_crates_dep(&pd.name));
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
            let d = dep::normalize(&pdeps[i], &fdeps[i])?;
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
                        utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 2, ct.get_dcolor());
                        let (name, attrs) = d.to_toml();
                        deps.insert(name, attrs);
                    }
                }
                Some(_) => println!("tipatipitiritoutou"),
                None => {
                    let mut deps = Table::new();
                    for d in ds {
                        utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 2, ct.get_dcolor());
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
    pub async fn remove_deps<S: AsRef<str>>(&self, names: S, ct: ColorType) -> Result<()> {
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
                    utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 2, ct.get_dcolor());
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
    pub async fn list(&self, ct: ColorType) -> Result<()> {
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
                utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 2, ct.get_dcolor());
            }
        }

        println!("{}", "=".repeat(40).cyan());
        Ok(())
    }
    pub fn from_cur() -> Result<Self> {
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
