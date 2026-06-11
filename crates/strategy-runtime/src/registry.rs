use std::collections::HashMap;

/// Assigns stable u16 slot IDs to feature names at instance-init time.
/// Read-only during the hot loop — never mutated per tick.
#[derive(Debug, Default)]
pub struct FeatureRegistry {
    name_to_slot: HashMap<String, u16>,
    slot_to_name: Vec<String>,
}

impl FeatureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or assign a slot ID. ONLY call during instance initialization, not per tick.
    pub fn get_or_assign(&mut self, name: &str) -> u16 {
        if let Some(&id) = self.name_to_slot.get(name) {
            return id;
        }
        let id = self.slot_to_name.len() as u16;
        self.name_to_slot.insert(name.to_owned(), id);
        self.slot_to_name.push(name.to_owned());
        id
    }

    pub fn get(&self, name: &str) -> Option<u16> {
        self.name_to_slot.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.slot_to_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slot_to_name.is_empty()
    }

    pub fn name_of(&self, slot: u16) -> Option<&str> {
        self.slot_to_name.get(slot as usize).map(String::as_str)
    }

    /// Iterate all (name, slot) pairs — used only on the fallback path.
    pub fn iter_slots(&self) -> impl Iterator<Item = (&str, u16)> {
        self.slot_to_name
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i as u16))
    }
}
