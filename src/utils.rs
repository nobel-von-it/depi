#[derive(Debug, Clone)]
pub enum ColorType {
    One(DColor),
    Random,
}

impl ColorType {
    pub fn get_dcolor(&self) -> DColor {
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

#[derive(Default, Clone, Copy, Debug)]
pub enum DColor {
    #[default]
    WithoutColor,
    GOIDA,
    Osetia,
    Poland,
}

impl DColor {
    pub fn get_random() -> Self {
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

pub mod funcs {
    use anyhow::{Result, anyhow};
    use std::{env, fs, path::Path};

    pub fn current_absolute() -> Result<String> {
        absolutize(env::current_dir().unwrap_or(".".into()))
    }
    pub fn absolutize<P: AsRef<Path>>(path: P) -> Result<String> {
        Ok(fs::canonicalize(path)?
            .file_name()
            .ok_or(anyhow!("failed file_name()"))?
            .to_str()
            .ok_or(anyhow!("failed to_str()"))?
            .to_string())
    }

    pub(super) fn get_term_size() -> Option<(u16, u16)> {
        use libc::{TIOCGWINSZ, ioctl};
        use std::io;
        use std::mem::MaybeUninit;
        use std::os::fd::AsRawFd;

        #[repr(C)]
        struct Winsize {
            ws_row: u16,
            ws_col: u16,
            ws_xpixel: u16,
            ws_ypixel: u16,
        }

        let stdout = io::stdout();
        let fd_stdout = stdout.as_raw_fd();

        let mut size: MaybeUninit<Winsize> = MaybeUninit::uninit();

        unsafe {
            if ioctl(fd_stdout, TIOCGWINSZ, size.as_mut_ptr()) != -1 {
                let size = size.assume_init();
                Some((size.ws_col, size.ws_row))
            } else {
                None
            }
        }
    }
}

pub mod style {
    use colored::Colorize;
    use once_cell::sync::Lazy;

    use crate::{
        dep::{DType, Dep},
        utils::DColor,
    };

    static TERMINAL_SIZE: Lazy<(u16, u16)> =
        Lazy::new(|| super::funcs::get_term_size().unwrap_or((40, 20)));

    pub fn print_start_msg<S: AsRef<str>>(name: S) {
        println!("{}", name.as_ref().to_ascii_uppercase().bold().on_cyan());
        println!("{}", "=".repeat(TERMINAL_SIZE.0 as usize).cyan());
    }
    pub fn print_end_msg() {
        println!("{}", "=".repeat(TERMINAL_SIZE.0 as usize).cyan());
    }

    pub fn print_cargo_field(dtype: &DType) {
        println!("{}", dtype.to_cargo_field().green())
    }
    pub fn print_cargo_field_a(dtype: &DType) {
        println!("{}", dtype.to_cargo_field().red())
    }
    pub fn print_colored_ref_dep_version_update<S: AsRef<str>>(
        dep: &Dep,
        oldv: S,
        mnl: usize,
        mvl: usize,
        tabbing: usize,
        dct: DColor,
    ) {
        let dname = dep.name.as_ref();
        let dver = dep.version.as_ref();
        let oldv = oldv.as_ref();
        print_colored_val_dep_version_update(dname, dver, oldv, mnl, mvl, tabbing, dct);
    }
    pub fn print_colored_val_dep_version_update<S: AsRef<str>>(
        dname: S,
        dver: S,
        oldv: S,
        mnl: usize,
        mvl: usize,
        tabbing: usize,
        dct: DColor,
    ) {
        let dname = dname.as_ref();
        let dver = dver.as_ref();
        let oldv = oldv.as_ref();
        match dct {
            DColor::WithoutColor => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    dname.bold(),
                    oldv,
                    "->".dimmed(),
                    dver.bold()
                )
            }
            DColor::GOIDA => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    dname.bold(),
                    oldv.blue(),
                    "->".dimmed(),
                    dver.bold().red()
                )
            }
            DColor::Osetia => {
                println!(
                    "{}{:<mnl$} {:<mvl$} {} {}",
                    " ".repeat(tabbing),
                    dname.bold(),
                    oldv.yellow(),
                    "->".dimmed(),
                    dver.bold().red()
                )
            }
            DColor::Poland => {
                let oll = oldv.len() / 2;
                let oldvl = &oldv[0..oll];
                let oldvr = &oldv[oll..oldv.len()];

                let nmvl = mvl - oldvl.len();
                println!(
                    "{}{:<mnl$} {}{:<nmvl$} {} {}",
                    " ".repeat(tabbing),
                    dname.bold(),
                    oldvl,
                    oldvr.red(),
                    "->".dimmed(),
                    dver.bold().red()
                )
            }
        }
    }
    pub fn print_colored_ref_dep_full(
        dep: &Dep,
        mnl: usize,
        mvl: usize,
        tabbing: usize,
        dct: DColor,
    ) {
        let dname = &dep.name;
        let dver = &dep.version;
        let dfeat = dep.features.as_deref();
        print_colored_val_dep_full(dname, dver, dfeat, mnl, mvl, tabbing, dct);
    }
    pub fn print_colored_val_dep_full<S: AsRef<str>>(
        dname: S,
        dver: S,
        dfeat: Option<&[String]>,
        mnl: usize,
        mvl: usize,
        tabbing: usize,
        dct: DColor,
    ) {
        let dname = dname.as_ref();
        let dver = dver.as_ref();
        match dct {
            DColor::WithoutColor => {
                if let Some(fs) = &dfeat {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$} {} {}",
                        " ".repeat(tabbing),
                        &dname.bold(),
                        "@".dimmed(),
                        &dver,
                        ":".dimmed(),
                        fs.join(", "),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &dname,
                        "@".dimmed(),
                        &dver
                    );
                }
            }
            DColor::GOIDA => {
                if let Some(fs) = &dfeat {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$} {} {}",
                        " ".repeat(tabbing),
                        &dname.bold(),
                        "@".dimmed(),
                        &dver.blue(),
                        ":".dimmed(),
                        fs.join(", ").red(),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &dname.bold(),
                        "@".dimmed(),
                        &dver.blue()
                    );
                }
            }
            DColor::Osetia => {
                if let Some(fs) = &dfeat {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$} {} {}",
                        " ".repeat(tabbing),
                        &dname.bold(),
                        "@".dimmed(),
                        &dver.yellow(),
                        ":".dimmed(),
                        fs.join(", ").red(),
                    );
                } else {
                    println!(
                        "{}{:<mnl$} {} {:<mvl$}",
                        " ".repeat(tabbing),
                        &dname.bold(),
                        "@".dimmed(),
                        &dver.yellow()
                    );
                }
            }
            DColor::Poland => {
                if let Some(fs) = &dfeat {
                    let dvr = &dver;
                    let dvh = dvr.len() / 2;
                    let dvrl = &dvr[0..dvh];
                    let dvrr = &dvr[dvh..dvr.len()];

                    let nmvl = mvl - dvrl.len();
                    println!(
                        "{}{:<mnl$} {} {}{:<nmvl$} {} {}",
                        " ".repeat(tabbing),
                        &dname.bold(),
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
                        &dname.bold(),
                        "@".dimmed(),
                        &dver.red()
                    );
                }
            }
        }
    }
}

pub mod ver {
    use anyhow::{Result, anyhow};
    use log::warn;

    #[derive(PartialOrd, Ord, PartialEq, Eq, Clone, Debug, Default)]
    pub struct OrdVersion(pub u32, pub u32, pub u32);

    impl OrdVersion {
        pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
            let mut res = Self::default();

            let mut s = s.as_ref();

            if let Some((left, right)) = s.split_once("-") {
                s = left;
                warn!("version with suffix {right} is not supported");
                warn!("current version is {left}");
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
}
