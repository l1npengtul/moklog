use color_eyre::Result;
use std::env::var;

#[derive(Clone, Debug, PartialOrd, PartialEq, Eq)]
pub struct Config {
    pub postgres: String,
    pub admin_key: String,
    pub git: String,
    pub branch: String,
}

impl Config {
    pub fn new() -> Result<()> {
        let postgres = var("POSTGRES_URL")?;
        let admin_key = var("SECRET")?;
        let git = var("GIT_URL")?;
        let branch = var("GIT_BRANCH")?;

        Ok(Config {
            postgres,
            admin_key,
            git,
            branch,
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
}
