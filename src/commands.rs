use std::path::PathBuf;
use std::{fs, io::Write, process};

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::utils::ColorType;
use crate::{cargo, storage};

const MAIN: &str = r#"
fn main() {
    println!("Hello Depi!");
}
"#;

#[derive(Debug, Parser)]
#[clap(about = "Dependencies Manager for Rust Projects", version)]
enum DepiCommand {
    Init {
        #[clap(short = 'D', long)]
        deps: Option<String>,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    New {
        #[clap(required = true)]
        name: String,

        #[clap(short = 'D', long)]
        deps: Option<String>,
        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    Add {
        #[clap(required = true)]
        deps: String,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    Remove {
        #[clap(required = true)]
        names: String,

        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    List {
        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },
    Update {
        #[clap(short, long, default_value = "osetia")]
        color: ColorType,
    },

    Alias {
        #[clap(subcommand)]
        command: AliasCommand,
    },
}

#[derive(Subcommand, Debug)]
enum AliasCommand {
    Add {
        #[clap(required = true)]
        name: String,
        #[clap(required = true)]
        original: String,
    },
    Remove {
        #[clap(required = true)]
        name: String,
    },
    List,
}

pub async fn handle_command() -> Result<()> {
    let com = DepiCommand::parse();
    match com {
        DepiCommand::Init { deps, color } => {
            // let cp = Cargo::from_cur()?;
            let cs = cargo::Cargo::init_project(None, deps.as_deref(), color).await?;

            let mut f = fs::File::create("Cargo.toml")?;
            f.write_all(cs.as_bytes())?;

            fs::create_dir("src")?;
            let mut f = fs::File::create("src/main.rs")?;
            f.write_all(MAIN.as_bytes())?;

            let gout = process::Command::new("git").arg("init").output()?.stdout;
            println!("{}", String::from_utf8(gout)?.bold());
        }
        DepiCommand::New { name, deps, color } => {
            let cs =
                cargo::Cargo::init_project(Some(name.as_ref()), deps.as_deref(), color).await?;

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
            let cp = cargo::Cargo::from_cur()?;
            cp.append_deps(deps, color).await?;
        }
        DepiCommand::Remove { names, color } => {
            let cp = cargo::Cargo::from_cur()?;
            cp.remove_deps(names, color).await?;
        }
        DepiCommand::Update { color } => {
            let cp = cargo::Cargo::from_cur()?;
            cp.update_deps(color).await?;
        }
        DepiCommand::List { color } => {
            let cp = cargo::Cargo::from_cur()?;
            cp.list(color).await?;
        }
        DepiCommand::Alias { command } => {
            let mut a_s = storage::AliasStorage::load()?;
            match command {
                AliasCommand::Add { name, original } => match a_s.add(&name, &original) {
                    Some(old) => {
                        println!("{} was replased by {}", old, original)
                    }
                    None => {
                        println!("added new alias")
                    }
                },
                AliasCommand::Remove { name } => match a_s.rem(&name) {
                    Some(removed) => {
                        println!("{} ({}) was removed", name, removed)
                    }
                    None => {
                        println!("{} not exist", name)
                    }
                },
                AliasCommand::List => {
                    let list = a_s.list();
                    println!("ALIAS LIST (");
                    for (k, v) in list {
                        println!("  {} -> {},", k, v);
                    }
                    println!(");");
                }
            }
            a_s.save()?;
        }
    }
    Ok(())
}
