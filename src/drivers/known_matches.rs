use serde::Deserialize;

const EMBEDDED_KNOWN_MATCHES: &str = include_str!("../../data/known_matches.toml");

#[derive(Debug, Deserialize)]
pub struct KnownMatches {
    pub matches: Vec<KnownMatch>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KnownMatch {
    pub model: String,
    pub driver: String,
    pub source: String,
}

impl KnownMatches {
    pub fn load_embedded() -> Self {
        toml::from_str(EMBEDDED_KNOWN_MATCHES)
            .expect("embedded known_matches.toml is invalid")
    }

    /// Find an exact match for the given model string (case-insensitive).
    pub fn find(&self, model: &str) -> Option<&KnownMatch> {
        let model_lower = model.to_lowercase();
        self.matches.iter().find(|m| m.model.to_lowercase() == model_lower)
    }
}
