use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct SecretSet {
    values: BTreeMap<String, String>,
    missing: Vec<String>,
}

impl SecretSet {
    pub fn from_env_names(names: &[String]) -> Self {
        let mut set = Self::default();

        for name in names {
            match std::env::var(name) {
                Ok(value) if !value.is_empty() => set.add_value(name, value),
                _ => set.add_missing(name),
            }
        }

        set
    }

    pub fn add_value(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();

        if value.is_empty() {
            self.add_missing(name);
            return;
        }

        self.values.insert(name.clone(), value);
        self.missing.retain(|missing| missing != &name);
    }

    pub fn add_missing(&mut self, name: impl Into<String>) {
        let name = name.into();

        if !self.missing.contains(&name) && !self.values.contains_key(&name) {
            self.missing.push(name);
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }

    pub fn missing_names(&self) -> Vec<String> {
        let mut missing = self.missing.clone();
        missing.sort();
        missing
    }

    pub fn env_pairs(&self) -> BTreeMap<String, String> {
        self.values.clone()
    }

    pub fn mask(&self, input: &str) -> String {
        let mut masked = input.to_string();

        for value in self.values.values() {
            if !value.is_empty() {
                masked = masked.replace(value, "[REDACTED]");
            }
        }

        masked
    }
}

#[cfg(test)]
mod tests {
    use super::SecretSet;

    #[test]
    fn masks_secret_values() {
        let mut secrets = SecretSet::default();
        secrets.add_value("OPENAI_API_KEY", "sk-test-value");

        assert_eq!(secrets.mask("token=sk-test-value"), "token=[REDACTED]");
    }

    #[test]
    fn tracks_missing_secret_names() {
        let mut secrets = SecretSet::default();
        secrets.add_missing("OPENAI_API_KEY");

        assert_eq!(secrets.missing_names(), vec!["OPENAI_API_KEY"]);
    }
}
