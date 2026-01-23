use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::{fs, path::PathBuf};

use anyhow::{Result, anyhow};
use futures::future;
use log::info;
use toml_edit::{DocumentMut, Formatted, Item, Table, Value};

use crate::dep::{CargoDep, Dep};
use crate::storage;
use crate::{
    dep::{self, DType, ExtDep},
    utils::{self, ColorType},
};

const SECTION_ORDER: [&str; 16] = [
    "package",
    "workspace",
    "lib",
    "bin",
    "example",
    "test",
    "bench",
    "features",
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "target",
    "patch",
    "replace",
    "profile",
    "badges",
];

pub struct Cargo(pub PathBuf);

impl Cargo {
    pub fn update_dep_type(deps: &Table) -> Result<(Vec<ExtDep>, Vec<String>)> {
        let mut fds = Vec::new();
        let mut vds = Vec::new();
        info!("init feature and and old version vector");
        for (k, v) in deps {
            let d = ExtDep::from_toml(k, v)?;
            vds.push(d.version.clone());
            fds.push(d);
        }
        info!("prepared {} deps to update", fds.len());
        Ok((fds, vds))
    }
    pub async fn update_deps(&self, ct: ColorType) -> Result<()> {
        utils::style::print_start_msg("UPDATE DEP(S)");

        info!("parsing Cargo.toml file...");
        let content = fs::read_to_string(&self.0)?;
        let mut doc = content.parse::<DocumentMut>()?;
        info!("parsed successfully");

        let mut futures = Vec::new();

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(Item::Table(deps)) = doc.get(&dtcf) {
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

            if let Some(Item::Table(deps)) = doc.get_mut(&dtcf) {
                info!("updating {} field", &dtcf);
                let mut ndeps = Table::new();
                for ud in &uds {
                    let (name, attrs) = ud.to_toml();
                    ndeps.insert(&name, attrs);
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
                utils::style::print_cargo_field(&dtype);
                if uds.len() != vds.len() {
                    return Err(anyhow!(
                        "damn error in fetching and updating deps: {}",
                        vds.len() - uds.len()
                    ));
                }
                for i in 0..uds.len() {
                    if uds[i].version > vds[i] {
                        let dep = Dep::External(uds[i].clone());
                        utils::style::print_colored_ref_dep_version_update(
                            &dep,
                            &vds[i],
                            mnl,
                            mvl,
                            2,
                            ct.get_dcolor(),
                        );
                    }
                }
            }
        }

        info!("real update {} fields", real_updated);
        info!("skip saving");
        if real_updated > 0 {
            info!("saving changes...");
            fs::write(&self.0, doc.to_string())?;
        }

        utils::style::print_end_msg();
        Ok(())
    }
    pub async fn init_project<S: AsRef<str>>(
        name: Option<S>,
        deps: Option<S>,
        ct: ColorType,
    ) -> Result<String> {
        utils::style::print_start_msg("INIT PROJECT");

        let mut newc = Table::new();

        let mut project = Table::new();
        let project_name = if let Some(name) = name {
            let name = name.as_ref().to_string();
            name
        } else {
            utils::funcs::current_absolute()?
        };

        let str_val = |s: &str| Item::Value(Value::String(Formatted::new(s.to_string())));
        project.insert("name", str_val(&project_name));
        project.insert("version", str_val("0.1.0"));
        project.insert("edition", str_val("2024"));

        newc.insert("package", Item::Table(project));

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
                    .and_modify(|tds: &mut Vec<Dep>| tds.push(Dep::External(d.clone())))
                    .or_insert(vec![Dep::External(d)]);
            }

            for (t, ds) in hmdeps {
                utils::style::print_cargo_field(&t);

                let mut tdeps = Table::new();
                for d in ds {
                    utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 0, 2, ct.get_dcolor());
                    let (name, attrs) = d.to_toml();
                    tdeps.insert(&name, attrs);
                }

                newc.insert(&t.to_cargo_field(), Item::Table(tdeps));
            }
        }

        utils::style::print_end_msg();
        Ok(newc.to_string())
    }
    pub async fn append_deps<S: AsRef<str>>(&self, deps: S, ct: ColorType) -> Result<()> {
        utils::style::print_start_msg("ADD DEP(S)");

        let content = fs::read_to_string(&self.0)?;
        let mut doc = content.parse::<DocumentMut>()?;

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
                .and_modify(|tds: &mut Vec<Dep>| tds.push(Dep::External(d.clone())))
                .or_insert(vec![Dep::External(d)]);
        }

        for (t, ds) in hmdeps {
            utils::style::print_cargo_field(&t);
            let section_key = t.to_cargo_field();
            let section = doc.entry(&section_key).or_insert(Item::Table(Table::new()));

            let deps_table = section
                .as_table_mut()
                .ok_or_else(|| anyhow!("Section [{}] is not a table", section_key))?;

            for d in ds {
                utils::style::print_colored_ref_dep_full(&d, mnl, mvl, 0, 2, ct.get_dcolor());
                let (name, attrs) = d.to_toml();
                deps_table.insert(&name, attrs);
            }
        }

        utils::style::print_end_msg();
        fs::write(&self.0, doc.to_string())?;

        Ok(())
    }
    pub async fn remove_deps<S: AsRef<str>>(&self, names: S, ct: ColorType) -> Result<()> {
        utils::style::print_start_msg("REMOVE DEP(S)");

        let content = fs::read_to_string(&self.0)?;
        let mut doc = content.parse::<DocumentMut>()?;
        let names = names.as_ref().trim().split(",").collect::<HashSet<_>>();

        let mut e_mnl = 0;
        let mut e_mvl = 0;

        let mut l_mpl = 0;

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(Item::Table(deps)) = doc.get(&dtcf) {
                for (k, v) in deps.iter() {
                    if names.contains(&k) {
                        let d = Dep::from_toml(k, v)?;
                        match &d {
                            Dep::External(e_d) => {
                                if e_d.name.len() > e_mnl {
                                    e_mnl = e_d.name.len();
                                }
                                if e_d.version.len() > e_mvl {
                                    e_mvl = e_d.version.len();
                                }
                            }
                            Dep::Local(l_d) => {
                                if l_d.name.len() > l_mpl {
                                    l_mpl = l_d.name.len();
                                }
                            }
                        }
                    }
                }
            }
        }

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(Item::Table(deps)) = doc.get_mut(&dtcf) {
                let mut removed_deps = Vec::new();
                for (k, v) in deps.iter() {
                    if names.contains(&k) {
                        let d = Dep::from_toml(k, v)?;
                        removed_deps.push(d);
                    }
                }

                if removed_deps.is_empty() {
                    continue;
                }

                utils::style::print_cargo_field_a(&dtype);
                for d in removed_deps {
                    utils::style::print_colored_ref_dep_full(
                        &d,
                        e_mnl,
                        e_mvl,
                        l_mpl,
                        2,
                        ct.get_dcolor(),
                    );
                }
                deps.retain(|k, _| !names.contains(&k));
                if deps.is_empty() {
                    doc.remove(&dtcf);
                }
            }
        }

        utils::style::print_end_msg();

        fs::write(&self.0, doc.to_string())?;
        Ok(())
    }
    async fn _get_deps_from_value(t: &Table) -> Vec<Dep> {
        t.iter()
            .map(|(dk, dv)| Dep::from_toml(dk, dv))
            .flatten()
            .collect()
    }
    pub async fn list(&self, ct: ColorType) -> Result<()> {
        utils::style::print_start_msg("LIST DEP(S)");

        let content = fs::read_to_string(&self.0)?;
        let doc = content.parse::<DocumentMut>()?;

        let mut e_mnl = 0;
        let mut e_mvl = 0;

        let mut l_mpl = 0;

        let mut hmdeps = HashMap::new();
        let mut total = 0;

        for dtype in [DType::Normal, DType::Dev, DType::Build] {
            let dtcf = dtype.to_cargo_field();
            if let Some(Item::Table(deps)) = doc.get(&dtcf) {
                for (n, ats) in deps {
                    let d = Dep::from_toml(n, ats)?;
                    match &d {
                        Dep::External(e_d) => {
                            if e_mnl < e_d.name.len() {
                                e_mnl = e_d.name.len();
                            }
                            if e_mvl < e_d.version.len() {
                                e_mvl = e_d.version.len();
                            }
                        }
                        Dep::Local(l_d) => {
                            if l_mpl < l_d.path.len() {
                                l_mpl = l_d.path.len();
                            }
                        }
                    }
                    hmdeps
                        .entry(dtype.clone())
                        .and_modify(|tds: &mut Vec<Dep>| tds.push(d.clone()))
                        .or_insert(vec![d]);
                    total += 1;
                }
            }
        }

        for (t, ds) in hmdeps {
            utils::style::print_total_dependencies(total);
            utils::style::print_cargo_field(&t);
            for d in &ds {
                if let Dep::External(e_d) = d {
                    utils::style::print_colored_val_ext_dep_full(
                        &e_d.name,
                        &e_d.version,
                        e_d.features.as_deref(),
                        e_mnl,
                        e_mvl,
                        2,
                        ct.get_dcolor(),
                    );
                }
            }
            for d in &ds {
                if let Dep::Local(l_d) = d {
                    utils::style::print_colored_val_loc_dep_full(
                        &l_d.name,
                        &l_d.path,
                        l_mpl,
                        2,
                        ct.get_dcolor(),
                    );
                }
            }
        }

        utils::style::print_end_msg();
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
