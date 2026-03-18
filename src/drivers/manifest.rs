use serde::Deserialize;

const EMBEDDED_DRIVERS_TOML: &str = include_str!("../../data/drivers.toml");

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub manufacturers: Vec<Manufacturer>,
}

#[derive(Debug, Deserialize)]
pub struct Manufacturer {
    pub name: String,
    pub prefixes: Vec<String>,
    pub universal_drivers: Vec<UniversalDriver>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UniversalDriver {
    pub name: String,
    pub url: String,
    pub format: String,
}

impl Manifest {
    pub fn load_embedded() -> Self {
        toml::from_str(EMBEDDED_DRIVERS_TOML)
            .expect("embedded drivers.toml is invalid")
    }

    /// Find the manufacturer whose prefix matches the given model string.
    pub fn find_manufacturer(&self, model: &str) -> Option<&Manufacturer> {
        let model_upper = model.to_uppercase();
        self.manufacturers.iter().find(|m| {
            m.prefixes.iter().any(|p| model_upper.starts_with(&p.to_uppercase()))
        })
    }
}
