use color_eyre::Result;
use std::env::var;

#[derive(Clone, Debug, PartialOrd, PartialEq, Eq)]
pub struct Config {
    pub postgres: String,
    pub admin_key: String,
    pub git: String,
    pub branch: String,
    pub default_timezone: i32,
    pub sitename: String,
}

impl Config {
    pub fn new() -> Result<Config> {
        let postgres = var("POSTGRES_URL")?;
        let admin_key = var("SECRET")?;
        let git = var("GIT_URL")?;
        let branch = var("GIT_BRANCH")?;
        let default_timezone = var("TIMEZONE_DEFAULT")?.parse::<i32>()?;
        let sitename = var("SITENAME")?;

        Ok(Config {
            postgres,
            admin_key,
            git,
            branch,
            default_timezone,
            sitename,
        })
    }

    pub fn postgres(&self) -> &str {
        &self.postgres
    }

    pub fn admin_key(&self) -> &str {
        &self.admin_key
    }

    pub fn git(&self) -> &str {
        &self.git
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn default_timezone(&self) -> i32 {
        self.default_timezone
    }

    pub fn sitename(&self) -> &str {
        &self.sitename
    }
    pub fn srv_large_subdomain(&self) -> bool {
        self.srv_large_subdomain
    }
}
